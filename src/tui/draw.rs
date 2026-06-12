use ratatui::Frame;

use crate::{model::RuntimeConfig, paths::AppPaths};

use super::{
    config_panel, footer, found_panel, layout, logs_panel, main_menu_panel,
    state::{HitMap, TuiState},
    top,
    view::RuntimeView,
};

pub fn draw(
    frame: &mut Frame<'_>,
    state: &mut TuiState,
    runtime: &RuntimeView,
    runtime_config: &RuntimeConfig,
    paths: &AppPaths,
) {
    state.hits = HitMap::default();
    let areas = layout::main(
        frame.area(),
        layout::uses_tokenized_endpoint(&state.editable),
    );
    top::draw(frame, areas.top, runtime, runtime_config, state.started_at);
    logs_panel::draw(frame, areas.logs, runtime);
    found_panel::draw(frame, areas.found, runtime, runtime_config.top_n);
    config_panel::draw(frame, areas.config, state, runtime_config, paths);

    main_menu_panel::draw(frame, areas.menu, state, runtime);
    footer::draw(frame, areas.footer, state);
}
