use std::{collections::BTreeMap, path::PathBuf};

use anyhow::Result;

use super::{
    demands::{self, LoadedDemand},
    registry, state,
    workspace::Workspace,
};
use crate::model::{DemandId, RegisteredProject};

/// Stable identity of a demand across reloads.
#[derive(Clone, PartialEq)]
pub struct DemandKey {
    pub workspace: PathBuf,
    pub id: DemandId,
}

pub struct Catalog {
    pub projects: Vec<RegisteredProject>,
    pub demands: Vec<Entry>,
    pub warnings: Vec<String>,
    pub active: BTreeMap<PathBuf, DemandId>,
}

pub struct Entry {
    pub workspace: PathBuf,
    pub loaded: LoadedDemand,
}

impl Catalog {
    /// Loads every registered project; unreadable ones become warnings.
    pub fn load() -> Result<Self> {
        Ok(Self::from_projects(registry::projects()?))
    }

    pub fn from_projects(projects: Vec<RegisteredProject>) -> Self {
        let mut catalog = Self {
            projects,
            demands: Vec::new(),
            warnings: Vec::new(),
            active: BTreeMap::new(),
        };
        for registration in &catalog.projects {
            let loaded = Workspace::open(registration.workspace.clone()).and_then(|workspace| {
                let active = state::read(&workspace)?.active_demand;
                Ok((demands::scan(&workspace)?, active))
            });
            match loaded {
                Ok((scanned, active)) => {
                    catalog.warnings.extend(scanned.warnings);
                    catalog
                        .demands
                        .extend(scanned.items.into_iter().map(|loaded| Entry {
                            workspace: registration.workspace.clone(),
                            loaded,
                        }));
                    if let Some(id) = active {
                        catalog.active.insert(registration.workspace.clone(), id);
                    }
                }
                Err(error) => catalog
                    .warnings
                    .push(format!("{}: {error:#}", registration.name)),
            }
        }
        catalog
    }

    /// Sub-demands of `parent`, in folder order.
    pub fn children_of<'a>(&'a self, parent: &'a Entry) -> impl Iterator<Item = &'a Entry> {
        let id = parent.loaded.id();
        self.demands.iter().filter(move |entry| {
            entry.workspace == parent.workspace
                && entry.loaded.document.metadata.parent.as_ref() == Some(id)
        })
    }

    pub fn find(&self, key: &DemandKey) -> Option<&Entry> {
        self.demands.iter().find(|entry| entry.key() == *key)
    }
}

impl Entry {
    pub fn key(&self) -> DemandKey {
        DemandKey {
            workspace: self.workspace.clone(),
            id: self.loaded.id().clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::{TempWorkspace, id};

    #[test]
    fn children_of_returns_only_the_sub_demands_of_that_parent() {
        let temp = TempWorkspace::with_demands(&["p", "other"]);
        temp.create_child("c1", "p");
        temp.create_child("o1", "other");
        let catalog = temp.catalog();
        let parent = catalog.find(&DemandKey {
            workspace: temp.workspace.root().to_path_buf(),
            id: id("p"),
        });
        let ids: Vec<_> = catalog
            .children_of(parent.unwrap())
            .map(|entry| entry.loaded.id().to_string())
            .collect();
        assert_eq!(ids, ["c1"]);
    }

    #[test]
    fn find_matches_on_workspace_and_id() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let catalog = temp.catalog();
        let key = |workspace: PathBuf, name: &str| DemandKey {
            workspace,
            id: id(name),
        };
        let root = temp.workspace.root().to_path_buf();
        assert!(catalog.find(&key(root.clone(), "a")).is_some());
        assert!(catalog.find(&key(root, "ghost")).is_none());
        assert!(
            catalog
                .find(&key(PathBuf::from("/elsewhere"), "a"))
                .is_none()
        );
    }

    #[test]
    fn entry_key_identifies_the_demand() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let catalog = temp.catalog();
        let key = catalog.demands[0].key();
        assert_eq!(key.id, id("a"));
        assert_eq!(key.workspace, temp.workspace.root());
    }
}
