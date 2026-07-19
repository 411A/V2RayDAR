use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
};

use crate::constants::PROXY_EMOJI;

use super::{
    main_menu_panel::{row_hits_with_offset, visible_row_count},
    util::draw_scrollbar,
    view::RuntimeView,
};

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub fn draw(
    frame: &mut Frame<'_>,
    area: Rect,
    runtime: &RuntimeView,
    top_n: usize,
    scroll: &mut usize,
    found_visible: &mut usize,
    selected_found: Option<&usize>,
    proxy_pending_uri: Option<&str>,
    found_rows: &mut Vec<(usize, Rect)>,
    found_uris: &mut Vec<String>,
    focused: bool,
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
            Row::new(["", "#", "Proto", "Name", "Endpoint", "Latency"]).style(
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            vec![
                Constraint::Length(4),
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
                "", "Rank", "Seen", "Sub Name", "Protocol", "Name", "Endpoint", "Latency",
            ])
            .style(
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            vec![
                Constraint::Length(4),
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

    let visible_rows = visible_row_count(area);
    *found_visible = visible_rows;
    let total_items = runtime.ranked.iter().take(top_n).count();
    let max_offset = total_items.saturating_sub(visible_rows);
    *scroll = (*scroll).min(max_offset);

    *found_rows = row_hits_with_offset(area, total_items, *scroll);

    // Store URIs for proxy selection
    found_uris.clear();
    found_uris.extend(
        runtime
            .ranked
            .iter()
            .take(top_n)
            .map(|item| item.uri.clone()),
    );

    let rows = runtime
        .ranked
        .iter()
        .take(top_n)
        .skip(*scroll)
        .take(visible_rows)
        .enumerate()
        .map(|(visible_index, item)| {
            let latency = item
                .latency_ms
                .map_or_else(|| "-".to_string(), |value| format!("{value} ms"));
            let row_index = *scroll + visible_index;
            let is_selected = selected_found == Some(&row_index);
            // Show 🚪 only after refresh finishes — never during probing.
            let proxy_ready = !runtime.refreshing;
            let is_proxy_row = proxy_ready
                && proxy_pending_uri.map_or(item.is_proxy, |pending| pending == item.uri);
            let proxy_cell = if is_proxy_row {
                Cell::from(PROXY_EMOJI)
            } else {
                Cell::from("")
            };
            let base_style = if is_selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default()
            };
            if very_narrow {
                Row::new([
                    Cell::from(item.rank.to_string()),
                    Cell::from(truncate(&item.protocol, 6)),
                    Cell::from(truncate(&item.display_name, 16)),
                    Cell::from(latency),
                ])
                .style(base_style)
            } else if narrow {
                Row::new([
                    proxy_cell,
                    Cell::from(item.rank.to_string()),
                    Cell::from(truncate(&item.protocol, 8)),
                    Cell::from(truncate(&item.display_name, 16)),
                    Cell::from(truncate(&item.endpoint, 16)),
                    Cell::from(latency),
                ])
                .style(base_style)
            } else {
                Row::new([
                    proxy_cell,
                    Cell::from(item.rank.to_string()),
                    Cell::from(item.stability_count.to_string()),
                    Cell::from(item.source.as_str()),
                    Cell::from(item.protocol.as_str()),
                    Cell::from(item.display_name.as_str()),
                    Cell::from(item.endpoint.as_str()),
                    Cell::from(latency),
                ])
                .style(base_style)
            }
        });

    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    frame.render_widget(
        Table::new(rows, widths).header(header).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
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
