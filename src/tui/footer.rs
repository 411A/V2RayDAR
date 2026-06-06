use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::state::{InputMode, TuiState};

pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let input = match state.input_mode {
        InputMode::Command => format!("  :{}", state.input),
        _ => String::new(),
    };
    let dirty = if state.dirty { "unsaved" } else { "saved" };
    let dirty_color = if state.dirty {
        Color::Yellow
    } else {
        Color::Green
    };
    let line = Line::from(vec![
        Span::styled(
            "Up/Down or j/k nav | Enter select/edit | Esc/Ctrl+H back | :save :q",
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("   "),
        Span::styled(dirty, Style::default().fg(dirty_color)),
        Span::raw("   "),
        Span::raw(state.status.clone()),
        Span::styled(input, Style::default().fg(Color::Cyan)),
    ]);

    frame.render_widget(Paragraph::new(line), area);
}
