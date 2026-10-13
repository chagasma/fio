//! JSON projections of records for `show`, `list` and `session list`.
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, FixedOffset};
use serde::Serialize;

use crate::{
    model::{Agent, Demand, DemandId, Item, ItemKind, Session, Status, section},
    store::{demands::LoadedDemand, markdown, sessions::SessionRecord},
};

#[derive(Serialize)]
pub struct SessionView<'a> {
    /// Local fio ID; `session_id` below is the agent's native ID.
    pub id: &'a str,
    #[serde(flatten)]
    pub session: &'a Session,
    pub path: &'a Path,
    pub body: &'a str,
}

#[derive(Serialize)]
pub struct DemandView<'a> {
    #[serde(flatten)]
    pub demand: &'a Demand,
    pub path: &'a Path,
    pub body: &'a str,
    pub objective: String,
    pub current_state: String,
    pub decisions: Vec<Item>,
    pub questions: Vec<Item>,
    pub debts: Vec<Item>,
    pub children: Vec<ChildView<'a>>,
    pub sessions: Vec<SessionView<'a>>,
}

#[derive(Serialize)]
pub struct ChildView<'a> {
    pub id: &'a DemandId,
    pub title: &'a str,
    pub status: Status,
}

impl<'a> DemandView<'a> {
    pub fn new(
        demand: &'a LoadedDemand,
        sessions: &'a [SessionRecord],
        children: &'a [LoadedDemand],
    ) -> Result<Self> {
        let body = &demand.document.body;
        Ok(Self {
            demand: &demand.document.metadata,
            path: &demand.path,
            body,
            objective: markdown::section_text(body, section::OBJECTIVE)?,
            current_state: markdown::section_text(body, section::CURRENT_STATE)?,
            decisions: markdown::items(body, ItemKind::Decision)?,
            questions: markdown::items(body, ItemKind::Question)?,
            debts: markdown::items(body, ItemKind::Debt)?,
            children: children
                .iter()
                .map(|child| {
                    let child = &child.document.metadata;
                    ChildView {
                        id: &child.id,
                        title: &child.title,
                        status: child.status,
                    }
                })
                .collect(),
            sessions: sessions
                .iter()
                .map(|record| SessionView {
                    id: &record.id,
                    session: record.metadata(),
                    path: &record.path,
                    body: &record.document.body,
                })
                .collect(),
        })
    }
}

/// One open session, as listed by `fio session list --json`.
#[derive(Serialize)]
pub struct OpenSession<'a> {
    pub id: &'a str,
    pub demand: &'a DemandId,
    pub path: &'a PathBuf,
    pub agent: Agent,
    pub started: DateTime<FixedOffset>,
    pub session_id: Option<&'a str>,
    pub transcript: Option<&'a str>,
}

impl<'a> OpenSession<'a> {
    pub fn new(record: &'a SessionRecord) -> Self {
        let session = record.metadata();
        Self {
            id: &record.id,
            demand: &session.demand,
            path: &record.path,
            agent: session.agent,
            started: session.started,
            session_id: session.session_id.as_deref(),
            transcript: session.transcript.as_deref(),
        }
    }
}
