mod actions;
mod input;
mod mascot;
mod rows;
mod screen;
mod theme;
mod ui;
mod watch;

use std::{
    io::IsTerminal,
    time::{Duration, Instant},
};

use anyhow::{Result, ensure};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;

use crate::{
    index::Index,
    model::{Priority, Status},
    store::catalog::Catalog,
};
use actions::{External, Picker};
use screen::{Prompt, Row, Screen, Tab};

/// What a key event did to the event loop.
enum KeyOutcome {
    Ignored,
    Handled,
    Quit,
}

/// What currently captures keyboard input.
enum Modal {
    None,
    Input { prompt: Prompt, text: String },
    Picker(Picker),
    Help,
}

/// Reads the projects again; swapped out in tests so they never touch the user's registry or cache.
type Loader = Box<dyn Fn() -> Result<(Catalog, Index)>>;

fn load_from_disk() -> Result<(Catalog, Index)> {
    let catalog = Catalog::load()?;
    let index = Index::rebuild(&catalog)?;
    Ok((catalog, index))
}

struct App {
    load: Loader,
    catalog: Catalog,
    index: Index,
    screen: Screen,
    history: Vec<Screen>,
    selection: ListState,
    tab: Tab,
    scroll: u16,
    max_scroll: u16,
    status: Option<Status>,
    priority: Option<Priority>,
    query: String,
    message: String,
    modal: Modal,
    quit: bool,
    external: Option<External>,
    clipboard: Option<arboard::Clipboard>,
    /// Rows of the current list screen; rebuilt by `invalidate`.
    rows: Vec<Row>,
    rows_error: Option<String>,
    /// Rendered text of the current detail screen; cleared by `invalidate`.
    detail: Option<String>,
    /// When the TUI started; drives the mascot animation.
    started: Instant,
}

pub fn run() -> Result<()> {
    ensure!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        "The TUI requires an interactive terminal. Use fio --help for CLI commands."
    );
    let (catalog, index) = load_from_disk()?;
    let mut app = App::new(catalog, index, Box::new(load_from_disk));
    let mut watch = watch::FileWatch::new().ok();
    if let Some(watch) = &mut watch {
        app.catalog.warnings.extend(watch.sync(&app.catalog));
    } else {
        app.catalog
            .warnings
            .push("Live reload unavailable; use F5 to refresh".into());
    }
    ratatui::run(|terminal| app.event_loop(terminal, &mut watch))
}

impl App {
    fn new(catalog: Catalog, index: Index, load: Loader) -> Self {
        let mut app = Self {
            load,
            catalog,
            index,
            screen: Screen::Projects,
            history: Vec::new(),
            selection: ListState::default().with_selected(Some(0)),
            tab: Tab::Summary,
            scroll: 0,
            max_scroll: 0,
            status: None,
            priority: None,
            query: String::new(),
            message: String::new(),
            modal: Modal::None,
            quit: false,
            external: None,
            clipboard: None,
            rows: Vec::new(),
            rows_error: None,
            detail: None,
            started: Instant::now(),
        };
        app.invalidate();
        app
    }

