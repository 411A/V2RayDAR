use ratatui::layout::{Constraint, Layout, Rect};

#[derive(Debug, Clone, Copy)]
pub struct MainLayout {
    pub top: Rect,
    pub config: Rect,
    pub logs: Rect,
    pub found: Rect,
    pub menu: Rect,
    pub footer: Rect,
}

pub fn main(area: Rect) -> MainLayout {
    let [top, config, middle, menu, footer] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(6),
        Constraint::Length(12),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let [logs, found] =
        Layout::vertical([Constraint::Length(5), Constraint::Fill(1)]).areas(middle);

    MainLayout {
        top,
        config,
        logs,
        found,
        menu,
        footer,
    }
}
