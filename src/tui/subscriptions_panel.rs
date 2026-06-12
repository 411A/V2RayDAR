use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
};

use super::state::TuiState;

pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &mut TuiState) {
    let header = Row::new(["#", "✓/✗", "Priority", "Name", "URL"]).style(
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );
    let add_row = std::iter::once(
        Row::new([
            Cell::from("+"),
            Cell::from("+"),
            Cell::from("-"),
            Cell::from("New Subscription"),
            Cell::from("Enter to start guided setup"),
        ])
        .style(if state.selected_subscription == 0 {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::Green)
        }),
    );
    let rows = add_row.chain(state.editable.subscriptions.iter().enumerate().map(
        |(index, source)| {
            let row_index = index + 1;
            let selected = row_index == state.selected_subscription;
            let enabled = if source.enabled { "✓" } else { "✗" };
            let style = if selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else if source.enabled {
                Style::default()
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Row::new([
                Cell::from(row_index.to_string()),
                Cell::from(enabled),
                Cell::from(source.priority.to_string()),
                Cell::from(source.name.clone()),
                Cell::from(source.url.clone()),
            ])
            .style(style)
        },
    ));

    state.hits.subscription_rows = row_hits(area, state.editable.subscriptions.len() + 1);
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(4),
                Constraint::Length(4),
                Constraint::Length(10),
                Constraint::Length(20),
                Constraint::Fill(1),
            ],
        )
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Subscriptions"),
        ),
        area,
    );
}

fn row_hits(area: Rect, count: usize) -> Vec<(usize, Rect)> {
    let mut rows = Vec::new();
    let first_y = area.y.saturating_add(2);
    let last_y = area.y.saturating_add(area.height.saturating_sub(1));
    for index in 0..count {
        let y = first_y.saturating_add(usize_to_u16_saturating(index));
        if y >= last_y {
            break;
        }
        rows.push((
            index,
            Rect::new(area.x.saturating_add(1), y, area.width.saturating_sub(2), 1),
        ));
    }
    rows
}

fn usize_to_u16_saturating(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}
