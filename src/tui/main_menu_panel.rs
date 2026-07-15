use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Cell, Row, Table},
};

use crate::constants::{CONFIG_FILE_NAME, CONFIG_KEYS, MAIN_ITEMS, SUBSCRIPTION_ACTIONS};

use super::{
    config_editor,
    state::{
        ConfigKey, InputMode, MainItem, MenuView, NewSubscriptionStep, SubscriptionAction, TuiState,
    },
    util::draw_scrollbar,
    view::RuntimeView,
};

pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &mut TuiState, runtime: &RuntimeView) {
    match state.view {
        MenuView::Main => draw_main(frame, area, state),
        MenuView::Subscriptions => super::subscriptions_panel::draw(frame, area, state),
        MenuView::NewSubscription => draw_new_subscription(frame, area, state),
        MenuView::SubscriptionActions => draw_subscription_actions(frame, area, state),
        MenuView::Configurations => draw_configurations(frame, area, state),
        MenuView::Logs => {
            state.hits.live_logs_area = Some(area);
            draw_logs(frame, area, state, runtime);
        }
    }
}

fn draw_main(frame: &mut Frame<'_>, area: Rect, state: &mut TuiState) {
    let visible_rows = visible_row_count(area).max(1);
    let total = MAIN_ITEMS.len();
    let offset = scroll_offset(state.selected_main, total, visible_rows);
    let rows = MAIN_ITEMS
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible_rows)
        .map(|(index, item)| {
            let (name, value) = match item {
                MainItem::OpenConfig => ("Open Configs File", CONFIG_FILE_NAME),
                MainItem::Sharing => (
                    "Share subscription URL on LAN",
                    if state.editable.sharing.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    },
                ),
                MainItem::Proxy => (
                    "Persistent proxy for app traffic",
                    if state.editable.proxy.enabled {
                        if state.editable.proxy.discoverable {
                            "enabled (LAN)"
                        } else {
                            "enabled"
                        }
                    } else {
                        "disabled"
                    },
                ),
                MainItem::Subscriptions => ("Subscriptions", "enter to manage sources"),
                MainItem::CleanCache => ("Clean Cache", "delete cached subscription snapshots"),
                MainItem::Configurations => ("Configurations", "enter to edit config values"),
                MainItem::Logs => ("Live Logs", "enter to inspect refresh progress"),
            };
            Row::new([Cell::from(name), Cell::from(value)])
                .style(row_style(index == state.selected_main, value))
        });
    state.hits.main_rows = row_hits_with_offset(area, total, offset);
    render_table(
        frame,
        area,
        "Main Menu",
        vec!["Item", "Value"],
        vec![Constraint::Length(34), Constraint::Fill(1)],
        rows,
    );
    draw_scrollbar(frame, area, total, visible_rows, offset, false);
}

fn draw_logs(frame: &mut Frame<'_>, area: Rect, state: &mut TuiState, runtime: &RuntimeView) {
    let visible_rows = visible_row_count(area).max(1);
    let total = runtime.live_logs.len();
    let max_offset = total.saturating_sub(visible_rows);
    state.selected_log = state.selected_log.min(max_offset);
    let start = max_offset.saturating_sub(state.selected_log);
    let lines = if runtime.live_logs.is_empty() {
        vec![Line::from("Waiting for refresh logs...")]
    } else {
        runtime
            .live_logs
            .iter()
            .skip(start)
            .take(visible_rows)
            .map(|line| Line::from(line.clone()))
            .collect()
    };

    frame.render_widget(
        ratatui::widgets::Paragraph::new(lines)
            .style(Style::default().fg(Color::Gray))
            .block(Block::default().borders(Borders::ALL).title("Live Logs"))
            .wrap(ratatui::widgets::Wrap { trim: true }),
        area,
    );
    draw_scrollbar(frame, area, total, visible_rows, state.selected_log, true);
}

fn draw_subscription_actions(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let selected = state.selected_subscription_ref();
    let visible_rows = visible_row_count(area).max(1);
    let total = SUBSCRIPTION_ACTIONS.len();
    let offset = scroll_offset(state.selected_action, total, visible_rows);
    let rows = SUBSCRIPTION_ACTIONS
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible_rows)
        .map(|(index, action)| {
            Row::new([
                Cell::from(action_label(*action)),
                Cell::from(action_value(*action, selected, state)),
            ])
            .style(row_style(index == state.selected_action, ""))
        });
    render_table(
        frame,
        area,
        "Subscription Actions",
        vec!["Action", "Target"],
        vec![Constraint::Length(24), Constraint::Fill(1)],
        rows,
    );
    draw_scrollbar(frame, area, total, visible_rows, offset, false);
}

fn draw_new_subscription(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let draft = state.new_subscription.as_ref();
    let current = match state.input_mode {
        InputMode::NewSubscription(step) => Some(step),
        _ => None,
    };
    let rows = [
        wizard_row(
            current == Some(NewSubscriptionStep::Url),
            "1",
            "Subscription URL",
            wizard_value(
                state,
                NewSubscriptionStep::Url,
                draft.map(|d| d.url.as_str()),
            ),
            "Paste the full subscription URL",
        ),
        wizard_row(
            current == Some(NewSubscriptionStep::Name),
            "2",
            "Display name",
            wizard_value(
                state,
                NewSubscriptionStep::Name,
                draft.map(|d| d.name.as_str()),
            ),
            "Short human-readable name",
        ),
        wizard_row(
            current == Some(NewSubscriptionStep::Priority),
            "3",
            "Priority",
            wizard_value(
                state,
                NewSubscriptionStep::Priority,
                draft.map(|d| d.priority.to_string()).as_deref(),
            ),
            "Lower numbers are listed first",
        ),
        wizard_row(
            current == Some(NewSubscriptionStep::Enabled),
            "4",
            "Enabled",
            wizard_value(
                state,
                NewSubscriptionStep::Enabled,
                draft.map(|d| if d.enabled { "yes" } else { "no" }),
            ),
            "yes/no, true/false, on/off",
        ),
    ];
    render_table(
        frame,
        area,
        "New Subscription",
        vec!["Step", "Field", "Value", "Guide"],
        vec![
            Constraint::Length(6),
            Constraint::Length(20),
            Constraint::Length(34),
            Constraint::Fill(1),
        ],
        rows.into_iter(),
    );
}

