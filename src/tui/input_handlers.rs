use crossterm::event::{KeyCode, KeyEvent};

use crate::config::{AppConfig, SubscriptionSource};

use super::{
    events::EventResult,
    state::{InputMode, MenuView, NewSubscriptionStep, SubscriptionDraft, TuiState},
};

pub fn start_new_subscription(state: &mut TuiState) {
    state.view = MenuView::NewSubscription;
    state.new_subscription = Some(SubscriptionDraft {
        name: String::new(),
        url: String::new(),
        priority: state.next_subscription_priority(),
        enabled: true,
    });
    start_input(
        state,
        InputMode::NewSubscription(NewSubscriptionStep::Url),
        "",
    );
}

pub fn start_input(state: &mut TuiState, mode: InputMode, value: &str) {
    state.input_mode = mode;
    state.input.clear();
    state.input.push_str(value);
    state.status = match state.input_mode {
        InputMode::Command => {
            "Command mode; type add/name/url/priority/toggle/delete/save/q".to_string()
        }
        InputMode::NewSubscription(step) => new_subscription_guide(step).to_string(),
        InputMode::ConfigValue(key) => format!(
            "{}; Enter applies, Esc cancels",
            super::config_editor::guide(key)
        ),
        InputMode::ResetConfirm => format!(
            "Type {} to reset non-subscription settings",
            state.reset_code.as_deref().unwrap_or("code")
        ),
        InputMode::CleanCacheConfirm => {
            "Type DELETE to clean cached subscription snapshots".to_string()
        }
        _ => "Edit mode; Enter applies, Esc cancels".to_string(),
    };
}

pub fn handle_input_key(state: &mut TuiState, key: KeyEvent) -> EventResult {
    match key.code {
        KeyCode::Esc => {
            state.input_mode = InputMode::None;
            if state.view == MenuView::NewSubscription {
                state.view = MenuView::Subscriptions;
                state.new_subscription = None;
            }
            state.reset_code = None;
            state.input.clear();
            state.status = "Edit cancelled".to_string();
        }
        KeyCode::Enter => commit_input(state),
        KeyCode::Backspace => {
            state.input.pop();
        }
        KeyCode::Char(value) => {
            state.input.push(value);
        }
        _ => {}
    }

    EventResult::Continue
}

fn commit_input(state: &mut TuiState) {
    match state.input_mode {
        InputMode::None | InputMode::Command | InputMode::CleanCacheConfirm => {}
        InputMode::NewSubscription(step) => commit_new_subscription_step(state, step),
        InputMode::Name => commit_name(state),
        InputMode::Url => commit_url(state),
        InputMode::Priority => commit_priority(state),
        InputMode::ConfigValue(key) => commit_config(state, key),
        InputMode::ResetConfirm => commit_reset(state),
    }
}

fn commit_name(state: &mut TuiState) {
    let value = state.input.trim().to_string();
    if value.is_empty() {
        state.status = "Name cannot be empty".to_string();
        return;
    }

    if let Some(source) = state.selected_subscription_mut() {
        source.name = value;
        state.dirty = true;
        finish_edit(state, "Name updated");
    }
}

fn commit_url(state: &mut TuiState) {
    let value = state.input.trim().to_string();
    if value.is_empty() {
        state.status = "URL cannot be empty".to_string();
        return;
    }

    if let Some(source) = state.selected_subscription_mut() {
        source.url = value;
        state.dirty = true;
        finish_edit(state, "URL updated");
    }
}

fn commit_priority(state: &mut TuiState) {
    let Ok(value) = state.input.trim().parse::<u32>() else {
        state.status = "Priority must be a number".to_string();
        return;
    };

    if let Some(source) = state.selected_subscription_mut() {
        source.priority = value;
        state.dirty = true;
        finish_edit(state, "Priority updated");
    }
}

