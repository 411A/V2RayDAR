use std::time::Instant;

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
    instant_now: Instant,
) {
    state.hits = HitMap::default();
    let areas = layout::main(
        frame.area(),
        layout::uses_tokenized_endpoint(&state.editable),
    );

    if !areas.top.is_empty() {
        top::draw(
            frame,
            areas.top,
            runtime,
            runtime_config,
            state.started_at,
            instant_now,
        );
    }
    if !areas.logs.is_empty() {
        state.hits.logs_area = Some(areas.logs);
        logs_panel::draw(frame, areas.logs, runtime, &mut state.scroll.logs);
    }
    if !areas.found.is_empty() {
        state.hits.found_area = Some(areas.found);
        found_panel::draw(
            frame,
            areas.found,
            runtime,
            runtime_config.top_n,
            &mut state.scroll.found,
        );
    }
    if !areas.config.is_empty() {
        config_panel::draw(frame, areas.config, state, runtime_config, paths);
    }

    main_menu_panel::draw(frame, areas.menu, state, runtime);
    footer::draw(frame, areas.footer, state);
}
