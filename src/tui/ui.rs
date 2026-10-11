use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, List, ListItem, ListState, Padding, Paragraph, Wrap},
};

use super::{
    App, Modal, mascot,
    screen::{Screen, Tab},
    theme,
};

const HELP: &str = "Keyboard

Enter: open selected item
Esc: back / cancel input
j/k or arrows: move / scroll
PageUp/PageDown, Home/End: navigate
1–5: global views (outside demand details)
1–6, Tab/Shift+Tab, left/right: demand tabs
f: cycle status filter   p: cycle priority filter   x: clear filters
/: search across all projects   F5: refresh from Markdown
: commands   ?: help   q / Ctrl+C: quit

Commands
projects, demands, active, questions, debts, search <text>, refresh, help, quit

Search matches word prefixes in demand and session Markdown.
Linked document contents and agent transcripts are not indexed.

Actions
n: new demand (select its project first)
s: set selected demand status   u: use as active demand
c: copy context   e: open file in VISUAL / EDITOR
r: choose an agent session to resume
Sessions / Docs tabs: e opens a file selector.
Files reload automatically; F5 refreshes manually.

Warnings
";

const GLOBAL_VIEWS: [&str; 5] = ["Projects", "All demands", "Active", "Questions", "Debts"];

fn panel() -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::border())
        .padding(Padding::horizontal(1))
}

