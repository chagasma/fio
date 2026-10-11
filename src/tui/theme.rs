//! Semantic styles. Only ANSI colors are used, so the terminal theme decides the
//! actual shades; contrast comes from bold text and filled backgrounds.
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};

use crate::model::{Priority, Status};

pub fn accent() -> Style {
    Style::new().fg(Color::Cyan)
}

pub fn heading() -> Style {
    accent().add_modifier(Modifier::BOLD)
}

pub fn muted() -> Style {
    Style::new().fg(Color::Gray).add_modifier(Modifier::DIM)
}

pub fn border() -> Style {
    Style::new().fg(Color::Blue)
}

/// Selected list row and active tab.
pub fn selected() -> Style {
    Style::new()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
        .remove_modifier(Modifier::DIM)
}

pub fn status(status: Status) -> Style {
    let color = match status {
        Status::Backlog => Color::Gray,
        Status::Active => Color::Green,
        Status::Paused => Color::Yellow,
        Status::Blocked => Color::Red,
        Status::Done => Color::Blue,
        Status::Archived => Color::DarkGray,
    };
    Style::new().fg(color).add_modifier(Modifier::BOLD)
}

pub fn priority(priority: Priority) -> Style {
    match priority {
        Priority::High => Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        Priority::Medium => Style::new().fg(Color::Yellow),
        Priority::Low => Style::new().fg(Color::Gray),
    }
}

pub fn message(text: &str) -> Style {
    if text.starts_with("Error") || text.contains("failed") || text.starts_with("Could not") {
        Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::Green)
    }
}

/// A keyboard shortcut followed by what it does.
pub fn hint<'a>(key: &'a str, what: &'a str) -> [Span<'a>; 3] {
    [
        Span::styled(key, heading()),
        Span::styled(format!(" {what}"), muted()),
        Span::raw("  "),
    ]
}

/// Highlights headings and list markers in plain detail or help text.
pub fn styled(text: &str) -> Text<'static> {
    const HEADINGS: [&str; 8] = [
        "Objective",
        "Current state",
        "Links",
        "Keyboard",
        "Commands",
        "Actions",
        "Warnings",
        "Nothing recorded yet.",
    ];
    let lines = text.lines().enumerate().map(|(index, line)| {
        let style = if HEADINGS.contains(&line) {
            heading()
        } else if line.starts_with("- [x]") {
            Style::new().fg(Color::Green).add_modifier(Modifier::DIM)
        } else if line.starts_with("- DEC") {
            Style::new().fg(Color::Cyan)
        } else if index == 0 || (line.contains(" | ") && line.matches(" | ").count() == 2) {
            Style::new().add_modifier(Modifier::BOLD)
        } else if line.starts_with("Status:") || line.starts_with("Created:") {
            muted()
        } else {
            Style::new()
        };
        Line::styled(line.to_owned(), style)
    });
    Text::from(lines.collect::<Vec<_>>())
}
