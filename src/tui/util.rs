use anyhow::Result;

use crate::{
    config::AppConfig,
    constants::{BYTE_UNITS, BYTES_PER_UNIT},
    db::Database,
};

/// Load the stored config, merge with TUI edits, and save back.
///
/// Same 3-way rule as the file era: fields the TUI changed (vs. startup)
/// win; untouched fields reload from the database so dashboard mutations
/// made meanwhile are never overwritten.
pub fn save_merged(db: &Database, startup: &AppConfig, tui: &AppConfig) -> Result<()> {
    let merged = crate::settings::load_app_config(db).map_or_else(
        |_| tui.clone(),
        |stored| merge_for_save(startup, tui, &stored),
    );
    crate::settings::save_app_config(db, &merged)
}

/// Merge in-memory (TUI) edits with stored values.
///
/// For each field: if the TUI changed it (differs from startup), keep the TUI value.
/// Otherwise pick up the stored value so concurrent dashboard edits are never overwritten.
pub fn merge_for_save(startup: &AppConfig, tui: &AppConfig, stored: &AppConfig) -> AppConfig {
    let mut merged = stored.clone();
    merge_top_level(&mut merged, startup, tui);
    merge_section_sharing(&mut merged, startup, tui);
    merge_section_proxy(&mut merged, startup, tui);
    merge_section_probe(&mut merged, startup, tui);
    merged.subscriptions.clone_from(&tui.subscriptions);
    merged
}

fn merge_top_level(merged: &mut AppConfig, startup: &AppConfig, tui: &AppConfig) {
    macro_rules! pick {
        ($field:ident) => {
            if tui.$field != startup.$field {
                merged.$field.clone_from(&tui.$field);
            }
        };
        (copy $field:ident) => {
            if tui.$field != startup.$field {
                merged.$field = tui.$field;
            }
        };
    }
    pick!(copy bind);
    pick!(copy top_n);
    pick!(copy refresh_seconds);
    pick!(copy ping_seconds);
    pick!(copy encoded_subscription);
    pick!(copy prioritize_stability);
    pick!(copy return_configs_asap);
    pick!(copy scan_all_configs);
    pick!(copy fetch_timeout_ms);
    pick!(copy fetch_concurrency);
    pick!(copy max_subscription_bytes);
    pick!(copy use_cache_only);
    pick!(emergency_config);
    pick!(copy clean_offlines_after_days);
}

fn merge_section_sharing(merged: &mut AppConfig, startup: &AppConfig, tui: &AppConfig) {
    macro_rules! pick {
        ($field:ident) => {
            if tui.sharing.$field != startup.sharing.$field {
                merged.sharing.$field.clone_from(&tui.sharing.$field);
            }
        };
        (copy $field:ident) => {
            if tui.sharing.$field != startup.sharing.$field {
                merged.sharing.$field = tui.sharing.$field;
            }
        };
    }
    pick!(copy enabled);
    pick!(copy require_token);
    pick!(token);
}

fn merge_section_proxy(merged: &mut AppConfig, startup: &AppConfig, tui: &AppConfig) {
    macro_rules! pick {
        ($field:ident) => {
            if tui.proxy.$field != startup.proxy.$field {
                merged.proxy.$field.clone_from(&tui.proxy.$field);
            }
        };
        (copy $field:ident) => {
            if tui.proxy.$field != startup.proxy.$field {
                merged.proxy.$field = tui.proxy.$field;
            }
        };
    }
    pick!(copy enabled);
    pick!(copy port);
    pick!(copy discoverable);
    pick!(copy rotating_proxy);
    pick!(health_check_url);
    pick!(copy health_check_interval_seconds);
}

fn merge_section_probe(merged: &mut AppConfig, startup: &AppConfig, tui: &AppConfig) {
    macro_rules! pick {
        ($field:ident) => {
            if tui.probe.$field != startup.probe.$field {
                merged.probe.$field.clone_from(&tui.probe.$field);
            }
        };
        (copy $field:ident) => {
            if tui.probe.$field != startup.probe.$field {
                merged.probe.$field = tui.probe.$field;
            }
        };
    }
    pick!(copy mode);
    if tui.probe.sing_box_path != startup.probe.sing_box_path {
        merged
            .probe
            .sing_box_path
            .clone_from(&tui.probe.sing_box_path);
        merged.probe.sing_box_path_auto = tui.probe.sing_box_path_auto;
    }
    pick!(copy connect_timeout_ms);
    pick!(copy active_timeout_ms);
    pick!(copy startup_timeout_ms);
    pick!(copy concurrency);
    pick!(copy batch_size);
    pick!(copy process_concurrency);
    pick!(test_url);
    pick!(accepted_statuses);
    pick!(download_url);
    pick!(copy download_bytes_limit);
}

