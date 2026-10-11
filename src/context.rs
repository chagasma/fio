use std::{
    collections::BTreeMap,
    fmt::{self, Write},
    path::PathBuf,
};

use anyhow::Result;
use chrono::{DateTime, FixedOffset, NaiveDate};
use serde::Serialize;

use crate::{
    model::{Agent, Item, ItemKind, Priority, Status, section},
    store::{
        demands::{self, LoadedDemand},
        markdown,
        sessions::{self, SessionRecord},
        workspace::Workspace,
    },
};

pub const DEFAULT_SESSIONS: usize = 3;
pub const DEFAULT_DECISIONS: usize = 10;

/// Context handed to agents. `docs` and relations are absolute, unlike `fio show --json`.
#[derive(Serialize)]
pub struct ContextPackage<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub project: &'a str,
    pub status: Status,
    pub priority: Priority,
    pub created: NaiveDate,
    pub updated: NaiveDate,
    pub objective: String,
    pub current_state: String,
    pub links: &'a BTreeMap<String, String>,
    pub questions: Vec<Item>,
    pub decisions: Vec<Item>,
    pub debts: Vec<Item>,
    pub sessions: Vec<SessionSummary>,
    pub docs: Vec<PathBuf>,
    pub related: Vec<Relation>,
    pub blocked_by: Vec<Relation>,
}

#[derive(Serialize)]
pub struct SessionSummary {
    pub path: PathBuf,
    pub agent: Agent,
    pub started: DateTime<FixedOffset>,
    pub ended: Option<DateTime<FixedOffset>>,
    pub objective: String,
    pub summary: String,
    pub next_steps: String,
}

#[derive(Serialize)]
pub struct Relation {
    pub id: String,
    pub title: Option<String>,
    pub status: Option<Status>,
    pub path: PathBuf,
    pub note: Option<String>,
}

pub fn build<'a>(
    workspace: &Workspace,
    demand: &'a LoadedDemand,
    session_count: usize,
    decision_count: usize,
) -> Result<ContextPackage<'a>> {
    let metadata = &demand.document.metadata;
    let body = &demand.document.body;
    let mut decisions = markdown::items(body, ItemKind::Decision)?;
    decisions.sort_by_key(item_date);
    let mut records = sessions::read(demand)?;
    records.sort_by(|a, b| (a.metadata().started, &a.path).cmp(&(b.metadata().started, &b.path)));
    Ok(ContextPackage {
        id: metadata.id.as_str(),
        title: &metadata.title,
        project: &metadata.project,
        status: metadata.status,
        priority: metadata.priority,
        created: metadata.created,
        updated: metadata.updated,
        objective: markdown::section_text(body, section::OBJECTIVE)?,
        current_state: markdown::section_text(body, section::CURRENT_STATE)?,
        links: &metadata.links,
        questions: markdown::open_items(body, ItemKind::Question)?,
        debts: markdown::open_items(body, ItemKind::Debt)?,
        decisions: decisions.into_iter().rev().take(decision_count).collect(),
        sessions: records
            .iter()
            .rev()
            .take(session_count)
            .map(session_summary)
            .collect::<Result<_>>()?,
        docs: metadata
            .docs
            .iter()
            .map(|doc| workspace.root().join(doc))
            .collect(),
        related: metadata
            .related
            .iter()
            .map(|id| relation(workspace, id))
            .collect(),
        blocked_by: metadata
            .blocked_by
            .iter()
            .map(|id| relation(workspace, id))
            .collect(),
    })
}

fn item_date(item: &Item) -> Option<NaiveDate> {
    item.text.split_whitespace().find_map(|word| {
        NaiveDate::parse_from_str(word.trim_start_matches('(').get(..10)?, "%Y-%m-%d").ok()
    })
}

fn session_summary(record: &SessionRecord) -> Result<SessionSummary> {
    let session = record.metadata();
    Ok(SessionSummary {
        objective: record.text(section::SESSION_OBJECTIVE)?,
        summary: record.text(section::WORK_COMPLETED)?,
        next_steps: record.text(section::NEXT_STEPS)?,
        path: record.path.clone(),
        agent: session.agent,
        started: session.started,
        ended: session.ended,
    })
}

