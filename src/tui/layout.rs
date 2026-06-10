use ratatui::layout::{Constraint, Layout, Rect};

use crate::{
    config::should_include_token_in_url,
    constants::{TUI_CONFIG_PANEL_ENDPOINT_HEIGHT, TUI_CONFIG_PANEL_HEIGHT},
};

#[derive(Debug, Clone, Copy)]
pub struct MainLayout {
    pub top: Rect,
    pub logs: Rect,
    pub found: Rect,
    pub config: Rect,
    pub menu: Rect,
    pub footer: Rect,
}

pub fn main(area: Rect, tokenized_endpoint: bool) -> MainLayout {
    let config_height = if tokenized_endpoint {
        TUI_CONFIG_PANEL_ENDPOINT_HEIGHT
    } else {
        TUI_CONFIG_PANEL_HEIGHT
    };
    let [top, logs, found, config, menu, footer] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(7),
        Constraint::Length(config_height),
        Constraint::Fill(1),
        Constraint::Length(2),
    ])
    .areas(area);

    MainLayout {
        top,
        logs,
        found,
        config,
        menu,
        footer,
    }
}

pub fn uses_tokenized_endpoint(config: &crate::config::AppConfig) -> bool {
    config.sharing.enabled && should_include_token_in_url(&config.sharing.token)
}
