use ratatui::Frame;

use crate::{
    model::{RuntimeConfig, RuntimeState},
    paths::AppPaths,
};

use super::{
    config_panel, footer, found_panel, layout, logs_panel, main_menu_panel, state::TuiState, top,
};

pub fn draw(
    frame: &mut Frame<'_>,
    state: &mut TuiState,
    runtime: &RuntimeState,
    runtime_config: &RuntimeConfig,
    paths: &AppPaths,
) {
    state.hits = Default::default();
    let areas = layout::main(frame.area());
    top::draw(frame, areas.top, runtime, runtime_config);
    config_panel::draw(frame, areas.config, state, runtime_config, paths);
    logs_panel::draw(frame, areas.logs, runtime);
    found_panel::draw(frame, areas.found, runtime, runtime_config.top_n);

    main_menu_panel::draw(frame, areas.menu, state);
    footer::draw(frame, areas.footer, state);
}
