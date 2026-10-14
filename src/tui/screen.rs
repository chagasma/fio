use std::path::PathBuf;

use ratatui::text::Text;

use crate::{
    model::{DemandId, ItemKind},
    store::catalog::DemandKey,
};

#[derive(Clone, PartialEq)]
pub enum Screen {
    Projects,
    /// Demands of one project, or of all projects.
    Demands(Option<PathBuf>),
    Active,
    Questions,
    Debts,
    Search,
    Detail(DemandKey),
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Summary,
    Sessions,
    Decisions,
    Questions,
    Debts,
    Docs,
}

impl Tab {
    pub const ALL: [Self; 6] = [
        Self::Summary,
        Self::Sessions,
        Self::Decisions,
        Self::Questions,
        Self::Debts,
        Self::Docs,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::Sessions => "Sessions",
            Self::Decisions => "Decisions",
            Self::Questions => "Questions",
            Self::Debts => "Debts",
            Self::Docs => "Docs",
        }
    }

    fn position(self) -> usize {
        Self::ALL
            .iter()
            .position(|tab| *tab == self)
            .expect("tab is listed")
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.position() + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        Self::ALL[(self.position() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    /// The tab selected by the digit key `1`–`6`.
    pub fn from_digit(digit: char) -> Option<Self> {
        let index = digit.to_digit(10)?.checked_sub(1)?;
        Self::ALL.get(index as usize).copied()
    }

    /// The item list a tab shows, for the tabs that map to one.
    pub fn item_kind(self) -> Option<ItemKind> {
        match self {
            Self::Decisions => Some(ItemKind::Decision),
            Self::Questions => Some(ItemKind::Question),
            Self::Debts => Some(ItemKind::Debt),
            _ => None,
        }
    }

    pub fn for_kind(kind: ItemKind) -> Self {
        match kind {
            ItemKind::Decision => Self::Decisions,
            ItemKind::Question => Self::Questions,
            ItemKind::Debt => Self::Debts,
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum RowId {
    Project(PathBuf),
    Demand(DemandKey),
    Item(DemandKey, String),
    Document(PathBuf),
}

pub struct Row {
    pub id: RowId,
    pub label: Text<'static>,
    pub target: Screen,
    pub tab: Tab,
}

pub struct Preview {
    pub title: String,
    pub text: String,
    pub scroll: u16,
    pub max_scroll: u16,
}

impl Preview {
    pub fn new(title: String, text: String) -> Self {
        Self {
            title,
            text,
            scroll: 0,
            max_scroll: 0,
        }
    }
}

/// Where a demand being created goes: a workspace, and optionally a parent demand.
#[derive(Clone)]
pub struct NewDemand {
    pub workspace: PathBuf,
    pub parent: Option<DemandId>,
}

/// A text entry in progress, with whatever it needs to complete.
pub enum Prompt {
    NewId(NewDemand),
    NewTitle(NewDemand, DemandId),
    AddItem(DemandKey, ItemKind),
    Answer(DemandKey, String),
    Search,
    Command,
}

impl Prompt {
    pub fn label(&self) -> String {
        match self {
            Self::NewId(NewDemand { parent: None, .. }) => "New demand ID: ".into(),
            Self::NewId(NewDemand {
                parent: Some(parent),
                ..
            }) => format!("New sub-demand ID (in {parent}): "),
            Self::NewTitle(..) => "New demand title: ".into(),
            Self::AddItem(_, kind) => format!("New {}: ", kind.section().to_lowercase()),
            Self::Answer(_, id) => format!("Answer {id}: "),
            Self::Search => "/".into(),
            Self::Command => ":".into(),
        }
    }

    /// Prompts that keep their text when submission fails.
    pub fn is_draft(&self) -> bool {
        !matches!(self, Self::Search | Self::Command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_wrap_in_both_directions() {
        assert!(Tab::Docs.next() == Tab::Summary);
        assert!(Tab::Summary.previous() == Tab::Docs);
        assert!(Tab::Summary.next() == Tab::Sessions);
    }

    #[test]
    fn digits_one_to_six_select_tabs() {
        assert!(Tab::from_digit('1') == Some(Tab::Summary));
        assert!(Tab::from_digit('6') == Some(Tab::Docs));
        assert!(Tab::from_digit('0').is_none());
        assert!(Tab::from_digit('7').is_none());
        assert!(Tab::from_digit('x').is_none());
    }

    #[test]
    fn item_tabs_map_to_item_kinds_and_back() {
        for kind in ItemKind::ALL {
            assert!(Tab::for_kind(kind).item_kind() == Some(kind));
        }
        assert!(Tab::Summary.item_kind().is_none());
    }

    #[test]
    fn prompt_labels_mention_the_parent_and_the_statuses() {
        let new = NewDemand {
            workspace: PathBuf::from("/w"),
            parent: None,
        };
        assert_eq!(Prompt::NewId(new.clone()).label(), "New demand ID: ");
        let child = NewDemand {
            parent: Some("p".parse().unwrap()),
            ..new
        };
        assert!(Prompt::NewId(child).label().contains("(in p)"));
        let answer = Prompt::Answer(
            DemandKey {
                workspace: PathBuf::from("/w"),
                id: "a".parse().unwrap(),
            },
            "Q2".into(),
        );
        assert_eq!(answer.label(), "Answer Q2: ");
    }

    #[test]
    fn only_search_and_command_prompts_discard_their_text() {
        assert!(!Prompt::Search.is_draft());
        assert!(!Prompt::Command.is_draft());
        let new = NewDemand {
            workspace: PathBuf::from("/w"),
            parent: None,
        };
        assert!(Prompt::NewId(new).is_draft());
    }
}
