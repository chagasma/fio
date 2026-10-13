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

#[cfg(test)]
mod tests {
    use super::*;

    const COMMAND: &str = "/usr/bin/fio hook claude";

    fn merged(mut config: Value) -> Value {
        merge_fio_hooks(&mut config, COMMAND).unwrap();
        config
    }

    fn fio_handlers(config: &Value, event: &str) -> usize {
        config["hooks"][event]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|group| group["hooks"].as_array().unwrap())
            .filter(|handler| handler["statusMessage"] == STATUS_MESSAGE)
            .count()
    }

    #[test]
    fn merge_installs_every_lifecycle_event() {
        let config = merged(json!({}));
        for event in EVENTS {
            assert_eq!(fio_handlers(&config, event), 1, "{event}");
        }
        let handler = &config["hooks"]["SessionEnd"][0]["hooks"][0];
        assert_eq!(handler["command"], COMMAND);
        assert_eq!(handler["timeout"], 3);
        assert_eq!(config["hooks"]["Stop"][0]["hooks"][0]["timeout"], 10);
    }

    #[test]
    fn merge_is_idempotent() {
        let once = merged(json!({}));
        assert_eq!(merged(once.clone()), once);
    }

    #[test]
    fn merge_keeps_user_hooks_and_settings() {
        let user = json!({"hooks":[{"type":"command","command":"echo hi"}]});
        let config = merged(json!({"model": "x", "hooks": {"Stop": [user.clone()]}}));
        assert_eq!(config["model"], "x");
        assert_eq!(config["hooks"]["Stop"][0], user);
        assert_eq!(fio_handlers(&config, "Stop"), 1);
    }

    #[test]
    fn merge_replaces_an_outdated_fio_command() {
        let old = json!({"hooks": {"Stop": [{"hooks":[{"type":"command","command":"old","statusMessage":STATUS_MESSAGE}]}]}});
        let config = merged(old);
        assert_eq!(fio_handlers(&config, "Stop"), 1);
        assert_eq!(config["hooks"]["Stop"][0]["hooks"][0]["command"], COMMAND);
    }

    #[test]
    fn merge_rejects_unexpected_shapes() {
        let mut array = json!([]);
        assert!(merge_fio_hooks(&mut array, COMMAND).is_err());
        let mut hooks = json!({"hooks": []});
        assert!(merge_fio_hooks(&mut hooks, COMMAND).is_err());
        let mut event = json!({"hooks": {"Stop": {}}});
        assert!(merge_fio_hooks(&mut event, COMMAND).is_err());
    }

    #[test]
    fn strip_removes_only_fio_handlers() {
        let user = json!({"hooks":[{"type":"command","command":"echo hi"}]});
        let mut config = merged(json!({"hooks": {"Stop": [user.clone()]}, "model": "x"}));
        strip_fio_hooks(&mut config).unwrap();
        assert_eq!(config, json!({"hooks": {"Stop": [user]}, "model": "x"}));
    }

    #[test]
    fn strip_leaves_a_mixed_group_with_its_user_handler() {
        let mixed = json!({"hooks": {"Stop": [{"hooks": [
            {"type":"command","command":"echo hi"},
            {"type":"command","command":"fio","statusMessage":STATUS_MESSAGE},
        ]}]}});
        let mut config = mixed;
        strip_fio_hooks(&mut config).unwrap();
        let handlers = config["hooks"]["Stop"][0]["hooks"].as_array().unwrap();
        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0]["command"], "echo hi");
    }

    #[test]
    fn strip_drops_empty_events_and_the_hooks_key() {
        let mut config = merged(json!({}));
        strip_fio_hooks(&mut config).unwrap();
        assert_eq!(config, json!({}));
    }

    #[test]
    fn strip_without_hooks_is_a_no_op() {
        let mut config = json!({"model": "x"});
        strip_fio_hooks(&mut config).unwrap();
        assert_eq!(config, json!({"model": "x"}));
    }

    #[test]
    fn hook_command_quotes_the_executable_and_pins_a_demand() {
        let id: DemandId = "my-demand".parse().unwrap();
        let command =
            hook_command(Path::new("/opt/my tools/fio"), Agent::Codex, Some(&id)).unwrap();
        assert_eq!(command, "'/opt/my tools/fio' hook codex --demand my-demand");
        let plain = hook_command(Path::new("/usr/bin/fio"), Agent::Claude, None).unwrap();
        assert_eq!(plain, "/usr/bin/fio hook claude");
    }

    #[test]
    fn read_settings_treats_a_missing_file_as_empty_and_refuses_invalid_json() {
        let temp = crate::store::testing::TempWorkspace::new();
        let dir = temp.workspace.root();
        assert_eq!(read_settings(&dir.join("missing.json")).unwrap(), json!({}));
        let invalid = dir.join("invalid.json");
        fs::write(&invalid, "{ nope").unwrap();
        assert!(read_settings(&invalid).is_err());
    }

    #[test]
    fn settings_paths_differ_per_agent() {
        let temp = crate::store::testing::TempWorkspace::new();
        let root = temp.workspace.root();
        assert_eq!(
            settings_path(&temp.workspace, Agent::Claude),
            root.join(".claude/settings.local.json")
        );
        assert_eq!(
            settings_path(&temp.workspace, Agent::Codex),
            root.join(".codex/hooks.json")
        );
    }
}
