use std::{fs::File, io::ErrorKind};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{demands, fs as store_fs, workspace::Workspace};
use crate::model::DemandId;

#[derive(Default, Deserialize, Serialize)]
pub struct State {
    pub active_demand: Option<DemandId>,
}

/// Serializes workspace-wide writes (session records and the default demand).
pub struct WorkspaceLock {
    _file: File,
}

impl WorkspaceLock {
    pub fn acquire(workspace: &Workspace) -> Result<Self> {
        store_fs::lock(&workspace.lock_file())
            .context("Could not lock the workspace for writing")
            .map(|file| Self { _file: file })
    }
}

pub fn read(workspace: &Workspace) -> Result<State> {
    match std::fs::read_to_string(workspace.state_file()) {
        Ok(source) => toml::from_str(&source).context("Invalid local fio state"),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(State::default()),
        Err(error) => Err(error).context("Could not read the local fio state"),
    }
}

pub fn active(workspace: &Workspace) -> Result<DemandId> {
    read(workspace)?
        .active_demand
        .context("No active demand; run fio use <id>")
}

pub fn select(workspace: &Workspace, id: &DemandId) -> Result<()> {
    demands::load(workspace, id)?;
    let _lock = WorkspaceLock::acquire(workspace)?;
    let mut state = read(workspace)?;
    state.active_demand = Some(id.clone());
    store_fs::write_atomic(&workspace.state_file(), &toml::to_string_pretty(&state)?)
}
