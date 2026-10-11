//! The fio mascot, a cat batting a ball of yarn, drawn with half-block characters.
mod frames;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use frames::{FRAMES, HEIGHT, WIDTH, color};

// The sprite is a few dozen pixels, far below `u16::MAX`.
#[allow(clippy::cast_possible_truncation)]
pub const COLUMNS: u16 = WIDTH as u16;
#[allow(clippy::cast_possible_truncation)]
pub const ROWS: u16 = (HEIGHT / 2) as u16;
pub const FRAME_COUNT: usize = FRAMES.len();

fn rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb(r, g, b)
}

/// Two pixel rows become one terminal row: `▀` carries the top pixel in the
/// foreground and the bottom pixel in the background.
fn lines(frame: usize) -> Vec<Line<'static>> {
    FRAMES[frame % FRAME_COUNT]
        .chunks(2)
        .map(|pair| {
            let top = pair[0].as_bytes();
            let bottom = pair.get(1).map_or(&[][..], |row| row.as_bytes());
            let spans = (0..WIDTH).map(|x| {
                let top = color(top[x]);
                let bottom = bottom.get(x).and_then(|pixel| color(*pixel));
                match (top, bottom) {
                    (None, None) => Span::raw(" "),
                    (Some(t), Some(b)) => Span::styled("▀", Style::new().fg(rgb(t)).bg(rgb(b))),
                    (Some(t), None) => Span::styled("▀", Style::new().fg(rgb(t))),
                    (None, Some(b)) => Span::styled("▄", Style::new().fg(rgb(b))),
                }
            });
            Line::from(spans.collect::<Vec<_>>())
        })
        .collect()
}

/// Whether the mascot fits in `area` with `extra` rows to spare.
pub fn fits(area: Rect, extra: u16) -> bool {
    area.width >= COLUMNS + 2 && area.height >= ROWS + extra
}

/// Draws the mascot centered horizontally at the top of `area`.
pub fn render(frame: &mut Frame, area: Rect, index: usize) {
    let x = area.x + area.width.saturating_sub(COLUMNS) / 2;
    let target = Rect::new(x, area.y, COLUMNS.min(area.width), ROWS.min(area.height));
    frame.render_widget(Paragraph::new(lines(index)), target);
}
