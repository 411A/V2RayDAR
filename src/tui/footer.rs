use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::state::{InputMode, MenuView, TuiState};

pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let input = match state.input_mode {
        InputMode::Command => format!(":{}", state.input),
        InputMode::None => String::new(),
        _ => state.input.clone(),
    };
    let dirty = if state.dirty { "unsaved" } else { "saved" };
    let dirty_color = if state.dirty {
        Color::Yellow
    } else {
        Color::Green
    };
    let guide_text = if state.view == MenuView::Logs {
        "Up/k older | Down/j newer | Esc/Ctrl+H back | :q"
    } else if state.view == MenuView::Subscriptions {
        "Up/Down or j/k nav | Enter toggle/add | e edit | Esc/Ctrl+H back | :save :q"
    } else {
        "Up/Down or j/k nav | Enter select/edit | Esc/Ctrl+H back | :save :q"
    };
    let guide = Line::from(Span::styled(
        guide_text,
        Style::default().fg(Color::DarkGray),
    ));
    let mut activity = vec![Span::styled(dirty, Style::default().fg(dirty_color))];
    if !input.is_empty() {
        activity.push(Span::raw(" | "));
        activity.push(Span::styled(input, Style::default().fg(Color::Cyan)));
    }
    if !state.status.is_empty() {
        activity.push(Span::raw(" | "));
        activity.push(Span::raw(state.status.clone()));
    }

    frame.render_widget(Paragraph::new(vec![guide, Line::from(activity)]), area);
}