fn relation(workspace: &Workspace, id: &str) -> Relation {
    let loaded = id
        .parse()
        .map_err(anyhow::Error::msg)
        .and_then(|parsed| demands::load(workspace, &parsed));
    match loaded {
        Ok(demand) => Relation {
            id: id.to_owned(),
            title: Some(demand.document.metadata.title),
            status: Some(demand.document.metadata.status),
            path: demand.path,
            note: None,
        },
        Err(error) => Relation {
            id: id.to_owned(),
            title: None,
            status: None,
            path: workspace.demands_dir().join(id).join("demand.md"),
            note: Some(error.to_string()),
        },
    }
}

impl ContextPackage<'_> {
    pub fn markdown(&self) -> String {
        let mut output = String::new();
        self.render(&mut output)
            .expect("writing to a String cannot fail");
        output
    }

    fn render(&self, out: &mut String) -> fmt::Result {
        write!(
            out,
            "# {} — {}\n\nProject: {}\nStatus: {}\n",
            self.id, self.title, self.project, self.status
        )?;
        text_section(out, "Objective", &self.objective)?;
        text_section(out, "Current state", &self.current_state)?;
        item_section(out, "Open questions", &self.questions)?;
        item_section(out, "Recent decisions", &self.decisions)?;
        item_section(out, "Open debts", &self.debts)?;
        session_section(out, &self.sessions)?;
        document_section(out, &self.docs)?;
        relation_section(out, "Related demands", &self.related)?;
        relation_section(out, "Blocked by", &self.blocked_by)?;
        if !self.links.is_empty() {
            out.push_str("\n## External links\n\n");
            for (name, url) in self.links {
                writeln!(out, "- {name}: {url}")?;
            }
        }
        Ok(())
    }
}

fn or_empty(text: &str) -> &str {
    if text.is_empty() {
        "Not recorded."
    } else {
        text
    }
}

fn text_section(out: &mut String, title: &str, text: &str) -> fmt::Result {
    writeln!(out, "\n## {title}\n\n{}", or_empty(text))
}

fn list_header(out: &mut String, title: &str, empty: bool) -> fmt::Result {
    writeln!(out, "\n## {title}\n")?;
    if empty {
        out.push_str("None.\n");
    }
    Ok(())
}

fn item_section(out: &mut String, title: &str, items: &[Item]) -> fmt::Result {
    list_header(out, title, items.is_empty())?;
    for item in items {
        writeln!(out, "- {}", item.text)?;
    }
    Ok(())
}

fn relation_section(out: &mut String, title: &str, relations: &[Relation]) -> fmt::Result {
    list_header(out, title, relations.is_empty())?;
    for relation in relations {
        if let (Some(title), Some(status)) = (&relation.title, relation.status) {
            writeln!(
                out,
                "- {} — {title} ({status}); `{}`",
                relation.id,
                relation.path.display()
            )?;
        } else {
            writeln!(
                out,
                "- {} — unavailable: {}",
                relation.id,
                relation.note.as_deref().unwrap_or("not found")
            )?;
        }
    }
    Ok(())
}

fn session_section(out: &mut String, sessions: &[SessionSummary]) -> fmt::Result {
    list_header(out, "Recent sessions", sessions.is_empty())?;
    for session in sessions {
        let status = if session.ended.is_some() {
            "closed"
        } else {
            "in progress"
        };
        writeln!(
            out,
            "### {} — {} ({status})\n",
            session.started.format("%Y-%m-%dT%H:%M:%S%:z"),
            session.agent.name()
        )?;
        writeln!(out, "Record: `{}`", session.path.display())?;
        writeln!(out, "\nObjective: {}", or_empty(&session.objective))?;
        writeln!(out, "\nSummary: {}", or_empty(&session.summary))?;
        writeln!(out, "\nNext steps: {}", or_empty(&session.next_steps))?;
    }
    Ok(())
}

fn document_section(out: &mut String, docs: &[PathBuf]) -> fmt::Result {
    list_header(out, "Linked documents", docs.is_empty())?;
    for doc in docs {
        writeln!(out, "- `{}`", doc.display())?;
    }
    Ok(())
}
