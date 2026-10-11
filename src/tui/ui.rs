use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

use super::{
    App, Modal,
    screen::{Screen, Tab},
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

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let title = match self.screen {
            Screen::Projects => "Projects",
            Screen::Demands(_) => "Demands",
            Screen::Active => "Active / blocked",
            Screen::Questions => "Open questions",
            Screen::Debts => "Open debts",
            Screen::Search => "Search",
            Screen::Detail(_) => "Demand",
        };
        let navigation = if self.is_detail() {
            Tab::ALL
                .iter()
                .enumerate()
                .map(|(i, tab)| {
                    let marker = if *tab == self.tab { " *" } else { "" };
                    format!("{} {}{marker}", i + 1, tab.title())
                })
                .collect::<Vec<_>>()
                .join("  ")
        } else {
            "1 Projects  2 All demands  3 Active  4 Questions  5 Debts".into()
        };
        let query = if self.screen == Screen::Search {
            format!(": {} (up to 200 matches)", self.query)
        } else {
            String::new()
        };
        frame.render_widget(
            Paragraph::new(navigation)
                .style(Style::default().fg(Color::Cyan))
                .block(Block::default().borders(Borders::ALL).title(format!(
                    " fio / {title}{query} · {} projects · {} demands ",
                    self.catalog.projects.len(),
                    self.catalog.demands.len()
                ))),
            area,
        );
    }

    fn draw_body(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default().borders(Borders::ALL);
        match &self.modal {
            Modal::Picker(picker) => {
                let items: Vec<_> = picker
                    .choices
                    .iter()
                    .map(|(label, _)| ListItem::new(label.clone()))
                    .collect();
                let mut state = ListState::default().with_selected(Some(picker.selected));
                frame.render_stateful_widget(
                    List::new(items)
                        .block(
                            block.title(format!(" {} · Enter launch · Esc cancel ", picker.title)),
                        )
                        .highlight_symbol("› ")
                        .highlight_style(Style::default().fg(Color::Cyan)),
                    area,
                    &mut state,
                );
                return;
            }
            Modal::Help => {
                let warnings = if self.catalog.warnings.is_empty() {
                    "None".into()
                } else {
                    self.catalog.warnings.join("\n")
                };
                self.render_text(
                    frame,
                    area,
                    format!("{HELP}{warnings}"),
                    block.title(" Help "),
                );
                return;
            }
            _ => (),
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
            self.render_text(frame, area, text, block);
            return;
        }
        if let Some(error) = &self.rows_error {
            frame.render_widget(
                Paragraph::new(format!("Could not load view: {error}")).block(block),
                area,
            );
        } else if self.rows.is_empty() {
            let text = if self.catalog.projects.is_empty() {
                "No projects registered. Run fio init in a workspace."
            } else {
                "No items match this view."
            };
            frame.render_widget(Paragraph::new(text).block(block), area);
        } else {
            let selected = self
                .selection
                .selected()
                .unwrap_or(0)
                .min(self.rows.len() - 1);
            self.selection.select(Some(selected));
            let items: Vec<_> = self
                .rows
                .iter()
                .map(|row| ListItem::new(row.label.as_str()))
                .collect();
            frame.render_stateful_widget(
                List::new(items)
                    .block(block)
                    .highlight_symbol("› ")
                    .highlight_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray)),
                area,
                &mut self.selection,
            );
        }
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let footer = match &self.modal {
            Modal::Input { prompt, text } => {
                format!("{}{text}▏\n{}", prompt.label(), self.message)
            }
            _ => format!(
                "Status filter: {} · Priority filter: {} · {} warnings\nn new · s status · u active · c copy · e edit · r resume · ? help  {}",
                self.status.map_or_else(|| "all".into(), |s| s.to_string()),
                self.priority
                    .map_or_else(|| "all".into(), |p| p.to_string()),
                self.catalog.warnings.len(),
                self.message
            ),
        };
        frame.render_widget(
            Paragraph::new(footer).block(Block::default().borders(Borders::ALL)),
            area,
        );
    }

    fn render_text(&mut self, frame: &mut Frame, area: Rect, text: String, block: Block<'_>) {
        let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
        let lines = paragraph.line_count(area.width.saturating_sub(2));
        let visible = area.height.saturating_sub(2) as usize;
        self.max_scroll = u16::try_from(lines.saturating_sub(visible)).unwrap_or(u16::MAX);
        self.scroll = self.scroll.min(self.max_scroll);
        frame.render_widget(paragraph.block(block).scroll((self.scroll, 0)), area);
    }
}
