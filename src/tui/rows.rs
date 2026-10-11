use std::path::Path;

use anyhow::Result;

use super::{
    App,
    screen::{Row, Screen, Tab},
};
use crate::{
    index::HitKind,
    model::{DemandId, ItemKind, Status, section},
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
                label: format!("{}    {}", project.name, project.workspace.display()),
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
                Some(Row {
                    label: format!(
                        "{}/{} [{}] {}\n  {}",
                        hit.project,
                        hit.demand,
                        hit.kind.name(),
                        hit.title,
                        hit.snippet.replace('\n', " ")
                    ),
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
            let prefix = format!(
                "{}/{}{}  [{} / {}]",
                demand.project,
                demand.id,
                if is_default { " *" } else { "" },
                demand.status,
                demand.priority
            );
            let kind = match self.screen {
                Screen::Questions => Some(ItemKind::Question),
                Screen::Debts => Some(ItemKind::Debt),
                _ => None,
            };
            let Some(kind) = kind else {
                rows.push(Row {
                    label: format!("{prefix}  {}", demand.title),
                    target: Screen::Detail(entry.key()),
                    tab: Tab::Summary,
                });
                continue;
            };
            let texts = markdown::open_items(&entry.loaded.document.body, kind).map_or_else(
                |error| vec![format!("Could not read {}: {error:#}", kind.section())],
                |items| items.into_iter().map(|item| item.text).collect(),
            );
            rows.extend(texts.into_iter().map(|text| Row {
                label: format!("{prefix}  {text}"),
                target: Screen::Detail(entry.key()),
                tab: Tab::for_kind(kind),
            }));
        }
        rows
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
            Tab::Summary => format!(
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
            ),
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
