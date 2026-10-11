use std::path::PathBuf;

use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    context::{self, DEFAULT_DECISIONS, DEFAULT_SESSIONS},
    model::{Agent, DemandId, now, section},
    store::{
        demands,
        sessions::{self, SessionRecord, StartOptions},
        state::{self, WorkspaceLock},
        workspace::Workspace,
    },
};

#[derive(Clone, Copy, Deserialize, PartialEq)]
pub enum HookEvent {
    SessionStart,
    Stop,
    SessionEnd,
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Source {
    Startup,
    Resume,
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
pub struct Event {
    pub session_id: String,
    pub cwd: PathBuf,
    pub hook_event_name: HookEvent,
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub stop_hook_active: bool,
    pub last_assistant_message: Option<String>,
    pub agent_id: Option<String>,
    source: Option<Source>,
}

pub fn process(
    workspace: &Workspace,
    agent: Agent,
    event: &Event,
    demand: Option<DemandId>,
) -> Result<Option<Value>> {
    ensure!(
        !event.session_id.trim().is_empty(),
        "Missing native session ID"
    );
    // Subagents may share their parent's native ID. Never close the parent's record.
    if event.agent_id.is_some() {
        return Ok(None);
    }
    let _lock = WorkspaceLock::acquire(workspace)?;
    let mut record = find(workspace, agent, &event.session_id)?;
    if event.hook_event_name == HookEvent::SessionStart {
        match bind(workspace, agent, event, demand, record)? {
            Binding::Guidance(output) => return Ok(Some(output)),
            Binding::Record(bound) => record = *bound,
        }
    }
    let Some(mut record) = record.filter(SessionRecord::is_open) else {
        return Ok(None);
    };
    if let Some(path) = &event.transcript_path {
        record.document.set_field("transcript", path)?;
    }
    match event.hook_event_name {
        HookEvent::SessionStart => inject_context(workspace, &record),
        HookEvent::Stop => capture_turn(record, event),
        HookEvent::SessionEnd => close_record(record),
        HookEvent::Other => Ok(None),
    }
}

/// The record for this agent's native session: the open one, or else the latest.
fn find(workspace: &Workspace, agent: Agent, native: &str) -> Result<Option<SessionRecord>> {
    let mut matches: Vec<_> = sessions::scan(workspace)?
        .items
        .into_iter()
        .filter(|record| {
            let session = record.metadata();
            session.agent == agent && session.session_id.as_deref() == Some(native)
        })
        .collect();
    matches.sort_by(|a, b| (a.metadata().started, &a.path).cmp(&(b.metadata().started, &b.path)));
    ensure!(
        matches.iter().filter(|record| record.is_open()).count() <= 1,
        "Multiple open fio records use this native session ID; resolve the duplicate before running hooks"
    );
    match matches.iter().position(SessionRecord::is_open) {
        Some(index) => Ok(Some(matches.remove(index))),
        None => Ok(matches.pop()),
    }
}

enum Binding {
    Record(Box<Option<SessionRecord>>),
    /// No demand could be chosen; the agent must ask the user.
    Guidance(Value),
}

/// Opens a record for a starting native session when none is usable.
fn bind(
    workspace: &Workspace,
    agent: Agent,
    event: &Event,
    demand: Option<DemandId>,
    record: Option<SessionRecord>,
) -> Result<Binding> {
    if let (Some(record), Some(demand)) = (&record, &demand) {
        let linked = &record.metadata().demand;
        ensure!(
            linked == demand,
            "This native session is already linked to '{linked}'; start a separate native session for '{demand}'"
        );
    }
    let reopen = match &record {
        None => true,
        Some(record) => {
            !record.is_open() && matches!(event.source, Some(Source::Startup | Source::Resume))
        }
    };
    if !reopen {
        return Ok(Binding::Record(Box::new(record)));
    }
    let id = demand
        .or_else(|| {
            record
                .as_ref()
                .map(|record| record.metadata().demand.clone())
        })
        .or(state::read(workspace)?.active_demand);
    let Some(id) = id else {
        return Ok(Binding::Guidance(no_demand_guidance(agent, event)));
    };
    let handle = sessions::start_unlocked(
        workspace,
        StartOptions {
            agent,
            demand: Some(id),
            objective: Some("Agent session linked by lifecycle hook".into()),
            session_id: Some(event.session_id.clone()),
            transcript: event.transcript_path.clone(),
        },
    )?;
    let record = SessionRecord::read(handle.path, &handle.demand)?;
    Ok(Binding::Record(Box::new(Some(record))))
}

fn session_start_output(text: &str) -> Value {
    json!({"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":text}})
}

fn no_demand_guidance(agent: Agent, event: &Event) -> Value {
    let transcript = event
        .transcript_path
        .as_ref()
        .map_or_else(String::new, |path| {
            format!(" --transcript {}", shell_words::quote(path))
        });
    let command = format!(
        "fio session start --agent {} --session-id {}{transcript} --demand <chosen-id> --json",
        agent.name(),
        shell_words::quote(&event.session_id),
    );
    session_start_output(&format!(
        "fio: No active demand. Run fio list and ask the user which demand to use. Do not create or select an arbitrary demand. After the user selects a demand, open its record with {command}, substituting the chosen ID. Keep the returned local ID and update that record; SessionEnd will close it automatically."
    ))
}

fn inject_context(workspace: &Workspace, record: &SessionRecord) -> Result<Option<Value>> {
    record.save()?;
    let demand = &record.metadata().demand;
    let loaded = demands::load(workspace, demand)?;
    let package =
        context::build(workspace, &loaded, DEFAULT_SESSIONS, DEFAULT_DECISIONS)?.markdown();
    Ok(Some(session_start_output(&format!(
        "fio already opened local session '{id}' for demand '{demand}'. Do not start a duplicate session. Use explicit demand IDs. Before finishing each turn, run fio session update {id} --summary \"Work completed and verified\" --next \"Next steps and blockers\". Update --state only after rereading the demand to preserve parallel agents' progress. Record decisions, questions and debts with fio. SessionEnd closes this record automatically.\n\n{package}",
        id = record.id,
    ))))
}

fn capture_turn(mut record: SessionRecord, event: &Event) -> Result<Option<Value>> {
    if let Some(text) = &event.last_assistant_message {
        // Store the response as evidence, preserving authored session summaries.
        // Quote it so headings and list items cannot become fio sections.
        let quoted = text
            .lines()
            .map(|line| format!("> {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        if !quoted.trim().is_empty() {
            crate::store::markdown::set_section(
                &mut record.document.body,
                section::LAST_AGENT_RESPONSE,
                &quoted,
            )?;
        }
    }
    // A summary is stale when it was recorded earlier and not refreshed since the last Stop.
    let session = record.metadata();
    let revision = session.fio_summary_revision.clone();
    let stale = revision.is_some() && revision == session.fio_summary_seen;
    if let Some(revision) = revision
        && session.fio_summary_seen.as_ref() != Some(&revision)
    {
        record.document.set_field("fio_summary_seen", revision)?;
    }
    record.save()?;
    if !event.stop_hook_active && (stale || record.text(section::WORK_COMPLETED)?.is_empty()) {
        return Ok(Some(
            json!({"decision":"block","reason":format!("Before finishing, record an accurate summary and next steps with fio session update {} --summary \"...\" --next \"...\". Do not claim incomplete work is finished. Do not end this session record: SessionEnd handles closure.", record.id)}),
        ));
    }
    Ok(None)
}

fn close_record(mut record: SessionRecord) -> Result<Option<Value>> {
    if record.text(section::WORK_COMPLETED)?.is_empty() {
        crate::store::markdown::set_section(
            &mut record.document.body,
            section::WORK_COMPLETED,
            "Session ended automatically without an authored work summary. See Last agent response and the linked transcript; completion has not been verified.",
        )?;
    }
    record.document.set_field("ended", now())?;
    record.save()?;
    Ok(None)
}
