use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
};

use super::{
    main_menu_panel::{row_hits_with_offset, scroll_offset, visible_row_count},
    state::TuiState,
    util::draw_scrollbar,
};

pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &mut TuiState) {
    let total = state.editable.subscriptions.len() + 1;
    let visible_rows = visible_row_count(area).max(1);
    let offset = scroll_offset(state.selected_subscription, total, visible_rows);

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
    let rows = add_row
        .chain(
            state
                .editable
                .subscriptions
                .iter()
                .enumerate()
                .map(|(index, source)| {
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
                }),
        )
        .skip(offset)
        .take(visible_rows);

    state.hits.subscription_rows = row_hits_with_offset(area, total, offset);
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
    draw_scrollbar(frame, area, total, visible_rows, offset, false);
}
