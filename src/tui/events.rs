use std::{path::Path, sync::Arc};

use anyhow::Result;
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use tokio::sync::RwLock;

use crate::{
    constants::{CONFIG_KEYS, MAIN_ITEMS, SUBSCRIPTION_ACTIONS},
    db::Database,
    model::RuntimeConfig,
    paths::AppPaths,
};

use super::{
    action_handlers::run_action,
    input_handlers::{handle_input_key, start_input},
    state::{Action, ConfigKey, InputMode, MainItem, MenuView, SubscriptionAction, TuiState},
};

pub enum EventResult {
    Continue,
    Quit,
}

pub fn handle_key(
    state: &mut TuiState,
    key: KeyEvent,
    paths: &AppPaths,
    runtime_config: &Arc<RwLock<RuntimeConfig>>,
    database: &Arc<Database>,
) -> Result<EventResult> {
    if key.kind != KeyEventKind::Press {
        return Ok(EventResult::Continue);
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(EventResult::Quit);
    }

    if is_back_shortcut(key) {
        go_back(state);
        return Ok(EventResult::Continue);
    }

    match state.input_mode {
        InputMode::Command => handle_command_key(state, key, &paths.config_path),
        InputMode::NewSubscription(_)
        | InputMode::Name
        | InputMode::Url
        | InputMode::Priority
        | InputMode::ConfigValue(_)
        | InputMode::ResetConfirm => Ok(handle_input_key(state, key)),
        InputMode::CleanCacheConfirm => handle_clean_cache_key(state, key, database),
        InputMode::None => handle_normal_key(state, key, paths, runtime_config),
    }
}

fn handle_normal_key(
    state: &mut TuiState,
    key: KeyEvent,
    paths: &AppPaths,
    runtime_config: &Arc<RwLock<RuntimeConfig>>,
) -> Result<EventResult> {
    match key.code {
        KeyCode::Char('q') => return Ok(EventResult::Quit),
        KeyCode::Esc => go_back(state),
        KeyCode::Enter => activate(state, paths, runtime_config)?,
        KeyCode::Up | KeyCode::Char('k') => move_up(state),
        KeyCode::Down | KeyCode::Char('j') => move_down(state),
        KeyCode::Char('e' | 'E') => edit_selected_subscription(state),
        KeyCode::Char(':') => start_input(state, InputMode::Command, ""),
        KeyCode::Char(' ') => run_action(state, Action::Toggle, &paths.config_path)?,
        KeyCode::Char('s') => run_action(state, Action::Save, &paths.config_path)?,
        _ => {}
    }

    Ok(EventResult::Continue)
}

const fn is_back_shortcut(key: KeyEvent) -> bool {
    if !key.modifiers.contains(KeyModifiers::CONTROL) {
        return false;
    }

    matches!(
        key.code,
        KeyCode::Backspace | KeyCode::Delete | KeyCode::Char('h' | 'H')
    )
}

fn handle_command_key(
    state: &mut TuiState,
    key: KeyEvent,
    config_path: &Path,
) -> Result<EventResult> {
    match key.code {
        KeyCode::Esc => cancel_command(state),
        KeyCode::Enter => return run_command(state, config_path),
        KeyCode::Backspace => {
            state.input.pop();
        }
        KeyCode::Char(value) => {
            state.input.push(value);
        }
        _ => {}
    }

    Ok(EventResult::Continue)
}

fn run_command(state: &mut TuiState, config_path: &Path) -> Result<EventResult> {
    let command = state.input.trim().to_ascii_lowercase();
    state.input.clear();
    state.input_mode = InputMode::None;

    match command.as_str() {
        "q" | "quit" => return Ok(EventResult::Quit),
        "a" | "add" => run_action(state, Action::Add, config_path)?,
        "n" | "name" => run_action(state, Action::EditName, config_path)?,
        "u" | "url" => run_action(state, Action::EditUrl, config_path)?,
        "p" | "priority" => run_action(state, Action::EditPriority, config_path)?,
        "t" | "toggle" => run_action(state, Action::Toggle, config_path)?,
        "d" | "delete" => run_action(state, Action::Delete, config_path)?,
        "w" | "save" => run_action(state, Action::Save, config_path)?,
        "" => state.status = "Command cancelled".to_string(),
        _ => state.status = format!("Unknown command: :{command}"),
    }

    Ok(EventResult::Continue)
}

