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
mod state;
mod subscriptions_panel;
mod top;
mod util;

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
    model::{RuntimeConfig, RuntimeState},
    paths::AppPaths,
};

use self::{
    events::{EventResult, handle_key, handle_mouse},
    state::TuiState,
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
    let mut last_tick = Instant::now();

    let result: Result<()> = loop {
        let runtime = state.read().await.clone();
        let config = runtime_config.read().await.clone();
        if let Err(err) =
            terminal.draw(|frame| draw::draw(frame, &mut tui, &runtime, &config, &paths))
        {
            break Err(err.into());
        }

        let timeout = Duration::from_millis(150).saturating_sub(last_tick.elapsed());
        if event::poll(timeout).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) => {
                    let handled = handle_key(&mut tui, key, &paths.config_path);
                    if matches!(handled?, EventResult::Quit) {
                        break Ok(());
                    }
                }
                Ok(Event::Mouse(mouse)) => {
                    let handled = handle_mouse(&mut tui, mouse, &paths.config_path);
                    if matches!(handled?, EventResult::Quit) {
                        break Ok(());
                    }
                }
                Ok(_) => {}
                Err(err) => break Err(err.into()),
            }
        }

        if last_tick.elapsed() >= Duration::from_secs(1) {
            last_tick = Instant::now();
        }
    };

    let restore_result = restore_terminal();
    result.and(restore_result)
}

fn restore_terminal() -> Result<()> {
    execute!(std::io::stdout(), DisableMouseCapture)?;
    disable_raw_mode()?;
    ratatui::restore();
    Ok(())
}