fn draw_configurations(frame: &mut Frame<'_>, area: Rect, state: &mut TuiState) {
    let visible_rows = visible_row_count(area);
    let total = CONFIG_KEYS.len();
    let offset = scroll_offset(state.selected_config, total, visible_rows);
    state.hits.config_rows = row_hits_with_offset(area, total, offset);
    let rows = CONFIG_KEYS
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible_rows)
        .map(|(index, key)| {
            Row::new([
                Cell::from(config_editor::label(*key)),
                Cell::from(config_value(state, *key)),
                Cell::from(config_editor::guide(*key)),
            ])
            .style(row_style(index == state.selected_config, ""))
        });
    render_table(
        frame,
        area,
        "Configurations",
        vec!["Key", "Value", "Guide"],
        vec![
            Constraint::Length(28),
            Constraint::Length(28),
            Constraint::Fill(1),
        ],
        rows,
    );
    draw_scrollbar(frame, area, total, visible_rows, offset, false);
}

fn config_value(state: &TuiState, key: ConfigKey) -> String {
    match (&state.input_mode, key) {
        (InputMode::ConfigValue(active), _) if *active == key => {
            format!("{}_", state.input)
        }
        (InputMode::ResetConfirm, ConfigKey::ResetDefaults) => {
            format!("{}_", state.input)
        }
        _ => config_editor::value(&state.editable, key),
    }
}

fn render_table<'a>(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &'static str,
    header: Vec<&'static str>,
    widths: Vec<Constraint>,
    rows: impl Iterator<Item = Row<'a>>,
) {
    let header = Row::new(header).style(
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(
        Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

fn wizard_row<'a>(
    selected: bool,
    step: &'static str,
    field: &'static str,
    value: String,
    guide: &'static str,
) -> Row<'a> {
    Row::new([
        Cell::from(step),
        Cell::from(field),
        Cell::from(value),
        Cell::from(guide),
    ])
    .style(row_style(selected, ""))
}

fn wizard_value(state: &TuiState, step: NewSubscriptionStep, committed: Option<&str>) -> String {
    if state.input_mode == InputMode::NewSubscription(step) {
        return format!("{}_", state.input);
    }
    committed
        .filter(|value| !value.is_empty())
        .unwrap_or("-")
        .to_string()
}

fn row_style(selected: bool, value: &str) -> Style {
    if selected {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else if value == "enabled" {
        Style::default().fg(Color::Green)
    } else if value == "disabled" {
        Style::default().fg(Color::Red)
    } else {
        Style::default()
    }
}

const fn action_label(action: SubscriptionAction) -> &'static str {
    match action {
        SubscriptionAction::EditName => "Edit name",
        SubscriptionAction::EditUrl => "Edit URL",
        SubscriptionAction::EditPriority => "Edit priority",
        SubscriptionAction::Toggle => "Enable/disable",
        SubscriptionAction::Delete => "Delete",
        SubscriptionAction::Back => "Back",
    }
}

fn active_subscription_input(action: SubscriptionAction, state: &TuiState) -> Option<String> {
    match (&state.input_mode, action) {
        (InputMode::Name, SubscriptionAction::EditName)
        | (InputMode::Url, SubscriptionAction::EditUrl)
        | (InputMode::Priority, SubscriptionAction::EditPriority) => {
            Some(format!("{}_", state.input))
        }
        _ => None,
    }
}

fn action_value(
    action: SubscriptionAction,
    source: Option<&crate::config::SubscriptionSource>,
    state: &TuiState,
) -> String {
    if let Some(value) = active_subscription_input(action, state) {
        return value;
    }
    let Some(source) = source else {
        return "-".to_string();
    };
    match action {
        SubscriptionAction::EditName => source.name.clone(),
        SubscriptionAction::EditUrl => source.url.clone(),
        SubscriptionAction::EditPriority => source.priority.to_string(),
        SubscriptionAction::Toggle => {
            if source.enabled {
                "enabled".to_string()
            } else {
                "disabled".to_string()
            }
        }
        SubscriptionAction::Delete => format!("delete {}", source.name),
        SubscriptionAction::Back => "return to subscriptions".to_string(),
    }
}

pub fn row_hits_with_offset(area: Rect, count: usize, offset: usize) -> Vec<(usize, Rect)> {
    let mut rows = Vec::new();
    let first_y = area.y.saturating_add(2);
    let last_y = area.y.saturating_add(area.height.saturating_sub(1));
    for index in offset..count {
        let visible_index = index.saturating_sub(offset);
        let y = first_y.saturating_add(usize_to_u16_saturating(visible_index));
        if y >= last_y {
            break;
        }
        rows.push((
            index,
            Rect::new(area.x + 1, y, area.width.saturating_sub(2), 1),
        ));
    }
    rows
}

pub fn usize_to_u16_saturating(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

pub const fn visible_row_count(area: Rect) -> usize {
    area.height.saturating_sub(3) as usize
}

pub const fn scroll_offset(selected: usize, total: usize, visible: usize) -> usize {
    if visible == 0 || total <= visible {
        return 0;
    }

    selected.saturating_add(1).saturating_sub(visible)
}