fn cancel_command(state: &mut TuiState) {
    state.input.clear();
    state.input_mode = InputMode::None;
    state.status = "Command cancelled".to_string();
}

pub fn handle_mouse(state: &mut TuiState, mouse: MouseEvent) -> EventResult {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            for (index, area) in &state.hits.main_rows {
                if contains(*area, mouse.column, mouse.row) {
                    state.selected_main = *index;
                    state.status = format!("Selected menu row {}", index + 1);
                    return EventResult::Continue;
                }
            }

            for (index, area) in &state.hits.subscription_rows {
                if contains(*area, mouse.column, mouse.row) {
                    state.selected_subscription = *index;
                    state.status = format!("Selected row {}", index + 1);
                    return EventResult::Continue;
                }
            }

            for (index, area) in &state.hits.config_rows {
                if contains(*area, mouse.column, mouse.row) {
                    state.selected_config = *index;
                    state.status = format!("Selected config row {}", index + 1);
                    return EventResult::Continue;
                }
            }
        }
        MouseEventKind::ScrollUp => scroll_up(state, mouse.column, mouse.row),
        MouseEventKind::ScrollDown => scroll_down(state, mouse.column, mouse.row),
        _ => {}
    }

    EventResult::Continue
}

fn scroll_up(state: &mut TuiState, x: u16, y: u16) {
    if let Some(area) = state.hits.logs_area
        && contains(area, x, y)
    {
        state.scroll.logs = state.scroll.logs.saturating_add(1);
        return;
    }

    if let Some(area) = state.hits.found_area
        && contains(area, x, y)
    {
        state.scroll.found = state.scroll.found.saturating_sub(1);
        return;
    }

    if let Some(area) = state.hits.live_logs_area
        && contains(area, x, y)
        && state.view == MenuView::Logs
    {
        state.selected_log = state.selected_log.saturating_add(1);
        return;
    }

    if state.view == MenuView::Subscriptions
        && state
            .hits
            .subscription_rows
            .iter()
            .any(|(_, area)| contains(*area, x, y))
    {
        state.selected_subscription = state.selected_subscription.saturating_sub(1);
        return;
    }

    if state.view == MenuView::Configurations
        && state
            .hits
            .config_rows
            .iter()
            .any(|(_, area)| contains(*area, x, y))
    {
        state.selected_config = state.selected_config.saturating_sub(1);
    }
}

fn scroll_down(state: &mut TuiState, x: u16, y: u16) {
    if let Some(area) = state.hits.logs_area
        && contains(area, x, y)
    {
        state.scroll.logs = state.scroll.logs.saturating_sub(1);
        return;
    }

    if let Some(area) = state.hits.found_area
        && contains(area, x, y)
    {
        state.scroll.found = state.scroll.found.saturating_add(1);
        return;
    }

    if let Some(area) = state.hits.live_logs_area
        && contains(area, x, y)
        && state.view == MenuView::Logs
    {
        state.selected_log = state.selected_log.saturating_sub(1);
        return;
    }

    if state.view == MenuView::Subscriptions
        && state
            .hits
            .subscription_rows
            .iter()
            .any(|(_, area)| contains(*area, x, y))
    {
        state.selected_subscription =
            (state.selected_subscription + 1).min(state.editable.subscriptions.len());
        return;
    }

    if state.view == MenuView::Configurations
        && state
            .hits
            .config_rows
            .iter()
            .any(|(_, area)| contains(*area, x, y))
    {
        state.selected_config = (state.selected_config + 1).min(CONFIG_KEYS.len() - 1);
    }
}

