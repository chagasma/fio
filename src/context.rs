use std::{
    collections::BTreeMap,
    fmt::{self, Write},
    path::PathBuf,
};

use anyhow::Result;
use chrono::{DateTime, FixedOffset, NaiveDate};
use serde::Serialize;

use crate::{
    model::{Agent, DemandId, Item, ItemKind, Priority, Status, section},
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
    pub parent: Option<&'a str>,
    pub sub_demands: Vec<Relation>,
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
        parent: metadata.parent.as_ref().map(DemandId::as_str),
        sub_demands: demands::children(workspace, demand)?
            .into_iter()
            .map(|child| {
                let child = child.document.metadata;
                Relation {
                    path: workspace.demand_dir(&child.id).join("demand.md"),
                    id: child.id.into(),
                    title: Some(child.title),
                    status: Some(child.status),
                    note: None,
                }
            })
            .collect(),
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
            path: id
                .parse()
                .map_or_else(
                    |_| workspace.demands_dir().join(id),
                    |parsed| workspace.demand_dir(&parsed),
                )
                .join("demand.md"),
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
        if let Some(parent) = self.parent {
            writeln!(out, "Parent: {parent}")?;
        }
        text_section(out, "Objective", &self.objective)?;
        text_section(out, "Current state", &self.current_state)?;
        item_section(out, "Open questions", &self.questions)?;
        item_section(out, "Recent decisions", &self.decisions)?;
        item_section(out, "Open debts", &self.debts)?;
        session_section(out, &self.sessions)?;
        document_section(out, &self.docs)?;
        if !self.sub_demands.is_empty() {
            let progress =
                Status::progress(self.sub_demands.iter().filter_map(|child| child.status));
            relation_section(
                out,
                &format!("Sub-demands ({progress} done)"),
                &self.sub_demands,
            )?;
        }
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        model::today,
        store::{
            demands,
            testing::{TempWorkspace, id},
        },
    };

    fn package_markdown(temp: &TempWorkspace, name: &str) -> String {
        let demand = demands::load(&temp.workspace, &id(name)).unwrap();
        build(
            &temp.workspace,
            &demand,
            DEFAULT_SESSIONS,
            DEFAULT_DECISIONS,
        )
        .unwrap()
        .markdown()
    }

    /// Replaces the first occurrence of `from` in the demand file.
    fn rewrite(temp: &TempWorkspace, name: &str, from: &str, to: &str) {
        let path = temp.workspace.demand_dir(&id(name)).join("demand.md");
        let source = fs::read_to_string(&path).unwrap();
        assert!(source.contains(from), "{from:?} not in demand");
        fs::write(path, source.replacen(from, to, 1)).unwrap();
    }

    #[test]
    fn empty_demand_renders_placeholders() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let text = package_markdown(&temp, "a");
        assert!(text.starts_with("# a — Title\n\nProject: test\nStatus: backlog\n"));
        assert!(text.contains("## Objective\n\nNot recorded."));
        assert!(text.contains("## Open questions\n\nNone."));
        assert!(!text.contains("Parent:"));
        assert!(!text.contains("Sub-demands"));
    }

    #[test]
    fn item_date_reads_the_date_written_by_add_item() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut locked = demands::load_for_update(&temp.workspace, &id("a")).unwrap();
        locked.add_item(ItemKind::Decision, "choice").unwrap();
        let body = &locked.document.body;
        let items = markdown::items(body, ItemKind::Decision).unwrap();
        assert_eq!(item_date(&items[0]), Some(today()));
    }

    #[test]
    fn decisions_are_oldest_first_by_date_and_limited_to_the_most_recent() {
        let temp = TempWorkspace::with_demands(&["a"]);
        rewrite(
            &temp,
            "a",
            "## Decisions\n",
            "## Decisions\n\n- DEC2 (2026-03-01): newest\n- DEC1 (2026-01-01): oldest\n- DEC3 (2026-02-01): middle\n",
        );
        let demand = demands::load(&temp.workspace, &id("a")).unwrap();
        let package = build(&temp.workspace, &demand, 3, 2).unwrap();
        let ids: Vec<_> = package
            .decisions
            .iter()
            .filter_map(|d| d.id.as_deref())
            .collect();
        assert_eq!(ids, ["DEC2", "DEC3"]);
    }

    #[test]
    fn only_open_questions_and_debts_are_included() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut locked = demands::load_for_update(&temp.workspace, &id("a")).unwrap();
        locked.add_item(ItemKind::Question, "open q").unwrap();
        locked.add_item(ItemKind::Question, "closed q").unwrap();
        locked
            .complete_item(ItemKind::Question, "Q2", Some("yes"))
            .unwrap();
        locked.add_item(ItemKind::Debt, "open d").unwrap();
        locked.add_item(ItemKind::Debt, "closed d").unwrap();
        locked.complete_item(ItemKind::Debt, "D2", None).unwrap();
        drop(locked);

        let text = package_markdown(&temp, "a");
        assert!(text.contains("open q") && text.contains("open d"));
        assert!(!text.contains("closed q") && !text.contains("closed d"));
    }

    #[test]
    fn sub_demands_report_progress_and_children_name_their_parent() {
        let temp = TempWorkspace::with_demands(&["p"]);
        temp.create_child("c1", "p");
        temp.create_child("c2", "p");
        demands::load_for_update(&temp.workspace, &id("c1"))
            .unwrap()
            .set_status(Status::Done)
            .unwrap();

        let parent = package_markdown(&temp, "p");
        assert!(parent.contains("## Sub-demands (1/2 done)"));
        assert!(parent.contains("- c1 — Title (done)"));
        assert!(package_markdown(&temp, "c1").contains("Parent: p\n"));
    }

    #[test]
    fn relations_to_missing_demands_are_reported_as_unavailable() {
        let temp = TempWorkspace::with_demands(&["a", "other"]);
        rewrite(&temp, "a", "related: []", "related:\n- other\n- ghost");
        let text = package_markdown(&temp, "a");
        assert!(text.contains("- other — Title (backlog);"));
        assert!(text.contains("- ghost — unavailable:"));
    }

    #[test]
    fn sessions_are_limited_and_marked_in_progress() {
        let temp = TempWorkspace::with_demands(&["a"]);
        for _ in 0..3 {
            temp.start_session("a", None, Some("Objective text"));
        }
        let demand = demands::load(&temp.workspace, &id("a")).unwrap();
        let package = build(&temp.workspace, &demand, 2, 10).unwrap();
        assert_eq!(package.sessions.len(), 2);
        assert!(package.markdown().contains("(in progress)"));
        assert!(package.markdown().contains("Objective: Objective text"));
    }

    #[test]
    fn json_exposes_the_documented_fields() {
        let temp = TempWorkspace::with_demands(&["p"]);
        temp.create_child("c", "p");
        let demand = demands::load(&temp.workspace, &id("p")).unwrap();
        let package = build(&temp.workspace, &demand, 3, 10).unwrap();
        let value = serde_json::to_value(&package).unwrap();
        for key in [
            "id",
            "parent",
            "sub_demands",
            "title",
            "project",
            "status",
            "priority",
            "created",
            "updated",
            "objective",
            "current_state",
            "links",
            "questions",
            "decisions",
            "debts",
            "sessions",
            "docs",
            "related",
            "blocked_by",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        assert_eq!(value["sub_demands"][0]["id"], "c");
        assert_eq!(value["sub_demands"][0]["status"], "backlog");
    }
}
