use ratatui::widgets::ListState;

use super::{
    App,
    screen::{RowId, Screen, Tab},
};
use crate::model::{Priority, Status};

/// A location includes its view state, so returning restores the user's place.
pub(super) struct Location {
    pub screen: Screen,
    selection: ListState,
    selected: Option<RowId>,
    tab: Tab,
    scroll: u16,
    status: Option<Status>,
    priority: Option<Priority>,
    query: String,
}

impl App {
    pub(super) fn go(&mut self, screen: Screen) {
        self.history.push(Location {
            screen: self.screen.clone(),
            selection: self.selection,
            selected: self.selected_row().map(|row| row.id.clone()),
            tab: self.tab,
            scroll: self.scroll,
            status: self.status,
            priority: self.priority,
            query: self.query.clone(),
        });
        self.screen = screen;
        self.selection = ListState::default().with_selected(Some(0));
        self.scroll = 0;
        self.invalidate();
    }

    pub(super) fn back(&mut self) {
        if let Some(location) = self.history.pop() {
            self.screen = location.screen;
            self.selection = location.selection;
            self.tab = location.tab;
            self.scroll = location.scroll;
            self.status = location.status;
            self.priority = location.priority;
            self.query = location.query;
            self.invalidate();
            self.restore_selection(location.selected.as_ref());
        }
    }

    pub(super) fn restore_selection(&mut self, selected: Option<&RowId>) {
        let position = selected
            .and_then(|id| self.rows.iter().position(|row| &row.id == id))
            .unwrap_or_else(|| self.selection.selected().unwrap_or(0))
            .min(self.rows.len().saturating_sub(1));
        self.selection.select(Some(position));
    }
}
