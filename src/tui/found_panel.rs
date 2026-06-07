use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
};

use super::view::RuntimeView;

pub fn draw(frame: &mut Frame<'_>, area: Rect, runtime: &RuntimeView, top_n: usize) {
    let header = Row::new([
        "Rank",
        "Subscription Name",
        "Protocol",
        "Name",
        "Endpoint",
        "Latency",
    ])
    .style(
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );
    let rows = runtime.ranked.iter().take(top_n).map(|item| {
        let latency = item
            .latency_ms
            .map(|value| format!("{value} ms"))
            .unwrap_or_else(|| "-".to_string());
        Row::new([
            Cell::from(item.rank.to_string()),
            Cell::from(item.source.clone()),
            Cell::from(item.protocol.clone()),
            Cell::from(item.name.clone()),
            Cell::from(item.endpoint.clone()),
            Cell::from(latency),
        ])
    });

    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(6),
                Constraint::Length(22),
                Constraint::Length(12),
                Constraint::Length(28),
                Constraint::Length(24),
                Constraint::Fill(1),
            ],
        )
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Current Found Configs"),
        ),
        area,
    );
}
