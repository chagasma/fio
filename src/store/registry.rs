use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use super::fs::write_atomic;
use crate::model::RegisteredProject;

/// Global project registry; unknown keys are preserved when rewriting.
#[derive(Default, Deserialize, Serialize)]
struct Config {
    #[serde(default)]
    projects: Vec<RegisteredProject>,
    #[serde(flatten)]
    extra: toml::Table,
}

fn config_path() -> Result<PathBuf> {
    let dirs =
        ProjectDirs::from("", "", "fio").context("Could not locate the configuration directory")?;
    Ok(dirs.config_dir().join("config.toml"))
}

fn read_config(path: &Path) -> Result<Config> {
    match fs::read_to_string(path) {
        Ok(contents) => toml::from_str(&contents)
            .with_context(|| format!("Invalid configuration: {}", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(error).with_context(|| format!("Could not read {}", path.display())),
    }
}

pub fn projects() -> Result<Vec<RegisteredProject>> {
    Ok(read_config(&config_path()?)?.projects)
}

pub fn register(name: &str, workspace: &Path) -> Result<()> {
    let path = config_path()?;
    let mut config = read_config(&path)?;
    for existing in &config.projects {
        let same_name = existing.name == name;
        let same_workspace = existing.workspace == workspace;
        if same_name && same_workspace {
            return Ok(());
        }
        if same_name || same_workspace {
            bail!(
                "Conflict with registered project: '{}' at {}",
                existing.name,
                existing.workspace.display()
            );
        }
    }
    config.projects.push(RegisteredProject {
        name: name.to_owned(),
        workspace: workspace.to_owned(),
    });
    write_atomic(
        &path,
        &toml::to_string_pretty(&toml::Table::try_from(&config)?)?,
    )
}
