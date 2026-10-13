use std::{
    collections::BTreeMap,
    fmt::Write,
    fs::{self, File},
    ops::Deref,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::Serialize;

use super::{
    Scan, fs as store_fs,
    markdown::{self, Document},
    workspace::Workspace,
};
use crate::model::{Demand, DemandId, ItemKind, Priority, Status, section, today};

/// A demand read from disk. Use [`LockedDemand`] to modify it.
pub struct LoadedDemand {
    pub path: PathBuf,
    pub document: Document<Demand>,
}

/// A demand holding its write lock; the lock is released on drop.
pub struct LockedDemand {
    demand: LoadedDemand,
    _lock: File,
}

#[derive(Serialize)]
pub struct DemandSummary {
    #[serde(flatten)]
    pub demand: Demand,
    pub workspace: PathBuf,
}

pub fn load(workspace: &Workspace, id: &DemandId) -> Result<LoadedDemand> {
    load_from(workspace, id, &workspace.demand_dir(id))
}

fn load_from(workspace: &Workspace, id: &DemandId, folder: &Path) -> Result<LoadedDemand> {
    let path = folder.join("demand.md");
    let source = fs::read_to_string(&path).with_context(|| {
        format!(
            "Could not read demand '{id}' in {}",
            workspace.root().display()
        )
    })?;
    let document = Document::<Demand>::parse(&source)
        .with_context(|| format!("Invalid demand: {}", path.display()))?;
    ensure!(
        &document.metadata.id == id,
        "Frontmatter ID does not match the directory: {}",
        path.display()
    );
    ensure!(
        document.metadata.project == workspace.name(),
        "Frontmatter project does not match the workspace: {}",
        path.display()
    );
    let expected_parent = folder
        .parent()
        .filter(|parent| *parent != workspace.demands_dir())
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    ensure!(
        document.metadata.parent.as_ref().map(DemandId::as_str) == expected_parent,
        "Frontmatter parent does not match the directory: {}",
        path.display()
    );
    Ok(LoadedDemand { path, document })
}

pub fn load_for_update(workspace: &Workspace, id: &DemandId) -> Result<LockedDemand> {
    let lock = store_fs::lock(&workspace.demand_dir(id).join("write.lock"))
        .with_context(|| format!("Could not open demand '{id}' for writing"))?;
    Ok(LockedDemand {
        demand: load(workspace, id)?,
        _lock: lock,
    })
}

/// Creates a demand, or a sub-demand inside `parent` (one level only).
pub fn create(
    workspace: &Workspace,
    id: &DemandId,
    title: &str,
    priority: Priority,
    parent: Option<&DemandId>,
) -> Result<()> {
    markdown::single_line(title)?;
    ensure!(
        id.as_str() != "sessions",
        "'sessions' is reserved for session records"
    );
    ensure!(
        !workspace.demand_dir(id).exists(),
        "Demand '{id}' already exists"
    );
    let folder = match parent {
        Some(parent) => {
            let parent = load(workspace, parent)?;
            ensure!(
                parent.document.metadata.parent.is_none(),
                "'{}' is already a sub-demand; only one level is supported",
                parent.id()
            );
            parent.folder().join(id)
        }
        None => workspace.demands_dir().join(id),
    };
    let today = today();
    let demand = Demand {
        id: id.clone(),
        title: title.trim().to_owned(),
        project: workspace.name().to_owned(),
        status: Status::default(),
        priority,
        parent: parent.cloned(),
        created: today,
        updated: today,
        links: BTreeMap::new(),
        related: Vec::new(),
        blocked_by: Vec::new(),
        docs: Vec::new(),
        extra: BTreeMap::new(),
    };
    let mut body = String::new();
    for title in [section::OBJECTIVE, section::CURRENT_STATE]
        .into_iter()
        .chain(ItemKind::ALL.map(ItemKind::section))
    {
        write!(body, "\n## {title}\n")?;
    }
    let document = Document::new(demand, body)?;
    fs::create_dir(&folder).with_context(|| format!("Could not create '{id}'"))?;
    store_fs::write_atomic(&folder.join("demand.md"), &document.render())
}

/// Reads every demand in the workspace, each followed by its sub-demands;
/// unreadable ones become warnings.
pub fn scan(workspace: &Workspace) -> Result<Scan<LoadedDemand>> {
    let mut result = Scan::default();
    for path in store_fs::read_dir_sorted(&workspace.demands_dir())? {
        if !path.is_dir() {
            continue;
        }
        load_folder(workspace, &path, &mut result);
        let children = scan_children(workspace, &path)?;
        result.items.extend(children.items);
        result.warnings.extend(children.warnings);
    }
    Ok(result)
}

fn load_folder(workspace: &Workspace, path: &Path, result: &mut Scan<LoadedDemand>) {
    let id = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("Demand directory name must be UTF-8")
        .and_then(|name| name.parse::<DemandId>().map_err(anyhow::Error::msg));
    match id.and_then(|id| load_from(workspace, &id, path)) {
        Ok(loaded) => result.items.push(loaded),
        Err(error) => result
            .warnings
            .push(format!("{}: {error:#}", path.display())),
    }
}

/// Sub-demands inside a top-level demand folder: subfolders holding a `demand.md`.
fn scan_children(workspace: &Workspace, folder: &Path) -> Result<Scan<LoadedDemand>> {
    let mut result = Scan::default();
    for path in store_fs::read_dir_sorted(folder)? {
        if path.join("demand.md").is_file() {
            load_folder(workspace, &path, &mut result);
        }
    }
    Ok(result)
}

/// Sub-demands of `parent`, in folder order.
pub fn children(workspace: &Workspace, parent: &LoadedDemand) -> Result<Vec<LoadedDemand>> {
    Ok(scan_children(workspace, parent.folder())?.items)
}

/// Demands of one workspace, optionally filtered by status.
pub fn list(workspace: &Workspace, status: Option<Status>) -> Result<Scan<DemandSummary>> {
    let scanned = scan(workspace)?;
    Ok(Scan {
        items: scanned
            .items
            .into_iter()
            .filter(|loaded| status.is_none_or(|status| loaded.document.metadata.status == status))
            .map(|loaded| DemandSummary {
                demand: loaded.document.metadata,
                workspace: workspace.root().to_owned(),
            })
            .collect(),
        warnings: scanned.warnings,
    })
}

impl LoadedDemand {
    pub fn id(&self) -> &DemandId {
        &self.document.metadata.id
    }

    pub fn folder(&self) -> &Path {
        self.path.parent().expect("demand file has a directory")
    }
}

impl Deref for LockedDemand {
    type Target = LoadedDemand;

    fn deref(&self) -> &LoadedDemand {
        &self.demand
    }
}

impl LockedDemand {
    pub fn save(&mut self) -> Result<()> {
        self.demand.document.set_field("updated", today())?;
        store_fs::write_atomic(&self.demand.path, &self.demand.document.render())
    }

    pub fn set_status(&mut self, status: Status) -> Result<()> {
        self.demand.document.set_field("status", status)?;
        self.save()
    }

    pub fn set_priority(&mut self, priority: Priority) -> Result<()> {
        self.demand.document.set_field("priority", priority)?;
        self.save()
    }

    pub fn set_current_state(&mut self, text: &str) -> Result<()> {
        markdown::set_section(&mut self.demand.document.body, section::CURRENT_STATE, text)?;
        self.save()
    }

    pub fn link(&mut self, workspace: &Workspace, paths: Vec<PathBuf>) -> Result<()> {
        let mut docs = self.demand.document.metadata.docs.clone();
        for path in paths {
            let path = path
                .canonicalize()
                .with_context(|| format!("Document does not exist: {}", path.display()))?;
            let relative = path
                .strip_prefix(workspace.root())
                .context("The document must be inside the workspace")?;
            let relative = relative.to_str().context("Document path must be UTF-8")?;
            if !docs.iter().any(|doc| doc == relative) {
                docs.push(relative.to_owned());
            }
        }
        self.demand.document.set_field("docs", docs)?;
        self.save()
    }

    pub fn add_item(&mut self, kind: ItemKind, text: &str) -> Result<String> {
        let id = markdown::add_item(&mut self.demand.document.body, kind, text, today())?;
        self.save()?;
        Ok(id)
    }

    pub fn complete_item(&mut self, kind: ItemKind, id: &str, answer: Option<&str>) -> Result<()> {
        markdown::complete_item(&mut self.demand.document.body, kind, id, answer, today())?;
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::{TempWorkspace, assert_err, id};

    fn ids(demands: &[LoadedDemand]) -> Vec<&str> {
        demands.iter().map(|demand| demand.id().as_str()).collect()
    }

    fn rewrite(path: &Path, from: &str, to: &str) {
        let source = fs::read_to_string(path).unwrap();
        assert!(source.contains(from), "{from:?} not in {source:?}");
        fs::write(path, source.replacen(from, to, 1)).unwrap();
    }

    #[test]
    fn create_writes_a_loadable_demand() {
        let temp = TempWorkspace::new();
        create(
            &temp.workspace,
            &id("a"),
            "  My title ",
            Priority::High,
            None,
        )
        .unwrap();
        let loaded = load(&temp.workspace, &id("a")).unwrap();
        let metadata = &loaded.document.metadata;
        assert_eq!(metadata.title, "My title");
        assert_eq!(metadata.project, "test");
        assert_eq!(metadata.status, Status::Backlog);
        assert_eq!(metadata.priority, Priority::High);
        assert!(metadata.parent.is_none());
    }

    #[test]
    fn create_writes_the_standard_sections() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let loaded = load(&temp.workspace, &id("a")).unwrap();
        for title in [
            "Objective",
            "Current state",
            "Decisions",
            "Questions",
            "Debts",
        ] {
            assert!(
                loaded.document.body.contains(&format!("## {title}")),
                "{title}"
            );
        }
    }

    #[test]
    fn create_rejects_an_existing_id() {
        let temp = TempWorkspace::with_demands(&["a"]);
        assert_err(temp.try_create("a", None), "already exists");
    }

    #[test]
    fn create_rejects_the_reserved_sessions_id() {
        let temp = TempWorkspace::new();
        assert_err(temp.try_create("sessions", None), "reserved");
    }

    #[test]
    fn create_rejects_multiline_titles() {
        let temp = TempWorkspace::new();
        let result = create(&temp.workspace, &id("a"), "x\ny", Priority::default(), None);
        assert_err(result, "single line");
    }

    #[test]
    fn sub_demands_record_their_parent() {
        let temp = TempWorkspace::with_demands(&["parent"]);
        temp.create_child("child", "parent");
        let child = load(&temp.workspace, &id("child")).unwrap();
        assert_eq!(child.document.metadata.parent, Some(id("parent")));
    }

    #[test]
    fn sub_demands_are_limited_to_one_level() {
        let temp = TempWorkspace::with_demands(&["parent"]);
        temp.create_child("child", "parent");
        assert_err(
            temp.try_create("gc", Some("child")),
            "only one level is supported",
        );
    }

    #[test]
    fn sub_demands_require_an_existing_parent() {
        let temp = TempWorkspace::new();
        assert_err(
            temp.try_create("c", Some("ghost")),
            "Could not read demand 'ghost'",
        );
    }

    #[test]
    fn sub_demand_ids_cannot_reuse_a_top_level_id() {
        let temp = TempWorkspace::with_demands(&["a", "p"]);
        assert_err(temp.try_create("a", Some("p")), "already exists");
    }

    #[test]
    fn sub_demand_ids_cannot_repeat_across_parents() {
        let temp = TempWorkspace::with_demands(&["p1", "p2"]);
        temp.create_child("c", "p1");
        assert_err(temp.try_create("c", Some("p2")), "already exists");
    }

    #[test]
    fn scan_lists_each_parent_followed_by_its_children() {
        let temp = TempWorkspace::with_demands(&["m", "z"]);
        temp.create_child("a-child", "m");
        let scanned = scan(&temp.workspace).unwrap();
        assert_eq!(ids(&scanned.items), ["m", "a-child", "z"]);
        assert_eq!(scanned.warnings, Vec::<String>::new());
    }

    #[test]
    fn scan_turns_unreadable_demands_into_warnings() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let broken = temp.workspace.demands_dir().join("broken");
        fs::create_dir(&broken).unwrap();
        fs::write(broken.join("demand.md"), "no frontmatter").unwrap();

        let scanned = scan(&temp.workspace).unwrap();
        assert_eq!(ids(&scanned.items), ["a"]);
        assert_eq!(scanned.warnings.len(), 1);
        assert!(scanned.warnings[0].contains("broken"));
    }

    #[test]
    fn scan_does_not_see_demands_nested_below_a_sub_demand() {
        let temp = TempWorkspace::with_demands(&["p"]);
        temp.create_child("c", "p");
        let child = temp.workspace.demand_dir(&id("c"));
        fs::create_dir(child.join("gc")).unwrap();
        fs::copy(child.join("demand.md"), child.join("gc/demand.md")).unwrap();

        let scanned = scan(&temp.workspace).unwrap();
        assert_eq!(ids(&scanned.items), ["p", "c"]);
        assert_eq!(scanned.warnings, Vec::<String>::new());
    }

    #[test]
    fn load_rejects_a_frontmatter_id_that_differs_from_the_folder() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let demands_dir = temp.workspace.demands_dir();
        fs::rename(demands_dir.join("a"), demands_dir.join("renamed")).unwrap();
        assert_err(
            load(&temp.workspace, &id("renamed")),
            "Frontmatter ID does not match",
        );
    }

    #[test]
    fn load_rejects_a_frontmatter_project_that_differs_from_the_workspace() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let path = temp.workspace.demand_dir(&id("a")).join("demand.md");
        rewrite(&path, "project: test", "project: other");
        assert_err(load(&temp.workspace, &id("a")), "project does not match");
    }

    #[test]
    fn load_rejects_a_parent_that_differs_from_the_folder() {
        let temp = TempWorkspace::with_demands(&["p"]);
        temp.create_child("c", "p");
        let demands_dir = temp.workspace.demands_dir();
        fs::rename(demands_dir.join("p/c"), demands_dir.join("c")).unwrap();
        assert_err(load(&temp.workspace, &id("c")), "parent does not match");
    }

    #[test]
    fn list_returns_every_demand_without_a_filter() {
        let temp = TempWorkspace::with_demands(&["a", "b"]);
        assert_eq!(list(&temp.workspace, None).unwrap().items.len(), 2);
    }

    #[test]
    fn list_filters_by_status() {
        let temp = TempWorkspace::with_demands(&["a", "b"]);
        load_for_update(&temp.workspace, &id("b"))
            .unwrap()
            .set_status(Status::Done)
            .unwrap();
        let done = list(&temp.workspace, Some(Status::Done)).unwrap().items;
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].demand.id.as_str(), "b");
    }

    #[test]
    fn children_are_sorted_and_scoped_to_the_parent() {
        let temp = TempWorkspace::with_demands(&["p", "other"]);
        temp.create_child("c2", "p");
        temp.create_child("c1", "p");
        temp.create_child("o1", "other");
        let parent = load(&temp.workspace, &id("p")).unwrap();
        let children = children(&temp.workspace, &parent).unwrap();
        assert_eq!(ids(&children), ["c1", "c2"]);
    }

    fn edited(temp: &TempWorkspace, edit: impl FnOnce(&mut LockedDemand)) -> LoadedDemand {
        edit(&mut load_for_update(&temp.workspace, &id("a")).unwrap());
        load(&temp.workspace, &id("a")).unwrap()
    }

    #[test]
    fn set_priority_persists() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let loaded = edited(&temp, |d| d.set_priority(Priority::Low).unwrap());
        assert_eq!(loaded.document.metadata.priority, Priority::Low);
    }

    #[test]
    fn set_current_state_persists() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let loaded = edited(&temp, |d| d.set_current_state("Halfway").unwrap());
        let text = markdown::section_text(&loaded.document.body, section::CURRENT_STATE).unwrap();
        assert_eq!(text, "Halfway");
    }

    #[test]
    fn add_and_complete_item_persist() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let loaded = edited(&temp, |d| {
            assert_eq!(d.add_item(ItemKind::Question, "why?").unwrap(), "Q1");
            d.complete_item(ItemKind::Question, "Q1", Some("because"))
                .unwrap();
        });
        let all = markdown::items(&loaded.document.body, ItemKind::Question).unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].done);
    }

    #[test]
    fn link_stores_workspace_relative_paths_once() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let doc = temp.workspace.root().join("notes.md");
        fs::write(&doc, "x").unwrap();
        let loaded = edited(&temp, |d| {
            d.link(&temp.workspace, vec![doc.clone(), doc.clone()])
                .unwrap();
        });
        assert_eq!(loaded.document.metadata.docs, ["notes.md"]);
    }

    #[test]
    fn link_rejects_missing_documents_and_paths_outside_the_workspace() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut locked = load_for_update(&temp.workspace, &id("a")).unwrap();
        let missing = temp.workspace.root().join("missing.md");
        assert_err(
            locked.link(&temp.workspace, vec![missing]),
            "does not exist",
        );
        let outside = std::env::current_exe().unwrap();
        assert_err(
            locked.link(&temp.workspace, vec![outside]),
            "inside the workspace",
        );
    }

    #[test]
    fn saving_bumps_the_updated_date() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let path = temp.workspace.demand_dir(&id("a")).join("demand.md");
        rewrite(
            &path,
            &format!("updated: \"{}\"", today()),
            "updated: \"2000-01-01\"",
        );
        let loaded = edited(&temp, |d| d.set_status(Status::Active).unwrap());
        let stale = chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
        assert_ne!(loaded.document.metadata.updated, stale);
    }
}