pub fn human_bytes(bytes: u64) -> String {
    let mut value = u64_to_f64(bytes);
    let mut unit = 0_usize;
    while value >= BYTES_PER_UNIT && unit < BYTE_UNITS.len() - 1 {
        value /= BYTES_PER_UNIT;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", BYTE_UNITS[unit])
    } else {
        format!("{value:.2} {}", BYTE_UNITS[unit])
    }
}

#[allow(clippy::cast_precision_loss)]
const fn u64_to_f64(value: u64) -> f64 {
    value as f64
}

pub const fn bool_text(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

/// Draw a ratatui `Scrollbar` inside the block's inner area.
///
/// `visible_rows` = actual content rows visible (excluding borders & headers).
/// `invert` = true when offset=0 means newest/bottom content.
pub fn draw_scrollbar(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    total_items: usize,
    visible_rows: usize,
    scroll_offset: usize,
    invert: bool,
) {
    use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

    if area.height < 3 || visible_rows == 0 {
        return;
    }
    let inner_height = area.height.saturating_sub(2) as usize;
    if total_items <= visible_rows || inner_height < 3 {
        return;
    }

    // The scrollbar renders in the block's inner area (between borders).
    // vertical margin=1 strips top/bottom borders; horizontal=0 keeps full width.
    let inner = area.inner(ratatui::layout::Margin {
        vertical: 1,
        horizontal: 0,
    });

    // Map scroll_offset → position for the scrollbar.
    // scroll_offset ranges 0..max_scroll where max_scroll = total_items - visible_rows.
    // Ratatui maps position = content_length - 1 to the very bottom of the track.
    let max_scroll = total_items.saturating_sub(visible_rows);
    let last_position = total_items.saturating_sub(1);
    let position = (scroll_offset.min(max_scroll) * last_position)
        .checked_div(max_scroll)
        .map_or(0, |raw| {
            if invert {
                last_position.saturating_sub(raw)
            } else {
                raw
            }
        });

    let mut state = ScrollbarState::new(total_items)
        .position(position)
        .viewport_content_length(1);

    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight).thumb_symbol("▣"),
        inner,
        &mut state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SubscriptionSource;

    struct TempGuard(std::path::PathBuf);
    impl Drop for TempGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn open_temp_db(name: &str) -> (Database, TempGuard) {
        let dir = std::env::temp_dir().join(format!(
            "v2raydar-merge-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("temp dir can be created");
        let db = Database::open(&dir.join("data.db")).expect("db opens");
        (db, TempGuard(dir))
    }

    #[test]
    fn merge_prefers_tui_edits_and_keeps_stored_untouched() {
        let startup = AppConfig::default_for_first_run();
        let mut tui = startup.clone();
        tui.top_n = 25;
        let mut stored = startup.clone();
        stored.refresh_seconds = 60;

        let merged = merge_for_save(&startup, &tui, &stored);

        assert_eq!(merged.top_n, 25, "TUI edit wins");
        assert_eq!(merged.refresh_seconds, 60, "stored edit survives");
    }

    #[test]
    fn merge_takes_whole_subscription_list_from_tui() {
        let startup = AppConfig::default_for_first_run();
        let mut tui = startup.clone();
        tui.subscriptions = vec![SubscriptionSource {
            name: "tui".to_string(),
            url: "https://example.com/tui.txt".to_string(),
            enabled: true,
            priority: 1,
        }];
        let mut stored = startup.clone();
        stored.subscriptions = vec![SubscriptionSource {
            name: "stored".to_string(),
            url: "https://example.com/stored.txt".to_string(),
            enabled: true,
            priority: 2,
        }];

        let merged = merge_for_save(&startup, &tui, &stored);

        assert_eq!(merged.subscriptions, tui.subscriptions);
    }

    #[test]
    fn save_merged_keeps_concurrent_dashboard_edits() {
        let (db, _guard) = open_temp_db("save-merged");
        let startup = AppConfig::default_for_first_run();
        crate::settings::save_app_config(&db, &startup).expect("seeds");
        // A dashboard mutation lands straight in the database meanwhile.
        let mut stored = crate::settings::load_app_config(&db).expect("loads");
        stored.refresh_seconds = 60;
        crate::settings::save_app_config(&db, &stored).expect("dashboard saves");

        // The TUI touched only top_n.
        let mut tui = startup.clone();
        tui.top_n = 25;
        save_merged(&db, &startup, &tui).expect("saves");

        let reloaded = crate::settings::load_app_config(&db).expect("reloads");
        assert_eq!(reloaded.top_n, 25);
        assert_eq!(reloaded.refresh_seconds, 60);
    }
}
