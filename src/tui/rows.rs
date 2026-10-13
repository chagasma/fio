use std::{fmt::Write, path::Path};

use anyhow::Result;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};

use super::{
    App,
    screen::{Row, Screen, Tab},
    theme,
};
use crate::{
    index::HitKind,
    model::{Demand, DemandId, ItemKind, Status, section},
    store::{
        catalog::{DemandKey, Entry},
        markdown, sessions,
    },
};

const SEARCH_LIMIT: usize = 200;

impl App {
    pub(super) fn build_rows(&self) -> Result<Vec<Row>> {
        match &self.screen {
            Screen::Projects => Ok(self.project_rows()),
            Screen::Search => self.search_rows(),
            Screen::Detail(_) => Ok(Vec::new()),
            _ => Ok(self.demand_rows()),
        }
    }

    fn project_rows(&self) -> Vec<Row> {
        self.catalog
            .projects
            .iter()
            .map(|project| Row {
                label: Line::from(vec![
                    Span::styled(project.name.clone(), theme::heading()),
                    Span::raw("   "),
                    Span::styled(project.workspace.display().to_string(), theme::muted()),
                ])
                .into(),
                target: Screen::Demands(Some(project.workspace.clone())),
                tab: Tab::Summary,
            })
            .collect()
    }

    fn search_rows(&self) -> Result<Vec<Row>> {
        let hits = self.index.search(&self.query, SEARCH_LIMIT)?;
        Ok(hits
            .into_iter()
            .filter_map(|hit| {
                let key = DemandKey {
                    workspace: hit.workspace.clone(),
                    id: hit.demand.parse::<DemandId>().ok()?,
                };
                self.catalog.find(&key)?;
                let title = Line::from(vec![
                    Span::styled(format!("{}/{}", hit.project, hit.demand), theme::heading()),
                    Span::styled(format!("  [{}]  ", hit.kind.name()), theme::muted()),
                    Span::raw(hit.title.clone()),
                ]);
                let mut snippet = vec![Span::raw("    ")];
                snippet.extend(highlight(&hit.snippet.replace('\n', " ")));
                Some(Row {
                    label: Text::from(vec![title, Line::from(snippet)]),
                    target: Screen::Detail(key),
                    tab: if hit.kind == HitKind::Session {
                        Tab::Sessions
                    } else {
                        Tab::Summary
                    },
                })
            })
            .collect())
    }

    fn demand_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let name_width = self
            .catalog
            .demands
            .iter()
            .map(|entry| {
                display_name(&entry.loaded.document.metadata)
                    .chars()
                    .count()
            })
            .max()
            .unwrap_or(0);
        for entry in &self.catalog.demands {
            let demand = &entry.loaded.document.metadata;
            if self.status.is_some_and(|status| demand.status != status)
                || self
                    .priority
                    .is_some_and(|priority| demand.priority != priority)
            {
                continue;
            }
            match &self.screen {
                Screen::Demands(Some(workspace)) if &entry.workspace != workspace => continue,
                Screen::Active if !matches!(demand.status, Status::Active | Status::Blocked) => {
                    continue;
                }
                _ => (),
            }
            let is_default = self.catalog.active.get(&entry.workspace) == Some(&demand.id);
            let name = display_name(demand);
            let mut prefix = vec![
                Span::styled(format!("{name:<name_width$}"), theme::heading()),
                if is_default {
                    Span::styled(
                        " *",
                        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::raw("  ")
                },
            ];
            prefix.extend([
                Span::raw("  "),
                Span::styled(
                    format!("{:<8}", demand.status.name()),
                    theme::status(demand.status),
                ),
                Span::styled(
                    format!(" {:<6}", demand.priority.to_string()),
                    theme::priority(demand.priority),
                ),
                Span::raw("  "),
            ]);
            let kind = match self.screen {
                Screen::Questions => Some(ItemKind::Question),
                Screen::Debts => Some(ItemKind::Debt),
                _ => None,
            };
            let Some(kind) = kind else {
                let mut spans = prefix;
                spans.push(Span::raw(demand.title.clone()));
                rows.push(Row {
                    label: Line::from(spans).into(),
                    target: Screen::Detail(entry.key()),
                    tab: Tab::Summary,
                });
                continue;
            };
            let texts = markdown::open_items(&entry.loaded.document.body, kind).map_or_else(
                |error| vec![format!("Could not read {}: {error:#}", kind.section())],
                |items| items.into_iter().map(|item| item.text).collect(),
            );
            rows.extend(texts.into_iter().map(|text| {
                let mut spans = prefix.clone();
                spans.push(Span::raw(text));
                Row {
                    label: Line::from(spans).into(),
                    target: Screen::Detail(entry.key()),
                    tab: Tab::for_kind(kind),
                }
            }));
        }
        rows
    }

