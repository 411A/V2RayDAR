use ratatui::layout::{Constraint, Layout, Rect};

#[derive(Debug, Clone, Copy)]
pub struct MainLayout {
    pub top: Rect,
    pub logs: Rect,
    pub found: Rect,
    pub config: Rect,
    pub menu: Rect,
    pub footer: Rect,
}

pub fn main(area: Rect) -> MainLayout {
    let [top, logs, found, config, menu, footer] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(7),
        Constraint::Length(8),
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
