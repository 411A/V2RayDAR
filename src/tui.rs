mod action_handlers;
mod config_editor;
mod config_panel;
mod draw;
mod events;
mod firewall;
mod footer;
mod found_panel;
mod input_handlers;
mod layout;
mod logs_panel;
mod main_menu_panel;
mod open_config;
mod setup;
pub(crate) mod state;
mod subscriptions_panel;
mod top;
mod util;
mod view;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use tokio::sync::RwLock;

use crate::{
    config::AppConfig,
    constants::{TUI_FRAME_INTERVAL, TUI_INPUT_POLL_INTERVAL, TUI_MAX_EVENTS_PER_FRAME},
    model::{RuntimeConfig, RuntimeState},
    paths::AppPaths,
};

use self::{
    events::{EventResult, handle_key, handle_mouse},
    state::TuiState,
    view::RuntimeView,
};

pub async fn run(
    initial_config: AppConfig,
    paths: AppPaths,
    state: Arc<RwLock<RuntimeState>>,
    runtime_config: Arc<RwLock<RuntimeConfig>>,
) -> Result<()> {
    enable_raw_mode()?;
    let mut terminal = ratatui::try_init()?;
    execute!(std::io::stdout(), EnableMouseCapture)?;
    let mut tui = TuiState::new(initial_config);
    let mut next_frame = Instant::now();

    let result: Result<()> = loop {
        let now = Instant::now();
        if now >= next_frame {
            let config = runtime_config.read().await.clone();
            let runtime = {
                let runtime = state.read().await;
                RuntimeView::from_state(&runtime, &config)
            };
            if let Err(err) =
                terminal.draw(|frame| draw::draw(frame, &mut tui, &runtime, &config, &paths))
            {
                break Err(err.into());
            }

            next_frame = now + TUI_FRAME_INTERVAL;
        }

        let poll_timeout = next_frame
            .saturating_duration_since(Instant::now())
            .min(TUI_INPUT_POLL_INTERVAL);
        if event::poll(poll_timeout).unwrap_or(false)
            && matches!(drain_events(&mut tui, &paths)?, EventResult::Quit)
        {
            break Ok(());
        }
    };

    let restore_result = restore_terminal();
    result.and(restore_result)
}

fn drain_events(tui: &mut TuiState, paths: &AppPaths) -> Result<EventResult> {
    for _ in 0..TUI_MAX_EVENTS_PER_FRAME {
        match event::read() {
            Ok(Event::Key(key)) => {
                if matches!(handle_key(tui, key, &paths.config_path)?, EventResult::Quit) {
                    return Ok(EventResult::Quit);
                }
            }
            Ok(Event::Mouse(mouse)) => {
                if matches!(
                    handle_mouse(tui, mouse, &paths.config_path)?,
                    EventResult::Quit
                ) {
                    return Ok(EventResult::Quit);
                }
            }
            Ok(_) => {}
            Err(err) => return Err(err.into()),
        }

        if !event::poll(Duration::ZERO).unwrap_or(false) {
            break;
        }
    }

    Ok(EventResult::Continue)
}

pub async fn run_sing_box_setup(config: &mut AppConfig, paths: &AppPaths) -> Result<()> {
    setup::run(config, paths).await
}

pub fn restore_terminal() -> Result<()> {
    execute!(std::io::stdout(), DisableMouseCapture)?;
    disable_raw_mode()?;
    ratatui::restore();
    Ok(())
}