    fn event_loop(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        watch: &mut Option<watch::FileWatch>,
    ) -> Result<()> {
        let mut redraw = true;
        let mut last_tick = Instant::now();
        loop {
            if redraw {
                terminal.draw(|frame| self.draw(frame))?;
                redraw = false;
            }
            if let Some(watch) = watch {
                let result = watch.ready();
                let changed = matches!(result, Ok(true));
                redraw |= self.watch_result(result);
                if changed {
                    let warnings = watch.sync(&self.catalog);
                    self.catalog.warnings.extend(warnings);
                }
            }
            if self.is_animating(terminal.get_frame().area())
                && last_tick.elapsed() >= Duration::from_millis(250)
            {
                last_tick = Instant::now();
                redraw = true;
            }
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let event = event::read()?;
            if matches!(event, Event::Resize(_, _)) {
                redraw = true;
            }
            if let Event::Key(key) = event {
                match self.handle_key(key) {
                    KeyOutcome::Ignored => continue,
                    KeyOutcome::Quit => break,
                    KeyOutcome::Handled => redraw = true,
                }
                if let Some(external) = self.external.take() {
                    ratatui::try_restore()?;
                    let result = external.run();
                    *terminal = ratatui::try_init()?;
                    self.finish_external(result);
                }
                if self.quit {
                    break;
                }
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> KeyOutcome {
        if key.kind == KeyEventKind::Release {
            return KeyOutcome::Ignored;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return KeyOutcome::Quit;
        }
        if let Err(error) = self.key(key.code, key.modifiers) {
            self.message = format!("Error: {error:#}");
        }
        KeyOutcome::Handled
    }

    /// Applies a file watcher poll; returns whether the screen needs redrawing.
    fn watch_result(&mut self, result: Result<bool>) -> bool {
        match result {
            Ok(true) => {
                if let Err(error) = self.refresh() {
                    self.message = format!("Refresh failed: {error:#}");
                }
                true
            }
            Ok(false) => false,
            Err(error) => {
                self.message = format!("Live reload error: {error}. Use F5 to refresh.");
                true
            }
        }
    }

    /// Reports the outcome of an external program and reloads what it may have edited.
    fn finish_external(&mut self, result: Result<()>) {
        self.message = match (result, self.refresh()) {
            (Err(error), _) => format!("Error: {error:#}"),
            (_, Err(error)) => format!("Refresh failed: {error:#}"),
            (Ok(()), Ok(())) => "Returned to fio".into(),
        };
    }

    /// Navigates to `screen`, remembering the current one.
    fn go(&mut self, screen: Screen) {
        let previous = std::mem::replace(&mut self.screen, screen);
        self.history.push(previous);
        self.reset_view();
    }

    fn back(&mut self) {
        if let Some(screen) = self.history.pop() {
            self.screen = screen;
            self.reset_view();
        }
    }

    fn reset_view(&mut self) {
        self.selection = ListState::default().with_selected(Some(0));
        self.scroll = 0;
        self.invalidate();
    }

    /// Recomputes cached rows and clears the cached detail text.
    fn invalidate(&mut self) {
        self.detail = None;
        match self.build_rows() {
            Ok(rows) => {
                self.rows = rows;
                self.rows_error = None;
            }
            Err(error) => {
                self.rows = Vec::new();
                self.rows_error = Some(format!("{error:#}"));
            }
        }
    }

    fn is_detail(&self) -> bool {
        matches!(self.screen, Screen::Detail(_))
    }

    fn show_help(&mut self) {
        self.modal = Modal::Help;
        self.scroll = 0;
    }

    fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.selection.selected().unwrap_or(0))
    }

    fn is_valid(&self, screen: &Screen) -> bool {
        match screen {
            Screen::Detail(key) => self.catalog.find(key).is_some(),
            Screen::Demands(Some(workspace)) => self
                .catalog
                .projects
                .iter()
                .any(|project| &project.workspace == workspace),
            _ => true,
        }
    }

    fn refresh(&mut self) -> Result<()> {
        let (catalog, index) = (self.load)()?;
        let selected = self.selected_row().map(|row| row.target.clone());
        self.catalog = catalog;
        self.index = index;
        if !self.is_valid(&self.screen) {
            self.screen = Screen::Projects;
        }
        let history = std::mem::take(&mut self.history);
        self.history = history
            .into_iter()
            .filter(|screen| self.is_valid(screen))
            .collect();
        self.invalidate();
        if let Some(selected) = selected {
            let position = self
                .rows
                .iter()
                .position(|row| row.target == selected)
                .unwrap_or(0);
            self.selection.select(Some(position));
        }
        self.message = "Refreshed from Markdown".into();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ratatui::{Terminal, backend::TestBackend};
    use rusqlite::Connection;

    use super::*;
    use crate::{
        model::ItemKind,
        store::{
            catalog::DemandKey,
            demands,
            testing::{TempWorkspace, assert_err, id},
        },
    };

    const NONE: KeyModifiers = KeyModifiers::NONE;

    fn app_for(temp: &TempWorkspace) -> App {
        let project = temp.project();
        let load: Loader = Box::new(move || {
            let catalog = Catalog::from_projects(vec![project.clone()]);
            let index = Index::build(&catalog, Connection::open_in_memory()?)?;
            Ok((catalog, index))
        });
        let (catalog, index) = load().unwrap();
        App::new(catalog, index, load)
    }

    fn press(app: &mut App, key: KeyCode) {
        app.key(key, NONE).unwrap();
    }

    fn press_err(app: &mut App, key: KeyCode) -> anyhow::Result<()> {
        app.key(key, NONE)
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    /// Opens a prompt with `opener`, types `text` and presses Enter.
    fn submit(app: &mut App, opener: char, text: &str) {
        press(app, KeyCode::Char(opener));
        type_text(app, text);
        press(app, KeyCode::Enter);
    }

    fn screen_text(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(usize::from(width))
            .map(|row| {
                row.iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn key_of(temp: &TempWorkspace, name: &str) -> DemandKey {
        DemandKey {
            workspace: temp.workspace.root().to_path_buf(),
            id: id(name),
        }
    }

    fn open_detail(app: &mut App, temp: &TempWorkspace, name: &str) {
        app.go(Screen::Detail(key_of(temp, name)));
    }

    fn at(app: &App, screen: &Screen) -> bool {
        &app.screen == screen
    }

    fn row_targets(app: &App) -> Vec<String> {
        app.rows
            .iter()
            .map(|row| match &row.target {
                Screen::Detail(key) => key.id.to_string(),
                _ => "-".into(),
            })
            .collect()
    }

    #[test]
    fn starts_on_the_projects_screen_with_one_row() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let app = app_for(&temp);
        assert!(at(&app, &Screen::Projects));
        assert_eq!(app.rows.len(), 1);
    }

    #[test]
    fn digits_switch_between_global_views() {
        let temp = TempWorkspace::with_demands(&["a", "b"]);
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char('2'));
        assert!(at(&app, &Screen::Demands(None)));
        assert_eq!(row_targets(&app), ["a", "b"]);
        for (digit, screen) in [
            ('3', Screen::Active),
            ('4', Screen::Questions),
            ('5', Screen::Debts),
            ('1', Screen::Projects),
        ] {
            press(&mut app, KeyCode::Char(digit));
            assert!(at(&app, &screen));
        }
    }

    #[test]
    fn enter_opens_a_project_and_escape_returns() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Enter);
        let root = temp.workspace.root().to_path_buf();
        assert!(at(&app, &Screen::Demands(Some(root))));
        assert_eq!(row_targets(&app), ["a"]);
        press(&mut app, KeyCode::Esc);
        assert!(at(&app, &Screen::Projects));
    }

    #[test]
    fn escape_on_the_first_screen_does_nothing() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Esc);
        assert!(at(&app, &Screen::Projects));
    }