const fn move_up(state: &mut TuiState) {
    match state.view {
        MenuView::Main => state.selected_main = state.selected_main.saturating_sub(1),
        MenuView::Subscriptions => {
            state.selected_subscription = state.selected_subscription.saturating_sub(1);
        }
        MenuView::NewSubscription => {}
        MenuView::SubscriptionActions => {
            state.selected_action = state.selected_action.saturating_sub(1);
        }
        MenuView::Configurations => state.selected_config = state.selected_config.saturating_sub(1),
        MenuView::Logs => state.selected_log = state.selected_log.saturating_add(1),
    }
}

fn move_down(state: &mut TuiState) {
    match state.view {
        MenuView::Main => {
            state.selected_main = (state.selected_main + 1).min(MAIN_ITEMS.len() - 1);
        }
        MenuView::Subscriptions => {
            state.selected_subscription =
                (state.selected_subscription + 1).min(state.editable.subscriptions.len());
        }
        MenuView::NewSubscription => {}
        MenuView::SubscriptionActions => {
            state.selected_action = (state.selected_action + 1).min(SUBSCRIPTION_ACTIONS.len() - 1);
        }
        MenuView::Configurations => {
            state.selected_config = (state.selected_config + 1).min(CONFIG_KEYS.len() - 1);
        }
        MenuView::Logs => state.selected_log = state.selected_log.saturating_sub(1),
    }
}

fn activate(
    state: &mut TuiState,
    paths: &AppPaths,
    runtime_config: &Arc<RwLock<RuntimeConfig>>,
) -> Result<()> {
    match state.view {
        MenuView::Main => activate_main(state, paths, runtime_config),
        MenuView::Subscriptions => {
            if state.selected_subscription == 0 {
                run_action(state, Action::Add, &paths.config_path)?;
            } else {
                run_action(state, Action::Toggle, &paths.config_path)?;
            }
            Ok(())
        }
        MenuView::NewSubscription | MenuView::Logs => Ok(()),
        MenuView::SubscriptionActions => activate_subscription_action(state, &paths.config_path),
        MenuView::Configurations => {
            let key = CONFIG_KEYS[state.selected_config];
            if key == ConfigKey::ResetDefaults {
                state.reset_code = Some(reset_code());
                start_input(state, InputMode::ResetConfirm, "");
            } else {
                let value = super::config_editor::value(&state.editable, key);
                start_input(state, InputMode::ConfigValue(key), &value);
            }
            Ok(())
        }
    }
}

fn edit_selected_subscription(state: &mut TuiState) {
    if state.view != MenuView::Subscriptions {
        return;
    }

    if state.selected_subscription == 0 {
        state.status = "Press Enter to add a new subscription".to_string();
        return;
    }

    state.view = MenuView::SubscriptionActions;
}

