use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::{util::draw_scrollbar, view::RuntimeView};

pub fn draw(frame: &mut Frame<'_>, area: Rect, runtime: &RuntimeView, scroll: &mut usize) {
    if area.height < 3 || area.width < 10 {
        return;
    }
    let visible_height = area.height.saturating_sub(2) as usize;
    let total = runtime.logs.len();
    let max_offset = total.saturating_sub(visible_height);
    *scroll = (*scroll).min(max_offset);

    let lines = if runtime.logs.is_empty() {
        vec![Line::from("Waiting for refresh logs...")]
    } else {
        let start = max_offset.saturating_sub(*scroll);
        runtime
            .logs
            .iter()
            .skip(start)
            .take(visible_height)
            .map(|line| Line::from(line.clone()))
            .collect()
    };

    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(Color::Gray))
            .block(Block::default().borders(Borders::ALL).title("Recent Logs"))
            .wrap(Wrap { trim: true }),
        area,
    );
    draw_scrollbar(frame, area, total, visible_height, *scroll, true);
}
