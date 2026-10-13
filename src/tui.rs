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
