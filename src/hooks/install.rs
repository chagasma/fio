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
    let path = settings_path(&workspace, agent);
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

fn settings_path(workspace: &Workspace, agent: Agent) -> PathBuf {
    workspace.root().join(match agent {
        Agent::Claude => ".claude/settings.local.json",
        Agent::Codex => ".codex/hooks.json",
    })
}

/// Removes fio's lifecycle handlers from the workspace's agent settings,
/// leaving every other hook and setting untouched.
pub fn remove(agent: Agent) -> Result<()> {
    let workspace = Workspace::locate()?;
    let path = settings_path(&workspace, agent);
    if !path.exists() {
        println!(
            "No {} hook settings found at {}",
            agent.name(),
            path.display()
        );
        return Ok(());
    }
    let mut config = read_settings(&path)?;
    strip_fio_hooks(&mut config)?;
    if config.as_object().is_some_and(serde_json::Map::is_empty) {
        fs::remove_file(&path)?;
        println!(
            "{} hooks removed; deleted empty {}",
            agent.name(),
            path.display()
        );
    } else {
        write_atomic(
            &path,
            &format!("{}\n", serde_json::to_string_pretty(&config)?),
        )?;
        println!("{} hooks removed from {}", agent.name(), path.display());
    }
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
        remove_fio_handlers(groups);
        let timeout = if event == "SessionEnd" { 3 } else { 10 };
        groups.push(json!({"hooks":[{"type":"command","command":command,"statusMessage":STATUS_MESSAGE,"timeout":timeout}]}));
    }
    Ok(())
}

/// Identifies only fio-owned handlers; user commands and matchers stay intact.
fn remove_fio_handlers(groups: &mut Vec<Value>) {
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
}

fn strip_fio_hooks(config: &mut Value) -> Result<()> {
    let root = config
        .as_object_mut()
        .context("Agent settings must be a JSON object")?;
    let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    for event in EVENTS {
        if let Some(groups) = hooks.get_mut(event).and_then(Value::as_array_mut) {
            remove_fio_handlers(groups);
            if groups.is_empty() {
                hooks.remove(event);
            }
        }
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    Ok(())
}
