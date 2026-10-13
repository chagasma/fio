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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::Agent,
        store::{
            demands, sessions,
            testing::{TempWorkspace, id},
        },
    };

    fn started(temp: &TempWorkspace, demand: &str) -> sessions::SessionHandle {
        sessions::start(
            &temp.workspace,
            sessions::StartOptions {
                agent: Agent::Codex,
                demand: Some(id(demand)),
                objective: None,
                session_id: Some("native-1".into()),
                transcript: Some("/tmp/t.jsonl".into()),
            },
        )
        .unwrap()
    }

    #[test]
    fn demand_view_lists_children_with_id_title_and_status_only() {
        let temp = TempWorkspace::with_demands(&["p"]);
        temp.create_child("c", "p");
        let parent = demands::load(&temp.workspace, &id("p")).unwrap();
        let children = demands::children(&temp.workspace, &parent).unwrap();
        let view = DemandView::new(&parent, &[], &children).unwrap();
        let value = serde_json::to_value(&view).unwrap();
        assert_eq!(
            value["children"],
            serde_json::json!([{"id": "c", "title": "Title", "status": "backlog"}])
        );
    }

    #[test]
    fn session_view_hides_hook_bookkeeping_and_exposes_both_ids() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let handle = started(&temp, "a");
        let notes = sessions::Notes {
            summary: Some("Done".into()),
            ..sessions::Notes::default()
        };
        sessions::update(&temp.workspace, &handle.id, &notes).unwrap();

        let demand = demands::load(&temp.workspace, &id("a")).unwrap();
        let records = sessions::read(&demand).unwrap();
        assert!(records[0].metadata().fio_summary_revision.is_some());
        let view = DemandView::new(&demand, &records, &[]).unwrap();
        let value = serde_json::to_value(&view).unwrap();
        let session = &value["sessions"][0];
        assert_eq!(session["id"], handle.id.as_str());
        assert_eq!(session["session_id"], "native-1");
        assert!(session.get("fio_summary_revision").is_none());
        assert!(session.get("fio_summary_seen").is_none());
    }

    #[test]
    fn open_session_exposes_local_and_native_ids() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let handle = started(&temp, "a");
        let open = sessions::open(&temp.workspace, None).unwrap();
        let value = serde_json::to_value(OpenSession::new(&open.items[0])).unwrap();
        assert_eq!(value["id"], handle.id.as_str());
        assert_eq!(value["demand"], "a");
        assert_eq!(value["agent"], "codex");
        assert_eq!(value["session_id"], "native-1");
        assert_eq!(value["transcript"], "/tmp/t.jsonl");
    }
}