impl App {
    pub(super) fn draw(&mut self, frame: &mut Frame) {
        let areas = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(4),
        ])
        .split(frame.area());
        self.draw_header(frame, areas[0]);
        self.draw_body(frame, areas[1]);
        self.draw_footer(frame, areas[2]);
    }

    /// Index of the mascot frame to show right now.
    fn mascot_frame(&self) -> usize {
        (self.started.elapsed().as_millis() / 250) as usize % mascot::FRAME_COUNT
    }

    /// Whether the visible screen shows the animated mascot.
    pub(super) fn is_animating(&self, area: Rect) -> bool {
        self.shows_empty_state() && mascot::fits(area, 4)
    }

    fn shows_empty_state(&self) -> bool {
        !matches!(self.modal, Modal::Picker(_))
            && !self.is_detail()
            && (self.rows_error.is_some() || self.rows.is_empty())
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let current = match self.screen {
            Screen::Projects => Some(0),
            Screen::Demands(_) => Some(1),
            Screen::Active => Some(2),
            Screen::Questions => Some(3),
            Screen::Debts => Some(4),
            Screen::Search | Screen::Detail(_) => None,
        };
        let items: Vec<(String, bool)> = if self.is_detail() {
            Tab::ALL
                .iter()
                .map(|tab| (tab.title().to_owned(), *tab == self.tab))
                .collect()
        } else {
            GLOBAL_VIEWS
                .iter()
                .enumerate()
                .map(|(i, name)| ((*name).to_owned(), current == Some(i)))
                .collect()
        };
        let mut navigation = Vec::new();
        for (index, (name, active)) in items.into_iter().enumerate() {
            let label = format!(" {} {} ", index + 1, name);
            let style = if active {
                theme::selected()
            } else {
                theme::muted()
            };
            navigation.push(Span::styled(label, style));
            navigation.push(Span::raw(" "));
        }
        if self.screen == Screen::Search {
            navigation.push(Span::styled(
                format!(" search: {} (up to 200 matches) ", self.query),
                theme::selected(),
            ));
        }
        let title = Line::from(vec![
            Span::styled(" fio ", theme::selected()),
            Span::styled(
                format!(
                    " {} projects · {} demands ",
                    self.catalog.projects.len(),
                    self.catalog.demands.len()
                ),
                theme::muted(),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(Line::from(navigation)).block(panel().title(title)),
            area,
        );
    }

    fn draw_body(&mut self, frame: &mut Frame, area: Rect) {
        let block = panel();
        if let Modal::Picker(picker) = &self.modal {
            let items: Vec<_> = picker
                .choices
                .iter()
                .map(|(label, _)| ListItem::new(label.clone()))
                .collect();
            let mut state = ListState::default().with_selected(Some(picker.selected));
            let title = Line::from(vec![
                Span::styled(format!(" {} ", picker.title), theme::heading()),
                Span::styled("Enter launch · Esc cancel ", theme::muted()),
            ]);
            frame.render_stateful_widget(
                List::new(items)
                    .block(block.title(title))
                    .highlight_symbol("▶ ")
                    .highlight_style(theme::selected()),
                area,
                &mut state,
            );
            return;
        }
        if matches!(self.modal, Modal::Help) {
            self.draw_help(frame, area, block);
            return;
        }
        if let Screen::Detail(key) = &self.screen {
            let text = if let Some(text) = &self.detail {
                text.clone()
            } else {
                let text = match self.catalog.find(key) {
                    Some(entry) => self
                        .detail_text(entry)
                        .unwrap_or_else(|error| format!("Could not read this section: {error:#}")),
                    None => "This demand is no longer available.".into(),
                };
                let text = if text.trim().is_empty() {
                    "Nothing recorded yet.".into()
                } else {
                    text
                };
                self.detail = Some(text.clone());
                text
            };
            self.render_text(frame, area, theme::styled(&text), block);
            return;
        }
        if self.shows_empty_state() {
            let message = match (&self.rows_error, self.catalog.projects.is_empty()) {
                (Some(error), _) => format!("Could not load view: {error}"),
                (None, true) => "No projects registered. Run fio init in a workspace.".to_owned(),
                (None, false) => "No items match this view.".to_owned(),
            };
            self.draw_empty(frame, area, block, &message);
            return;
        }
        let selected = self
            .selection
            .selected()
            .unwrap_or(0)
            .min(self.rows.len() - 1);
        self.selection.select(Some(selected));
        let items: Vec<_> = self
            .rows
            .iter()
            .map(|row| ListItem::new(row.label.clone()))
            .collect();
        frame.render_stateful_widget(
            List::new(items)
                .block(block)
                .highlight_symbol("▶ ")
                .highlight_style(theme::selected()),
            area,
            &mut self.selection,
        );
    }

    fn draw_empty(&self, frame: &mut Frame, area: Rect, block: Block<'_>, message: &str) {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        // Center the mascot and its message as one group.
        let group = mascot::ROWS + 2;
        let show_mascot = mascot::fits(inner, 4);
        let height = if show_mascot { group } else { 1 };
        let top = inner.y + inner.height.saturating_sub(height) / 2;
        let mut text_area = Rect::new(
            inner.x,
            top,
            inner.width,
            inner.bottom().saturating_sub(top),
        );
        if show_mascot {
            mascot::render(
                frame,
                Rect::new(inner.x, top, inner.width, mascot::ROWS),
                self.mascot_frame(),
            );
            text_area.y += mascot::ROWS + 1;
            text_area.height = text_area.height.saturating_sub(mascot::ROWS + 1);
        }
        frame.render_widget(
            Paragraph::new(Span::styled(message.to_owned(), theme::muted()))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            text_area,
        );
    }

    fn draw_help(&mut self, frame: &mut Frame, area: Rect, block: Block<'_>) {
        let warnings = if self.catalog.warnings.is_empty() {
            "None".into()
        } else {
            self.catalog.warnings.join("\n")
        };
        let text = theme::styled(&format!("{HELP}{warnings}"));
        let block = block.title(Span::styled(" Help ", theme::heading()));
        self.render_text(frame, area, text, block);
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let lines = if let Modal::Input { prompt, text } = &self.modal {
            vec![
                Line::from(vec![
                    Span::styled(prompt.label(), theme::heading()),
                    Span::raw(text.clone()),
                    Span::styled("▏", theme::accent()),
                ]),
                Line::from(Span::styled(
                    self.message.clone(),
                    theme::message(&self.message),
                )),
            ]
        } else {
            let filter = |name: &str, value: Option<Span<'static>>| {
                let value = value.unwrap_or_else(|| Span::styled("all", theme::muted()));
                vec![Span::styled(format!("{name} "), theme::muted()), value]
            };
            let mut filters = filter(
                "status",
                self.status
                    .map(|s| Span::styled(s.name(), theme::status(s))),
            );
            filters.push(Span::raw("   "));
            filters.extend(filter(
                "priority",
                self.priority
                    .map(|p| Span::styled(p.to_string(), theme::priority(p))),
            ));
            filters.push(Span::styled(
                format!("   {} warnings", self.catalog.warnings.len()),
                theme::muted(),
            ));
            let mut hints = Vec::new();
            for (key, what) in [
                ("n", "new"),
                ("s", "status"),
                ("u", "active"),
                ("c", "copy"),
                ("e", "edit"),
                ("r", "resume"),
                ("/", "search"),
                ("?", "help"),
            ] {
                hints.extend(theme::hint(key, what));
            }
            hints.push(Span::styled(
                self.message.clone(),
                theme::message(&self.message).add_modifier(Modifier::ITALIC),
            ));
            vec![Line::from(filters), Line::from(hints)]
        };
        frame.render_widget(Paragraph::new(lines).block(panel()), area);
    }

    fn render_text(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        text: Text<'static>,
        block: Block<'_>,
    ) {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.render_scrolled(frame, inner, text);
    }

    fn render_scrolled(&mut self, frame: &mut Frame, area: Rect, text: Text<'static>) {
        let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
        let lines = paragraph.line_count(area.width);
        self.max_scroll =
            u16::try_from(lines.saturating_sub(area.height as usize)).unwrap_or(u16::MAX);
        self.scroll = self.scroll.min(self.max_scroll);
        frame.render_widget(paragraph.scroll((self.scroll, 0)), area);
    }
}

/// The splash shown briefly when the TUI opens.
pub(super) fn draw_splash(frame: &mut Frame, index: usize) {
    let area = frame.area();
    let height = mascot::ROWS + 3;
    if area.height < height || area.width < mascot::COLUMNS {
        return;
    }
    let top = area.y + (area.height - height) / 2;
    mascot::render(
        frame,
        Rect::new(area.x, top, area.width, mascot::ROWS),
        index,
    );
    let title = Paragraph::new(vec![
        Line::from(Span::styled("fio", theme::heading())),
        Line::from(Span::styled(
            "demand-based context for agents",
            Style::new().add_modifier(Modifier::DIM),
        )),
    ])
    .alignment(Alignment::Center);
    frame.render_widget(
        title,
        Rect::new(area.x, top + mascot::ROWS + 1, area.width, 2),
    );
}