    /// Parent and sub-demand lines for the summary tab.
    fn family(&self, entry: &Entry) -> String {
        let demand = &entry.loaded.document.metadata;
        let children: Vec<_> = self.catalog.children_of(entry).collect();
        let mut text = String::new();
        if let Some(parent) = &demand.parent {
            let _ = write!(text, "\n\nParent: {parent}");
        }
        if !children.is_empty() {
            let progress = Status::progress(
                children
                    .iter()
                    .map(|child| child.loaded.document.metadata.status),
            );
            let _ = write!(text, "\n\nSub-demands ({progress} done)");
            for child in children {
                let child = &child.loaded.document.metadata;
                let _ = write!(text, "\n{} [{}] {}", child.id, child.status, child.title);
            }
        }
        text
    }

    /// Text shown for the current tab of a demand.
    pub(super) fn detail_text(&self, entry: &Entry) -> Result<String> {
        let loaded = &entry.loaded;
        let demand = &loaded.document.metadata;
        let body = &loaded.document.body;
        if let Some(kind) = self.tab.item_kind() {
            return markdown::section_text(body, kind.section());
        }
        Ok(match self.tab {
            Tab::Summary => {
                format!(
                    "{} / {}\n{}\nStatus: {}    Priority: {}\nCreated: {}    Updated: {}\n\nObjective\n{}\n\nCurrent state\n{}\n\nLinks\n{}\n\nRelated: {}\nBlocked by: {}\n\n{}",
                    demand.project,
                    demand.id,
                    demand.title,
                    demand.status,
                    demand.priority,
                    demand.created,
                    demand.updated,
                    markdown::section_text(body, section::OBJECTIVE)?,
                    markdown::section_text(body, section::CURRENT_STATE)?,
                    demand
                        .links
                        .iter()
                        .map(|(key, value)| format!("{key}: {value}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    demand.related.join(", "),
                    demand.blocked_by.join(", "),
                    loaded.path.display()
                ) + &self.family(entry)
            }
            Tab::Sessions => sessions::read(loaded)?
                .iter()
                .rev()
                .map(|record| {
                    let session = record.metadata();
                    let recorded = |value: &Option<String>| {
                        value.clone().unwrap_or_else(|| "Not recorded".into())
                    };
                    format!(
                        "{} | {} | {}\nNative session: {}\nTranscript: {}\nBranch: {}\n{}\n{}",
                        session.agent.name(),
                        session.started.format("%Y-%m-%dT%H:%M:%S%:z"),
                        session
                            .ended
                            .map_or_else(|| "Open".into(), |ended| ended.to_rfc3339()),
                        recorded(&session.session_id),
                        recorded(&session.transcript),
                        recorded(&session.branch),
                        record.path.display(),
                        record.document.body
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
            _ => demand
                .docs
                .iter()
                .map(|doc| Path::new(&entry.workspace).join(doc).display().to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        })
    }
}

/// Label of a demand in lists: sub-demands are indented under their parent.
fn display_name(demand: &Demand) -> String {
    if let Some(parent) = &demand.parent {
        format!("  └ {}/{}", parent, demand.id)
    } else {
        format!("{}/{}", demand.project, demand.id)
    }
}

/// Splits a search snippet at its `[match]` markers and emphasizes the matches.
fn highlight(snippet: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = snippet;
    while let Some(start) = rest.find('[') {
        let Some(end) = rest[start..].find(']') else {
            break;
        };
        spans.push(Span::styled(rest[..start].to_owned(), theme::muted()));
        spans.push(Span::styled(
            rest[start + 1..start + end].to_owned(),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
        rest = &rest[start + end + 1..];
    }
    spans.push(Span::styled(rest.to_owned(), theme::muted()));
    spans
}
