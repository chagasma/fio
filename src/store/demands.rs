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
    let path = workspace.demand_dir(id).join("demand.md");
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

pub fn create(workspace: &Workspace, id: &DemandId, title: &str, priority: Priority) -> Result<()> {
    markdown::single_line(title)?;
    let today = today();
    let demand = Demand {
        id: id.clone(),
        title: title.trim().to_owned(),
        project: workspace.name().to_owned(),
        status: Status::default(),
        priority,
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
    let folder = workspace.demand_dir(id);
    fs::create_dir(&folder).with_context(|| {
        format!("Could not create '{id}'; check whether the demand already exists")
    })?;
    store_fs::write_atomic(&folder.join("demand.md"), &document.render())
}

/// Reads every demand in the workspace; unreadable ones become warnings.
pub fn scan(workspace: &Workspace) -> Result<Scan<LoadedDemand>> {
    let mut result = Scan::default();
    for path in store_fs::read_dir_sorted(&workspace.demands_dir())? {
        if !path.is_dir() {
            continue;
        }
        let id = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("Demand directory name must be UTF-8")
            .and_then(|name| name.parse::<DemandId>().map_err(anyhow::Error::msg));
        match id.and_then(|id| load(workspace, &id)) {
            Ok(loaded) => result.items.push(loaded),
            Err(error) => result
                .warnings
                .push(format!("{}: {error:#}", path.display())),
        }
    }
    Ok(result)
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
