use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::Modifier,
    text::{Line, Span, Text},
    widgets::{Block, BorderType, List, ListItem, Padding, Paragraph, Wrap},
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
N: new sub-demand inside the selected demand
s: choose status   P: choose priority   u: use as active demand
a: answer / resolve the selected item, or record a decision, question or debt
v: preview the full context
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
            Constraint::Length(5),
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
        matches!(self.modal, Modal::None | Modal::Input { .. })
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
        if navigation.iter().map(Span::width).sum::<usize>()
            > usize::from(area.width.saturating_sub(4))
        {
            let title = if self.is_detail() {
                self.tab.title().to_owned()
            } else if self.screen == Screen::Search {
                format!("Search: {}", self.query)
            } else {
                GLOBAL_VIEWS[current.unwrap_or(0)].to_owned()
            };
            navigation = vec![
                Span::styled(title, theme::selected()),
                Span::styled(" · ? help", theme::muted()),
            ];
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
        if self.draw_modal(frame, area) {
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
        let area = if area.width >= 120 && area.height >= 12 && self.target().is_ok() {
            let areas =
                Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                    .split(area);
            let text = self
                .preview_text()
                .unwrap_or_else(|error| format!("Preview unavailable: {error:#}"));
            frame.render_widget(
                Paragraph::new(theme::styled(&text))
                    .wrap(Wrap { trim: false })
                    .block(
                        panel().title(Span::styled(" Preview · v full context ", theme::heading())),
                    ),
                areas[1],
            );
            areas[0]
        } else {
            area
        };
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

    fn draw_modal(&mut self, frame: &mut Frame, area: Rect) -> bool {
        if let Modal::Picker(picker) = &mut self.modal {
            let items: Vec<_> = picker
                .choices
                .iter()
                .map(|(label, _)| ListItem::new(wrap_text(label, area.width.saturating_sub(6))))
                .collect();
            picker.state.select(Some(picker.selected));
            let title = Line::from(vec![
                Span::styled(format!(" {} ", picker.title), theme::heading()),
                Span::styled("Enter select · Esc cancel ", theme::muted()),
            ]);
            frame.render_stateful_widget(
                List::new(items)
                    .block(panel().title(title))
                    .highlight_symbol("▶ ")
                    .highlight_style(theme::selected()),
                area,
                &mut picker.state,
            );
            return true;
        }
        if let Modal::Preview(preview) = &mut self.modal {
            let block = panel().title(Span::styled(
                format!(" {} ", preview.title),
                theme::heading(),
            ));
            let inner = block.inner(area);
            let paragraph = Paragraph::new(theme::styled(&preview.text)).wrap(Wrap { trim: false });
            preview.max_scroll = u16::try_from(
                paragraph
                    .line_count(inner.width)
                    .saturating_sub(inner.height as usize),
            )
            .unwrap_or(u16::MAX);
            preview.scroll = preview.scroll.min(preview.max_scroll);
            frame.render_widget(block, area);
            frame.render_widget(paragraph.scroll((preview.scroll, 0)), inner);
            return true;
        }
        false
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
                input_line(&prompt.label(), text, area.width.saturating_sub(4)),
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
            let shortcuts: &[(&str, &str)] = match self.modal {
                Modal::Preview(_) | Modal::Help => &[("j/k", "scroll"), ("Esc", "back")],
                Modal::Picker(_) => &[("j/k", "choose"), ("Enter", "select"), ("Esc", "cancel")],
                _ if self.target().is_ok() => &[
                    ("a", "actions"),
                    ("v", "context"),
                    ("s", "status"),
                    ("P", "priority"),
                    ("r", "resume"),
                    ("?", "help"),
                ],
                _ => &[("n", "new"), ("/", "search"), ("?", "help")],
            };
            for &(key, what) in shortcuts {
                let next = theme::hint(key, what);
                let width: usize = hints.iter().map(Span::width).sum();
                if width + next.iter().map(Span::width).sum::<usize>()
                    > usize::from(area.width.saturating_sub(4))
                {
                    break;
                }
                hints.extend(next);
            }
            vec![
                Line::from(filters),
                Line::from(hints),
                Line::from(Span::styled(
                    self.message.clone(),
                    theme::message(&self.message).add_modifier(Modifier::ITALIC),
                )),
            ]
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

/// Wrap picker labels so long questions and session objectives remain readable.
fn wrap_text(text: &str, width: u16) -> Text<'static> {
    let width = usize::from(width.max(1));
    let mut lines = Vec::new();
    for source in text.lines() {
        let mut line = String::new();
        let mut used = 0;
        for ch in source.chars() {
            let size = Span::raw(ch.to_string()).width();
            if used + size > width && !line.is_empty() {
                lines.push(Line::raw(std::mem::take(&mut line)));
                used = 0;
            }
            line.push(ch);
            used += size;
        }
        lines.push(Line::raw(line));
    }
    Text::from(lines)
}

fn input_line(label: &str, text: &str, width: u16) -> Line<'static> {
    let width = usize::from(width);
    let label = if Span::raw(label).width() + 2 < width {
        label
    } else {
        "> "
    };
    let available = width.saturating_sub(Span::raw(label).width() + 1);
    let mut used = 0;
    let tail: String = text
        .chars()
        .rev()
        .take_while(|ch| {
            used += Span::raw(ch.to_string()).width();
            used <= available
        })
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    Line::from(vec![
        Span::styled(label.to_owned(), theme::heading()),
        Span::raw(tail),
        Span::styled("▏", theme::accent()),
    ])
}
