use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use super::{fs::write_atomic, registry};
use crate::model::{DemandId, Project};

const PROJECT_FILE: &str = ".fio/project.toml";

/// A fio workspace: a directory with a `.fio/project.toml`.
#[derive(Clone)]
pub struct Workspace {
    root: PathBuf,
    project: String,
}

impl Workspace {
    pub fn open(root: PathBuf) -> Result<Self> {
        let project = read_project(&root.join(PROJECT_FILE))?;
        Ok(Self {
            root,
            project: project.name,
        })
    }

    /// Finds the workspace containing `directory`, if any.
    pub fn containing(directory: &Path) -> Result<Option<Self>> {
        directory
            .ancestors()
            .find(|path| path.join(PROJECT_FILE).is_file())
            .map(|root| Self::open(root.to_path_buf()))
            .transpose()
    }

    /// Finds the workspace containing the current directory.
    pub fn locate() -> Result<Self> {
        Self::containing(&std::env::current_dir()?.canonicalize()?)?
            .context("No fio project found; run fio init in the workspace")
    }

    /// Initializes `.fio/` in `directory` and registers the project.
    pub fn init(directory: &Path, name: Option<&str>) -> Result<Self> {
        let root = directory
            .canonicalize()
            .context("Could not resolve the workspace")?;
        let project_path = root.join(PROJECT_FILE);
        let existing = project_path
            .exists()
            .then(|| read_project(&project_path))
            .transpose()?;
        let project = match existing {
            Some(project) => {
                if name.is_some_and(|name| name != project.name) {
                    bail!(
                        "The workspace already belongs to project '{}'",
                        project.name
                    );
                }
                project
            }
            None => Project {
                name: name
                    .or_else(|| root.file_name().and_then(|name| name.to_str()))
                    .context("Could not determine the project name; use --name")?
                    .to_owned(),
                settings: toml::Table::new(),
            },
        };
        if project.name.trim().is_empty() {
            bail!("The project name cannot be empty");
        }
        registry::register(&project.name, &root)?;
        fs::create_dir_all(root.join(".fio/demands"))?;
        if !project_path.exists() {
            write_atomic(&project_path, &toml::to_string_pretty(&project)?)?;
        }
        Ok(Self {
            root,
            project: project.name,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> &str {
        &self.project
    }

    pub fn demands_dir(&self) -> PathBuf {
        self.root.join(".fio/demands")
    }

    pub fn demand_dir(&self, id: &DemandId) -> PathBuf {
        self.demands_dir().join(id)
    }

    pub fn state_file(&self) -> PathBuf {
        self.root.join(".fio/state.toml")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.root.join(".fio/state.lock")
    }
}

fn read_project(path: &Path) -> Result<Project> {
    let source =
        fs::read_to_string(path).with_context(|| format!("Could not read {}", path.display()))?;
    toml::from_str(&source).with_context(|| format!("Invalid configuration: {}", path.display()))
}
