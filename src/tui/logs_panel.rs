use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::model::RuntimeState;

pub fn draw(frame: &mut Frame<'_>, area: Rect, runtime: &RuntimeState) {
    let lines = if runtime.logs.is_empty() {
        vec![Line::from("Waiting for refresh logs...")]
    } else {
        runtime
            .logs
            .iter()
            .rev()
            .take(area.height.saturating_sub(2) as usize)
            .rev()
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
}
