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
    let height = area.height;
    let config_height = if tokenized_endpoint {
        TUI_CONFIG_PANEL_ENDPOINT_HEIGHT
    } else {
        TUI_CONFIG_PANEL_HEIGHT
    };

    if height >= 38 {
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
    } else if height >= 28 {
        let top_h = 3;
        let logs_h = 3;
        let found_h = 5;
        let footer_h = 2;
        let config_h =
            config_height.min(height.saturating_sub(top_h + logs_h + found_h + footer_h + 4));
        let remaining = height.saturating_sub(top_h + logs_h + found_h + config_h + footer_h);

        let [top, logs, found, config, menu, footer] = Layout::vertical([
            Constraint::Length(top_h),
            Constraint::Length(logs_h),
            Constraint::Length(found_h),
            Constraint::Length(config_h),
            Constraint::Length(remaining.max(4)),
            Constraint::Length(footer_h),
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
    } else {
        let top_h = 2;
        let found_h = height.saturating_sub(top_h + 6).max(3);
        let footer_h = 1;
        let remaining = height.saturating_sub(top_h + found_h + footer_h);
        let menu_h = remaining.max(3);

        let [top, found, menu, footer] = Layout::vertical([
            Constraint::Length(top_h),
            Constraint::Length(found_h),
            Constraint::Length(menu_h),
            Constraint::Length(footer_h),
        ])
        .areas(area);

        MainLayout {
            top,
            logs: Rect::ZERO,
            found,
            config: Rect::ZERO,
            menu,
            footer,
        }
    }
}

pub fn uses_tokenized_endpoint(config: &crate::config::AppConfig) -> bool {
    config.sharing.enabled && should_include_token_in_url(&config.sharing.token)
}
