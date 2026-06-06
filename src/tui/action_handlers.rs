use std::path::Path;

use anyhow::Result;

use super::{
    input_handlers::{start_input, start_new_subscription},
    state::{Action, InputMode, TuiState},
    util::save_config,
};

pub fn run_action(state: &mut TuiState, action: Action, config_path: &Path) -> Result<()> {
    match action {
        Action::Add => start_new_subscription(state),
        Action::EditName => {
            let value = selected_value(state, |source| source.name.clone());
            start_input(state, InputMode::Name, &value);
        }
        Action::EditUrl => {
            let value = selected_value(state, |source| source.url.clone());
            start_input(state, InputMode::Url, &value);
        }
        Action::EditPriority => {
            let value = selected_value(state, |source| source.priority.to_string());
            start_input(state, InputMode::Priority, &value);
        }
        Action::Toggle => toggle_subscription(state),
        Action::Delete => delete_subscription(state),
        Action::Save => save_now(state, config_path)?,
    }

    Ok(())
}

fn selected_value<F>(state: &TuiState, f: F) -> String
where
    F: FnOnce(&crate::config::SubscriptionSource) -> String,
{
    state.selected_subscription_ref().map(f).unwrap_or_default()
}

fn toggle_subscription(state: &mut TuiState) {
    let message = if let Some(source) = state.selected_subscription_mut() {
        source.enabled = !source.enabled;
        Some(format!(
            "{} is now {}",
            source.name,
            if source.enabled {
                "enabled"
            } else {
                "disabled"
            }
        ))
    } else {
        None
    };

    if let Some(message) = message {
        state.dirty = true;
        state.status = message;
    }
}

fn delete_subscription(state: &mut TuiState) {
    if state.editable.subscriptions.is_empty() {
        state.status = "No subscription to delete".to_string();
        return;
    }

    let Some(index) = state.selected_subscription_index() else {
        state.status = "No subscription selected".to_string();
        return;
    };
    let removed = state.editable.subscriptions.remove(index);
    state.clamp_selection();
    state.dirty = true;
    state.status = format!("Deleted {}", removed.name);
}

fn save_now(state: &mut TuiState, config_path: &Path) -> Result<()> {
    save_config(config_path, &state.editable)?;
    state.dirty = false;
    state.status = format!("Saved {}", config_path.display());
    Ok(())
}