    #[test]
    fn enter_on_a_demand_opens_its_detail() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Enter);
        assert!(at(&app, &Screen::Detail(key_of(&temp, "a"))));
        press(&mut app, KeyCode::Esc);
        assert!(at(&app, &Screen::Demands(None)));
    }

    #[test]
    fn tab_keys_cycle_through_the_detail_tabs() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut app = app_for(&temp);
        open_detail(&mut app, &temp, "a");
        press(&mut app, KeyCode::Tab);
        assert!(app.tab == Tab::Sessions);
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::Left);
        assert!(app.tab == Tab::Docs);
        press(&mut app, KeyCode::Right);
        assert!(app.tab == Tab::Summary);
    }

    #[test]
    fn digits_inside_a_detail_select_tabs_instead_of_views() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut app = app_for(&temp);
        open_detail(&mut app, &temp, "a");
        press(&mut app, KeyCode::Char('5'));
        assert!(app.tab == Tab::Debts);
        assert!(app.is_detail());
        press(&mut app, KeyCode::Char('6'));
        assert!(app.tab == Tab::Docs);
    }

    #[test]
    fn status_and_priority_filters_narrow_the_list_and_x_clears_them() {
        let temp = TempWorkspace::with_demands(&["a", "b"]);
        demands::load_for_update(&temp.workspace, &id("b"))
            .unwrap()
            .set_status(Status::Active)
            .unwrap();
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.status, Some(Status::Backlog));
        assert_eq!(row_targets(&app), ["a"]);
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(row_targets(&app), ["b"]);
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.priority, Some(Priority::High));
        assert!(app.rows.is_empty());
        press(&mut app, KeyCode::Char('x'));
        assert!(app.status.is_none() && app.priority.is_none());
        assert_eq!(app.rows.len(), 2);
    }

    #[test]
    fn active_view_lists_only_active_and_blocked_demands() {
        let temp = TempWorkspace::with_demands(&["a", "b", "c"]);
        for (name, status) in [("a", Status::Active), ("b", Status::Blocked)] {
            demands::load_for_update(&temp.workspace, &id(name))
                .unwrap()
                .set_status(status)
                .unwrap();
        }
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char('3'));
        assert_eq!(row_targets(&app), ["a", "b"]);
    }

    #[test]
    fn selection_movement_is_clamped_to_the_list() {
        let temp = TempWorkspace::with_demands(&["a", "b", "c"]);
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char('2'));
        let selected = |app: &App| app.selection.selected();
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(selected(&app), Some(1));
        press(&mut app, KeyCode::End);
        assert_eq!(selected(&app), Some(2));
        press(&mut app, KeyCode::Down);
        assert_eq!(selected(&app), Some(2));
        press(&mut app, KeyCode::Home);
        press(&mut app, KeyCode::Up);
        assert_eq!(selected(&app), Some(0));
        press(&mut app, KeyCode::PageDown);
        assert_eq!(selected(&app), Some(2));
        press(&mut app, KeyCode::PageUp);
        assert_eq!(selected(&app), Some(0));
    }

    #[test]
    fn detail_scrolling_is_bounded_by_the_rendered_height() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut app = app_for(&temp);
        open_detail(&mut app, &temp, "a");
        screen_text(&mut app, 60, 12);
        assert!(app.max_scroll > 0);
        press(&mut app, KeyCode::End);
        assert_eq!(app.scroll, app.max_scroll);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.scroll, app.max_scroll);
        press(&mut app, KeyCode::Home);
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn commands_navigate_and_report_unknown_ones() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut app = app_for(&temp);
        submit(&mut app, ':', "demands");
        assert!(at(&app, &Screen::Demands(None)));
        submit(&mut app, ':', "active");
        assert!(at(&app, &Screen::Active));
        submit(&mut app, ':', "bogus");
        assert!(app.message.contains("Unknown command"));
        submit(&mut app, ':', "quit");
        assert!(app.quit);
    }

    #[test]
    fn q_quits() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
    }

    #[test]
    fn prompt_editing_supports_backspace_ctrl_u_and_escape() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char(':'));
        type_text(&mut app, "ab");
        press(&mut app, KeyCode::Backspace);
        assert!(matches!(&app.modal, Modal::Input { text, .. } if text == "a"));
        app.key(KeyCode::Char('u'), KeyModifiers::CONTROL).unwrap();
        assert!(matches!(&app.modal, Modal::Input { text, .. } if text.is_empty()));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.modal, Modal::None));
    }

    #[test]
    fn help_opens_with_question_mark_and_closes_with_escape() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char('?'));
        assert!(matches!(app.modal, Modal::Help));
        assert!(screen_text(&mut app, 100, 40).contains("Keyboard"));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.modal, Modal::None));
    }

    #[test]
    fn search_prompt_lists_matching_demands() {
        let temp = TempWorkspace::with_demands(&["a", "b"]);
        let mut locked = demands::load_for_update(&temp.workspace, &id("a")).unwrap();
        locked
            .add_item(ItemKind::Decision, "adopt zebrafish tokens")
            .unwrap();
        drop(locked);
        let mut app = app_for(&temp);
        submit(&mut app, '/', "zebra");
        assert!(at(&app, &Screen::Search));
        assert_eq!(app.query, "zebra");
        assert_eq!(row_targets(&app), ["a"]);
        assert!(screen_text(&mut app, 100, 20).contains("search: zebra"));
    }

    #[test]
    fn refresh_picks_up_external_changes_and_keeps_the_selected_demand() {
        let temp = TempWorkspace::with_demands(&["b", "c"]);
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(row_targets(&app), ["b", "c"]);
        temp.create_demand("a");
        press(&mut app, KeyCode::F(5));
        assert_eq!(row_targets(&app), ["a", "b", "c"]);
        assert_eq!(app.selection.selected(), Some(2));
        assert!(app.message.contains("Refreshed"));
    }

    #[test]
    fn refresh_leaves_a_detail_whose_demand_disappeared() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut app = app_for(&temp);
        open_detail(&mut app, &temp, "a");
        fs::remove_dir_all(temp.workspace.demand_dir(&id("a"))).unwrap();
        press(&mut app, KeyCode::F(5));
        assert!(at(&app, &Screen::Projects));
    }

    #[test]
    fn new_demand_flow_creates_and_opens_the_demand() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        submit(&mut app, 'n', "fresh");
        assert!(matches!(
            &app.modal,
            Modal::Input {
                prompt: Prompt::NewTitle(..),
                ..
            }
        ));
        type_text(&mut app, "My title");
        press(&mut app, KeyCode::Enter);
        let loaded = demands::load(&temp.workspace, &id("fresh")).unwrap();
        assert_eq!(loaded.document.metadata.title, "My title");
        assert!(at(&app, &Screen::Detail(key_of(&temp, "fresh"))));
        assert!(app.message.contains("created"));
    }

    #[test]
    fn an_invalid_new_id_keeps_the_draft_for_correction() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char('n'));
        type_text(&mut app, "bad id");
        let result = press_err(&mut app, KeyCode::Enter);
        assert_err(result, "Invalid ID");
        assert!(matches!(&app.modal, Modal::Input { text, .. } if text == "bad id"));
    }

    #[test]
    fn escape_cancels_a_new_demand_prompt() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char('n'));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.modal, Modal::None));
    }

    #[test]
    fn new_demand_needs_a_selected_project_or_demand() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char('2'));
        assert_err(
            press_err(&mut app, KeyCode::Char('n')),
            "Select a demand first",
        );
    }

    #[test]
    fn new_sub_demand_is_allowed_only_below_a_top_level_demand() {
        let temp = TempWorkspace::with_demands(&["p"]);
        temp.create_child("c", "p");
        let mut app = app_for(&temp);

        open_detail(&mut app, &temp, "c");
        assert_err(press_err(&mut app, KeyCode::Char('N')), "only one level");

        app.back();
        open_detail(&mut app, &temp, "p");
        press(&mut app, KeyCode::Char('N'));
        type_text(&mut app, "kid");
        press(&mut app, KeyCode::Enter);
        type_text(&mut app, "Kid title");
        press(&mut app, KeyCode::Enter);
        let kid = demands::load(&temp.workspace, &id("kid")).unwrap();
        assert_eq!(kid.document.metadata.parent, Some(id("p")));
    }

    #[test]
    fn status_prompt_updates_the_demand_on_disk() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut app = app_for(&temp);
        open_detail(&mut app, &temp, "a");
        submit(&mut app, 's', "done");
        let loaded = demands::load(&temp.workspace, &id("a")).unwrap();
        assert_eq!(loaded.document.metadata.status, Status::Done);
        assert!(app.message.contains("Status updated"));
    }

    #[test]
    fn an_unknown_status_keeps_the_draft() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut app = app_for(&temp);
        open_detail(&mut app, &temp, "a");
        press(&mut app, KeyCode::Char('s'));
        type_text(&mut app, "nonsense");
        assert_err(press_err(&mut app, KeyCode::Enter), "nonsense");
        assert!(matches!(&app.modal, Modal::Input { text, .. } if text == "nonsense"));
    }

    #[test]
    fn actions_need_a_demand_target() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        for key in ['s', 'u', 'c', 'e', 'r', 'N'] {
            assert_err(
                press_err(&mut app, KeyCode::Char(key)),
                "Select a demand first",
            );
        }
    }

    #[test]
    fn use_selected_sets_the_workspace_default_and_marks_it() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut app = app_for(&temp);
        open_detail(&mut app, &temp, "a");
        press(&mut app, KeyCode::Char('u'));
        let active = crate::store::state::active(&temp.workspace).unwrap();
        assert_eq!(active, id("a"));
        assert_eq!(
            app.catalog.active.get(temp.workspace.root()),
            Some(&id("a"))
        );
        app.back();
        press(&mut app, KeyCode::Char('2'));
        assert!(screen_text(&mut app, 100, 20).contains(" *"));
    }

    #[test]
    fn resume_lists_only_sessions_with_safe_native_ids() {
        let temp = TempWorkspace::with_demands(&["a"]);
        temp.start_session("a", Some("native-1"), None);
        temp.start_session("a", Some("-flag"), None);
        temp.start_session("a", None, None);
        let mut app = app_for(&temp);
        open_detail(&mut app, &temp, "a");
        press(&mut app, KeyCode::Char('r'));
        match &app.modal {
            Modal::Picker(picker) => {
                assert_eq!(picker.choices.len(), 1);
                assert!(picker.choices[0].0.contains("native-1"));
            }
            _ => panic!("expected a picker"),
        }
        assert!(screen_text(&mut app, 100, 20).contains("Resume agent session"));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.modal, Modal::None) && app.external.is_none());
    }

    #[test]
    fn confirming_the_picker_queues_the_external_command() {
        let temp = TempWorkspace::with_demands(&["a"]);
        temp.start_session("a", Some("native-1"), None);
        let mut app = app_for(&temp);
        open_detail(&mut app, &temp, "a");
        press(&mut app, KeyCode::Char('r'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert!(app.external.is_some());
        assert!(matches!(app.modal, Modal::None));
    }

    #[test]
    fn resume_without_native_ids_explains_what_is_missing() {
        let temp = TempWorkspace::with_demands(&["a"]);
        temp.start_session("a", None, None);
        let mut app = app_for(&temp);
        open_detail(&mut app, &temp, "a");
        assert_err(
            press_err(&mut app, KeyCode::Char('r')),
            "No resumable sessions",
        );
    }

    #[test]
    fn edit_in_an_empty_docs_tab_reports_no_files() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut app = app_for(&temp);
        open_detail(&mut app, &temp, "a");
        press(&mut app, KeyCode::Char('6'));
        assert_err(press_err(&mut app, KeyCode::Char('e')), "No files recorded");
    }

    #[test]
    fn question_and_debt_views_list_one_row_per_open_item() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut locked = demands::load_for_update(&temp.workspace, &id("a")).unwrap();
        locked.add_item(ItemKind::Question, "open one").unwrap();
        locked.add_item(ItemKind::Question, "closed one").unwrap();
        locked
            .complete_item(ItemKind::Question, "Q2", Some("yes"))
            .unwrap();
        locked.add_item(ItemKind::Debt, "pay later").unwrap();
        drop(locked);
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char('4'));
        assert_eq!(app.rows.len(), 1);
        assert!(app.rows[0].tab == Tab::Questions);
        let text = screen_text(&mut app, 120, 20);
        assert!(text.contains("open one") && !text.contains("closed one"));
        press(&mut app, KeyCode::Char('5'));
        assert!(app.rows[0].tab == Tab::Debts);
        press(&mut app, KeyCode::Enter);
        assert!(app.tab == Tab::Debts && app.is_detail());
    }

    #[test]
    fn sub_demands_are_indented_in_lists_and_summarized_in_the_parent() {
        let temp = TempWorkspace::with_demands(&["p"]);
        temp.create_child("c", "p");
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char('2'));
        let text = screen_text(&mut app, 120, 20);
        assert!(text.contains("test/p") && text.contains("└ p/c"));

        open_detail(&mut app, &temp, "p");
        let detail = screen_text(&mut app, 120, 30);
        assert!(detail.contains("Sub-demands (0/1 done)"));
        app.back();
        open_detail(&mut app, &temp, "c");
        assert!(screen_text(&mut app, 120, 30).contains("Parent: p"));
    }

    #[test]
    fn detail_tabs_render_their_section() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut locked = demands::load_for_update(&temp.workspace, &id("a")).unwrap();
        locked.add_item(ItemKind::Decision, "use sqlite").unwrap();
        drop(locked);
        temp.start_session("a", Some("native-9"), None);
        let mut app = app_for(&temp);
        open_detail(&mut app, &temp, "a");

        assert!(screen_text(&mut app, 120, 30).contains("Objective"));
        press(&mut app, KeyCode::Char('3'));
        assert!(screen_text(&mut app, 120, 30).contains("use sqlite"));
        press(&mut app, KeyCode::Char('2'));
        assert!(screen_text(&mut app, 120, 40).contains("Native session: native-9"));
        press(&mut app, KeyCode::Char('4'));
        assert!(screen_text(&mut app, 120, 30).contains("Nothing recorded yet."));
    }

    #[test]
    fn empty_states_explain_themselves() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char('4'));
        assert!(screen_text(&mut app, 100, 30).contains("No items match this view."));

        let no_projects = Catalog::from_projects(Vec::new());
        let index = Index::build(&no_projects, Connection::open_in_memory().unwrap()).unwrap();
        let load: Loader = Box::new(|| unreachable!());
        let mut empty = App::new(no_projects, index, load);
        assert!(screen_text(&mut empty, 100, 30).contains("No projects registered"));
    }

    #[test]
    fn header_and_footer_show_counts_filters_and_messages() {
        let temp = TempWorkspace::with_demands(&["a", "b"]);
        let mut app = app_for(&temp);
        let text = screen_text(&mut app, 120, 20);
        assert!(text.contains("1 projects · 2 demands"));
        assert!(text.contains("status all") && text.contains("priority all"));
        press(&mut app, KeyCode::Char('f'));
        assert!(screen_text(&mut app, 120, 20).contains("status backlog"));
        press(&mut app, KeyCode::Char('/'));
        assert!(screen_text(&mut app, 120, 20).contains('/'));
    }

    fn key_event(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> KeyEvent {
        KeyEvent::new_with_kind(code, modifiers, kind)
    }

    #[test]
    fn key_release_events_are_ignored() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        let release = key_event(KeyCode::Char('q'), NONE, KeyEventKind::Release);
        assert!(matches!(app.handle_key(release), KeyOutcome::Ignored));
        assert!(!app.quit);
    }

    #[test]
    fn ctrl_c_quits_even_inside_a_prompt() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        press(&mut app, KeyCode::Char(':'));
        let ctrl_c = key_event(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        );
        assert!(matches!(app.handle_key(ctrl_c), KeyOutcome::Quit));
    }

    #[test]
    fn handled_keys_report_errors_in_the_message_bar() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        let failing = key_event(KeyCode::Char('s'), NONE, KeyEventKind::Press);
        assert!(matches!(app.handle_key(failing), KeyOutcome::Handled));
        assert_eq!(app.message, "Error: Select a demand first");
        let quit = key_event(KeyCode::Char('q'), NONE, KeyEventKind::Press);
        assert!(matches!(app.handle_key(quit), KeyOutcome::Handled));
        assert!(app.quit);
    }

    #[test]
    fn a_quiet_watcher_does_not_redraw() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        assert!(!app.watch_result(Ok(false)));
        assert_eq!(app.message, "");
    }

    #[test]
    fn a_watcher_change_reloads_the_catalog_and_redraws() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        temp.create_demand("late");
        assert!(app.catalog.demands.is_empty());
        assert!(app.watch_result(Ok(true)));
        assert_eq!(app.catalog.demands.len(), 1);
        assert!(app.message.contains("Refreshed"));
    }

    #[test]
    fn a_watcher_error_is_shown_with_the_manual_refresh_hint() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        assert!(app.watch_result(Err(anyhow::anyhow!("inotify limit"))));
        assert_eq!(
            app.message,
            "Live reload error: inotify limit. Use F5 to refresh."
        );
    }

    #[test]
    fn finishing_an_external_program_reports_success_or_failure() {
        let temp = TempWorkspace::new();
        let mut app = app_for(&temp);
        app.finish_external(Ok(()));
        assert_eq!(app.message, "Returned to fio");
        app.finish_external(Err(anyhow::anyhow!("editor exited with 1")));
        assert_eq!(app.message, "Error: editor exited with 1");
    }

    #[test]
    fn finishing_an_external_program_reports_a_failed_reload() {
        let calls = std::cell::Cell::new(0);
        let load: Loader = Box::new(move || {
            calls.set(calls.get() + 1);
            anyhow::ensure!(calls.get() == 1, "registry unreadable");
            let catalog = Catalog::from_projects(Vec::new());
            let index = Index::build(&catalog, Connection::open_in_memory()?)?;
            Ok((catalog, index))
        });
        let (catalog, index) = load().unwrap();
        let mut app = App::new(catalog, index, load);
        app.finish_external(Ok(()));
        assert_eq!(app.message, "Refresh failed: registry unreadable");
    }

    #[test]
    fn tiny_terminals_do_not_panic() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut app = app_for(&temp);
        for (width, height) in [(1, 1), (10, 5), (30, 8)] {
            screen_text(&mut app, width, height);
        }
        open_detail(&mut app, &temp, "a");
        screen_text(&mut app, 10, 5);
    }
}
