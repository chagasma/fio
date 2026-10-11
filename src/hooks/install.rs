use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use crate::{
    model::{Agent, DemandId},
    store::{demands, fs::write_atomic, workspace::Workspace},
};

const STATUS_MESSAGE: &str = "fio lifecycle integration";
const EVENTS: [&str; 3] = ["SessionStart", "Stop", "SessionEnd"];

/// Merges fio's lifecycle handlers into the workspace's agent settings.
pub fn configure(
    agent: Agent,
    executable: Option<PathBuf>,
    demand: Option<&DemandId>,
) -> Result<()> {
    let workspace = Workspace::locate()?;
    if let Some(id) = demand {
        demands::load(&workspace, id)?;
    }
    let executable = match executable {
        Some(path) => path,
        None => std::env::current_exe()?,
    }
    .canonicalize()
    .context("Could not resolve the fio executable")?;
    ensure!(executable.is_file(), "fio executable must be a file");
    let command = hook_command(&executable, agent, demand)?;
    let path = workspace.root().join(match agent {
        Agent::Claude => ".claude/settings.local.json",
        Agent::Codex => ".codex/hooks.json",
    });
    let mut config = read_settings(&path)?;
    merge_fio_hooks(&mut config, &command)?;
    write_atomic(
        &path,
        &format!("{}\n", serde_json::to_string_pretty(&config)?),
    )?;
    println!(
        "{} hooks installed in {}. Restart the agent and approve its project hook trust prompt if shown.",
        agent.name(),
        path.display()
    );
    Ok(())
}

fn hook_command(executable: &Path, agent: Agent, demand: Option<&DemandId>) -> Result<String> {
    let executable = executable
        .to_str()
        .context("Executable path must be UTF-8")?;
    let demand = demand.map_or_else(String::new, |id| {
        format!(" --demand {}", shell_words::quote(id.as_str()))
    });
    Ok(format!(
        "{} hook {}{demand}",
        shell_words::quote(executable),
        agent.name()
    ))
}

fn read_settings(path: &Path) -> Result<Value> {
    match fs::read_to_string(path) {
        Ok(source) => serde_json::from_str(&source)
            .context("Invalid agent hook settings; refusing to overwrite"),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(json!({})),
        Err(error) => Err(error.into()),
    }
}

fn merge_fio_hooks(config: &mut Value, command: &str) -> Result<()> {
    let hooks = config
        .as_object_mut()
        .context("Agent settings must be a JSON object")?
        .entry("hooks")
        .or_insert(json!({}))
        .as_object_mut()
        .context("hooks must be an object")?;
    for event in EVENTS {
        let groups = hooks
            .entry(event)
            .or_insert(json!([]))
            .as_array_mut()
            .context("Hook event must be an array")?;
        // Identify only fio-owned groups; leave user commands and matchers intact.
        groups.retain_mut(|group| {
            let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let before = handlers.len();
            handlers.retain(|handler| {
                handler.get("statusMessage").and_then(Value::as_str) != Some(STATUS_MESSAGE)
            });
            handlers.len() == before || !handlers.is_empty()
        });
        let timeout = if event == "SessionEnd" { 3 } else { 10 };
        groups.push(json!({"hooks":[{"type":"command","command":command,"statusMessage":STATUS_MESSAGE,"timeout":timeout}]}));
    }
    Ok(())
}
