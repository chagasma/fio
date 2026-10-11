mod install;
mod lifecycle;

use std::io::{self, Read};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use crate::{
    model::{Agent, DemandId},
    store::workspace::Workspace,
};
use lifecycle::{Event, HookEvent};

pub use install::{configure, remove};

pub fn handle(agent: Agent, demand: Option<DemandId>) -> Result<()> {
    // Hooks are advisory except for one bounded summary reminder on Stop.
    // A storage error must never prevent the user from working with an agent.
    match run(agent, demand) {
        Ok(Some(value)) => println!("{}", serde_json::to_string(&value)?),
        Ok(None) => println!("{{}}"),
        Err(error) => println!(
            "{}",
            json!({"systemMessage":format!("fio hook: {error:#}. Context capture was skipped; the agent may continue.")})
        ),
    }
    Ok(())
}

fn run(agent: Agent, demand: Option<DemandId>) -> Result<Option<Value>> {
    let mut source = String::new();
    io::stdin().take(1_048_577).read_to_string(&mut source)?;
    ensure!(source.len() <= 1_048_576, "Hook input exceeds 1 MiB");
    let event: Event = serde_json::from_str(&source).context("Invalid lifecycle event")?;
    if event.hook_event_name == HookEvent::Other {
        return Ok(None);
    }
    ensure!(event.cwd.is_absolute(), "Hook cwd must be an absolute path");
    let cwd = event
        .cwd
        .canonicalize()
        .context("Could not resolve hook cwd")?;
    match Workspace::containing(&cwd)? {
        Some(workspace) => lifecycle::process(&workspace, agent, &event, demand),
        None => Ok(None),
    }
}