fn commit_new_subscription_step(state: &mut TuiState, step: NewSubscriptionStep) {
    match step {
        NewSubscriptionStep::Url => commit_new_url(state),
        NewSubscriptionStep::Name => commit_new_name(state),
        NewSubscriptionStep::Priority => commit_new_priority(state),
        NewSubscriptionStep::Enabled => commit_new_enabled(state),
    }
}

fn commit_new_url(state: &mut TuiState) {
    let value = state.input.trim().to_string();
    if value.is_empty() {
        state.status = "URL cannot be empty".to_string();
        return;
    }
    if let Some(draft) = state.new_subscription.as_mut() {
        draft.url = value;
        start_input(
            state,
            InputMode::NewSubscription(NewSubscriptionStep::Name),
            "",
        );
    }
}

fn commit_new_name(state: &mut TuiState) {
    let value = state.input.trim().to_string();
    if value.is_empty() {
        state.status = "Name cannot be empty".to_string();
        return;
    }
    if let Some(draft) = state.new_subscription.as_mut() {
        draft.name = value;
        let priority = draft.priority.to_string();
        start_input(
            state,
            InputMode::NewSubscription(NewSubscriptionStep::Priority),
            &priority,
        );
    }
}

fn commit_new_priority(state: &mut TuiState) {
    let Ok(value) = state.input.trim().parse::<u32>() else {
        state.status = "Priority must be a number".to_string();
        return;
    };
    if let Some(draft) = state.new_subscription.as_mut() {
        draft.priority = value;
        let enabled = if draft.enabled { "yes" } else { "no" };
        start_input(
            state,
            InputMode::NewSubscription(NewSubscriptionStep::Enabled),
            enabled,
        );
    }
}

fn commit_new_enabled(state: &mut TuiState) {
    let Some(enabled) = parse_bool(state.input.trim()) else {
        state.status = "Enabled must be yes/no, true/false, on/off, or 1/0".to_string();
        return;
    };

    let Some(mut draft) = state.new_subscription.take() else {
        state.status = "New subscription draft is missing".to_string();
        return;
    };
    draft.enabled = enabled;
    state.editable.subscriptions.push(SubscriptionSource {
        name: draft.name,
        url: draft.url,
        enabled: draft.enabled,
        priority: draft.priority,
    });
    state.selected_subscription = state.editable.subscriptions.len();
    state.view = MenuView::Subscriptions;
    state.dirty = true;
    finish_edit(state, "Subscription added");
}

fn commit_config(state: &mut TuiState, key: super::state::ConfigKey) {
    let input = state.input.clone();
    match super::config_editor::apply(&mut state.editable, key, &input) {
        Ok(()) => {
            state.dirty = true;
            finish_edit(state, "Configuration updated");
        }
        Err(error) => state.status = error.to_string(),
    }
}

fn commit_reset(state: &mut TuiState) {
    let expected = state.reset_code.clone().unwrap_or_default();
    if state.input.trim() != expected {
        state.status = "Reset code did not match".to_string();
        return;
    }
    let subscriptions = state.editable.subscriptions.clone();
    state.editable = AppConfig::default_for_first_run();
    state.editable.subscriptions = subscriptions;
    state.reset_code = None;
    state.dirty = true;
    finish_edit(state, "Defaults restored; subscriptions kept");
}

fn finish_edit(state: &mut TuiState, message: &str) {
    state.input_mode = InputMode::None;
    state.input.clear();
    state.status = message.to_string();
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" | "enabled" => Some(true),
        "0" | "false" | "no" | "n" | "off" | "disabled" => Some(false),
        _ => None,
    }
}

const fn new_subscription_guide(step: NewSubscriptionStep) -> &'static str {
    match step {
        NewSubscriptionStep::Url => "Step 1/4: enter the subscription URL",
        NewSubscriptionStep::Name => "Step 2/4: enter a display name",
        NewSubscriptionStep::Priority => "Step 3/4: enter priority as a number",
        NewSubscriptionStep::Enabled => "Step 4/4: enable now? yes/no",
    }
}
