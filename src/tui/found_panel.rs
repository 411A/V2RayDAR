use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
};

use super::{util::draw_scrollbar, view::RuntimeView};

#[allow(clippy::too_many_lines)]
pub fn draw(
    frame: &mut Frame<'_>,
    area: Rect,
    runtime: &RuntimeView,
    top_n: usize,
    scroll: &mut usize,
) {
    if area.width < 30 || area.height < 3 {
        return;
    }

    let narrow = area.width < 80;
    let very_narrow = area.width < 55;

    let (header, widths) = if very_narrow {
        (
            Row::new(["#", "Proto", "Name", "Latency"]).style(
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            vec![
                Constraint::Length(4),
                Constraint::Length(6),
                Constraint::Fill(1),
                Constraint::Length(9),
            ],
        )
    } else if narrow {
        (
            Row::new(["#", "Proto", "Name", "Endpoint", "Latency"]).style(
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            vec![
                Constraint::Length(4),
                Constraint::Length(8),
                Constraint::Length(16),
                Constraint::Fill(1),
                Constraint::Length(9),
            ],
        )
    } else {
        (
            Row::new([
                "Rank", "Seen", "Sub Name", "Protocol", "Name", "Endpoint", "Latency",
            ])
            .style(
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            vec![
                Constraint::Length(6),
                Constraint::Length(6),
                Constraint::Length(22),
                Constraint::Length(12),
                Constraint::Length(28),
                Constraint::Length(24),
                Constraint::Fill(1),
            ],
        )
    };

    let visible_rows = area.height.saturating_sub(3) as usize;
    let total_items = runtime.ranked.iter().take(top_n).count();
    let max_offset = total_items.saturating_sub(visible_rows);
    *scroll = (*scroll).min(max_offset);

    let rows = runtime
        .ranked
        .iter()
        .take(top_n)
        .skip(*scroll)
        .take(visible_rows)
        .map(|item| {
            let latency = item
                .latency_ms
                .map_or_else(|| "-".to_string(), |value| format!("{value} ms"));
            if very_narrow {
                Row::new([
                    Cell::from(item.rank.to_string()),
                    Cell::from(truncate(&item.protocol, 6)),
                    Cell::from(truncate(&item.display_name, 16)),
                    Cell::from(latency),
                ])
            } else if narrow {
                Row::new([
                    Cell::from(item.rank.to_string()),
                    Cell::from(truncate(&item.protocol, 8)),
                    Cell::from(truncate(&item.display_name, 16)),
                    Cell::from(truncate(&item.endpoint, 16)),
                    Cell::from(latency),
                ])
            } else {
                Row::new([
                    Cell::from(item.rank.to_string()),
                    Cell::from(item.stability_count.to_string()),
                    Cell::from(item.source.as_str()),
                    Cell::from(item.protocol.as_str()),
                    Cell::from(item.display_name.as_str()),
                    Cell::from(item.endpoint.as_str()),
                    Cell::from(latency),
                ])
            }
        });

    frame.render_widget(
        Table::new(rows, widths).header(header).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Current Found Configs"),
        ),
        area,
    );
    draw_scrollbar(frame, area, total_items, visible_rows, *scroll, false);
}

fn truncate(value: &str, width: usize) -> std::borrow::Cow<'_, str> {
    if value.len() <= width {
        std::borrow::Cow::Borrowed(value)
    } else if width > 1 {
        let truncated: String = value.chars().take(width.saturating_sub(1)).collect();
        std::borrow::Cow::Owned(format!("{truncated}~"))
    } else {
        let first: String = value.chars().take(1).collect();
        std::borrow::Cow::Owned(first)
    }
}
