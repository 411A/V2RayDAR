use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{RwLock, watch};

use super::{
    events::{push_editable, update_live_runtime_config},
    input_handlers::{start_input, start_new_subscription},
    state::{Action, InputMode, TuiState},
    util::save_merged,
};
use crate::{config::AppConfig, db::Database, model::RuntimeConfig};

pub fn run_action(
    state: &mut TuiState,
    action: Action,
    db: &Database,
    config_tx: &watch::Sender<AppConfig>,
    runtime_config: &Arc<RwLock<RuntimeConfig>>,
) -> Result<()> {
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
        Action::Toggle => {
            toggle_subscription(state);
            push_editable(config_tx, state);
            update_live_runtime_config(runtime_config, state);
        }
        Action::Delete => {
            delete_subscription(state);
            push_editable(config_tx, state);
            update_live_runtime_config(runtime_config, state);
        }
        Action::Save => {
            save_now(state, db)?;
            push_editable(config_tx, state);
            update_live_runtime_config(runtime_config, state);
        }
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

fn save_now(state: &mut TuiState, db: &Database) -> Result<()> {
    save_merged(db, &state.startup_editable, &state.editable)?;
    state.dirty = false;
    state.status = "Saved to data.db".to_string();
    Ok(())
}
