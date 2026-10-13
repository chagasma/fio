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
        let mut catalog = Self {
            projects: registry::projects()?,
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
        Ok(catalog)
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
