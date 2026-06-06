use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
};

use super::{
    config_editor,
    state::{
        ConfigKey, InputMode, MainItem, MenuView, NewSubscriptionStep, SubscriptionAction, TuiState,
    },
};

pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &mut TuiState) {
    match state.view {
        MenuView::Main => draw_main(frame, area, state),
        MenuView::Subscriptions => super::subscriptions_panel::draw(frame, area, state),
        MenuView::NewSubscription => draw_new_subscription(frame, area, state),
        MenuView::SubscriptionActions => draw_subscription_actions(frame, area, state),
        MenuView::Configurations => draw_configurations(frame, area, state),
    }
}

fn draw_main(frame: &mut Frame<'_>, area: Rect, state: &mut TuiState) {
    let rows = MainItem::ALL.iter().enumerate().map(|(index, item)| {
        let (name, value) = match item {
            MainItem::Sharing => (
                "Share subscription URL on LAN",
                if state.editable.sharing.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
            ),
            MainItem::Subscriptions => ("Subscriptions", "enter to manage sources"),
            MainItem::Configurations => ("Configurations", "enter to edit configs.yaml"),
        };
        Row::new([Cell::from(name), Cell::from(value)])
            .style(row_style(index == state.selected_main, value))
    });
    state.hits.main_rows = row_hits(area, MainItem::ALL.len());
    render_table(
        frame,
        area,
        "Main Menu",
        vec!["Item", "Value"],
        vec![Constraint::Length(34), Constraint::Fill(1)],
        rows,
    );
}

fn draw_subscription_actions(frame: &mut Frame<'_>, area: Rect, state: &mut TuiState) {
    let selected = state.selected_subscription_ref();
    let rows = SubscriptionAction::ALL
        .iter()
        .enumerate()
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
    state.hits.config_rows = row_hits(area, ConfigKey::ALL.len());
    let rows = ConfigKey::ALL.iter().enumerate().map(|(index, key)| {
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

fn action_label(action: SubscriptionAction) -> &'static str {
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

fn row_hits(area: Rect, count: usize) -> Vec<(usize, Rect)> {
    let mut rows = Vec::new();
    let first_y = area.y.saturating_add(2);
    let last_y = area.y.saturating_add(area.height.saturating_sub(1));
    for index in 0..count {
        let y = first_y.saturating_add(index as u16);
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