fn activate_main(
    state: &mut TuiState,
    paths: &AppPaths,
    runtime_config: &Arc<RwLock<RuntimeConfig>>,
) -> Result<()> {
    match MAIN_ITEMS[state.selected_main] {
        MainItem::OpenConfig => {
            let message = super::open_config::open(&paths.config_path);
            if message.starts_with("Edit config manually:") {
                state.status = message;
            } else {
                state.status.clear();
            }
        }
        MainItem::Sharing => {
            state.editable.sharing.enabled = !state.editable.sharing.enabled;
            state.dirty = true;
            super::util::save_config(&paths.config_path, &state.editable)?;
            update_live_runtime_config(runtime_config, state);
            state.dirty = false;
            state.status = match super::firewall::apply(
                &paths.root_dir,
                state.editable.sharing.enabled,
                state.editable.bind.port(),
                crate::constants::FIREWALL_RULE_NAME,
            ) {
                Ok(message) => message,
                Err(error) => format!(
                    "Sharing {}; firewall not changed: {error}",
                    if state.editable.sharing.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ),
            };
        }
        MainItem::Proxy => {
            // Three-state cycle: disabled → enabled (local) → enabled (LAN) → disabled
            if !state.editable.proxy.enabled {
                state.editable.proxy.enabled = true;
                state.editable.proxy.discoverable = false;
            } else if !state.editable.proxy.discoverable {
                state.editable.proxy.discoverable = true;
            } else {
                state.editable.proxy.enabled = false;
                state.editable.proxy.discoverable = false;
            }
            state.dirty = true;
            super::util::save_config(&paths.config_path, &state.editable)?;
            update_live_runtime_config(runtime_config, state);
            state.dirty = false;
            state.status = match super::firewall::apply(
                &paths.root_dir,
                state.editable.proxy.discoverable,
                state.editable.proxy.port,
                crate::constants::FIREWALL_PROXY_RULE_NAME,
            ) {
                Ok(message) => message,
                Err(error) => format!(
                    "Proxy discoverable {}; firewall not changed: {error}",
                    if state.editable.proxy.discoverable {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ),
            };
        }
        MainItem::Subscriptions => state.view = MenuView::Subscriptions,
        MainItem::CleanCache => start_input(state, InputMode::CleanCacheConfirm, ""),
        MainItem::Configurations => state.view = MenuView::Configurations,
        MainItem::Logs => {
            state.view = MenuView::Logs;
            state.selected_log = 0;
        }
    }
    Ok(())
}

fn handle_clean_cache_key(
    state: &mut TuiState,
    key: KeyEvent,
    database: &Arc<Database>,
) -> Result<EventResult> {
    match key.code {
        KeyCode::Esc => {
            state.input.clear();
            state.input_mode = InputMode::None;
            state.status = "Clean cache cancelled".to_string();
        }
        KeyCode::Enter => {
            if state.input.trim() == "DELETE" {
                let db = database.clone();
                let result = std::thread::spawn(move || db.delete_all())
                    .join()
                    .map_err(|_| anyhow::anyhow!("database thread panicked"))?;
                match result {
                    Ok(()) => {
                        state.status = "Clean cache finished: database cleared".to_string();
                    }
                    Err(err) => {
                        state.status = format!("Clean cache failed: {err}");
                    }
                }
            } else {
                state.status = "Type DELETE to clean cache".to_string();
                return Ok(EventResult::Continue);
            }
            state.input.clear();
            state.input_mode = InputMode::None;
        }
        KeyCode::Backspace => {
            state.input.pop();
        }
        KeyCode::Char(value) => {
            state.input.push(value);
        }
        _ => {}
    }

    Ok(EventResult::Continue)
}

fn update_live_runtime_config(runtime_config: &Arc<RwLock<RuntimeConfig>>, state: &mut TuiState) {
    match runtime_config.try_write() {
        Ok(mut config) => *config = RuntimeConfig::from(&state.editable),
        Err(_) => {
            state.status =
                "Config saved; live server update is waiting for config reload".to_string();
        }
    }
}

fn activate_subscription_action(state: &mut TuiState, config_path: &Path) -> Result<()> {
    match SUBSCRIPTION_ACTIONS[state.selected_action] {
        SubscriptionAction::EditName => run_action(state, Action::EditName, config_path)?,
        SubscriptionAction::EditUrl => run_action(state, Action::EditUrl, config_path)?,
        SubscriptionAction::EditPriority => run_action(state, Action::EditPriority, config_path)?,
        SubscriptionAction::Toggle => run_action(state, Action::Toggle, config_path)?,
        SubscriptionAction::Delete => {
            run_action(state, Action::Delete, config_path)?;
            state.view = MenuView::Subscriptions;
        }
        SubscriptionAction::Back => state.view = MenuView::Subscriptions,
    }
    Ok(())
}

fn go_back(state: &mut TuiState) {
    state.view = match state.view {
        MenuView::Main | MenuView::Subscriptions | MenuView::Configurations | MenuView::Logs => {
            MenuView::Main
        }
        MenuView::NewSubscription => {
            state.input_mode = InputMode::None;
            state.input.clear();
            state.new_subscription = None;
            MenuView::Subscriptions
        }
        MenuView::SubscriptionActions => MenuView::Subscriptions,
    };
}

fn reset_code() -> String {
    let value = (chrono::Local::now().timestamp_subsec_millis() % 9000) + 1000;
    value.to_string()
}

const fn contains(area: ratatui::layout::Rect, x: u16, y: u16) -> bool {
    x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height
}
