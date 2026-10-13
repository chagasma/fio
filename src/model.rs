use std::{collections::BTreeMap, fmt, path::PathBuf, str::FromStr};

use chrono::{DateTime, FixedOffset, Local, NaiveDate, SubsecRound};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// Markdown section titles used by demand and session records.
pub mod section {
    pub const OBJECTIVE: &str = "Objective";
    pub const CURRENT_STATE: &str = "Current state";
    pub const SESSION_OBJECTIVE: &str = "Session objective";
    pub const WORK_COMPLETED: &str = "Work completed";
    pub const NEXT_STEPS: &str = "Next steps";
    pub const LAST_AGENT_RESPONSE: &str = "Last agent response";
}

#[derive(Deserialize, Serialize)]
pub struct Project {
    pub name: String,
    #[serde(flatten)]
    pub settings: toml::Table,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct RegisteredProject {
    pub name: String,
    pub workspace: PathBuf,
}

/// A validated demand identifier, safe to use as a directory name.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct DemandId(String);

impl DemandId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for DemandId {
    type Err = String;

    fn from_str(id: &str) -> Result<Self, String> {
        let valid = !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            && id.bytes().next().is_some_and(|c| c.is_ascii_alphanumeric());
        if valid {
            Ok(Self(id.to_owned()))
        } else {
            Err("Invalid ID: use letters, digits, hyphens or underscores, starting with a letter or digit (up to 128 characters)".into())
        }
    }
}

impl TryFrom<String> for DemandId {
    type Error = String;

    fn try_from(id: String) -> Result<Self, String> {
        id.parse()
    }
}

impl From<DemandId> for String {
    fn from(id: DemandId) -> Self {
        id.0
    }
}

impl fmt::Display for DemandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<std::path::Path> for DemandId {
    fn as_ref(&self) -> &std::path::Path {
        self.0.as_ref()
    }
}

/// Cycles through `all`, passing through `None` (no filter) between rounds.
fn cycle<T: Copy + PartialEq>(all: &[T], current: Option<T>) -> Option<T> {
    match current {
        None => all.first().copied(),
        Some(value) => {
            let index = all.iter().position(|candidate| *candidate == value)?;
            all.get(index + 1).copied()
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, ValueEnum, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Backlog,
    Active,
    Paused,
    Blocked,
    Done,
    Archived,
}

impl Status {
    pub const ALL: [Self; 6] = [
        Self::Backlog,
        Self::Active,
        Self::Paused,
        Self::Blocked,
        Self::Done,
        Self::Archived,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Archived => "archived",
        }
    }

    pub fn next_filter(current: Option<Self>) -> Option<Self> {
        cycle(&Self::ALL, current)
    }

    pub fn is_closed(self) -> bool {
        matches!(self, Self::Done | Self::Archived)
    }

    /// "done/total" over a set of statuses.
    pub fn progress(statuses: impl IntoIterator<Item = Self>) -> String {
        let (done, total) = statuses.into_iter().fold((0, 0), |(done, total), status| {
            (done + usize::from(status.is_closed()), total + 1)
        });
        format!("{done}/{total}")
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, ValueEnum, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    #[default]
    Medium,
    High,
}

impl Priority {
    const ALL: [Self; 3] = [Self::High, Self::Medium, Self::Low];

    pub fn next_filter(current: Option<Self>) -> Option<Self> {
        cycle(&Self::ALL, current)
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, ValueEnum, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    #[default]
    Codex,
}

impl Agent {
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

/// Kinds of list items kept in demand sections.
#[derive(Clone, Copy, PartialEq)]
pub enum ItemKind {
    Decision,
    Question,
    Debt,
}

impl ItemKind {
    pub const ALL: [Self; 3] = [Self::Decision, Self::Question, Self::Debt];

    pub fn section(self) -> &'static str {
        match self {
            Self::Decision => "Decisions",
            Self::Question => "Questions",
            Self::Debt => "Debts",
        }
    }

    pub fn prefix(self) -> &'static str {
        match self {
            Self::Decision => "DEC",
            Self::Question => "Q",
            Self::Debt => "D",
        }
    }

    /// Decisions are recorded facts; questions and debts can be completed.
    pub fn has_checkbox(self) -> bool {
        self != Self::Decision
    }

    pub fn owns(self, id: &str) -> bool {
        id.strip_prefix(self.prefix())
            .is_some_and(|number| !number.is_empty() && number.bytes().all(|c| c.is_ascii_digit()))
    }
}

pub fn now() -> DateTime<FixedOffset> {
    Local::now().fixed_offset().trunc_subsecs(0)
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

#[derive(Deserialize, Serialize)]
pub struct Demand {
    pub id: DemandId,
    pub title: String,
    pub project: String,
    #[serde(default)]
    pub status: Status,
    #[serde(default)]
    pub priority: Priority,
    /// Set for sub-demands, which live in the folder of their parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<DemandId>,
    pub created: NaiveDate,
    pub updated: NaiveDate,
    #[serde(default)]
    pub links: BTreeMap<String, String>,
    #[serde(default)]
    pub related: Vec<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default)]
    pub docs: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Deserialize, Serialize)]
#[allow(clippy::struct_field_names)] // `session_id` is the native agent ID, part of the record format
pub struct Session {
    pub agent: Agent,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub transcript: Option<String>,
    pub demand: DemandId,
    pub started: DateTime<FixedOffset>,
    #[serde(default)]
    pub ended: Option<DateTime<FixedOffset>>,
    #[serde(default)]
    pub branch: Option<String>,
    /// Hook bookkeeping, kept out of public JSON output.
    #[serde(default, skip_serializing)]
    pub fio_summary_revision: Option<String>,
    #[serde(default, skip_serializing)]
    pub fio_summary_seen: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize)]
pub struct Item {
    pub id: Option<String>,
    pub done: bool,
    pub text: String,
}
