//! User preferences in `data.db`: the `settings` and `subscriptions` tables.
//!
//! Replaces the legacy `configs.yaml` file. Settings flatten to dotted keys
//! (`bind`, `probe.mode`, `sharing.token`, ...) holding compact JSON literals,
//! so every leaf round-trips exactly with no per-field mapping code — a newly
//! added setting persists automatically. Subscriptions carry `priority`, which
//! is both the probe ranking weight and the list order (lower runs first),
//! in the TUI and on the dashboard alike.
//!
//! Runtime-only fields (`proxy.manual_proxy_uri`, `probe.sing_box_path_auto`)
//! are `#[serde(skip)]` and never reach the database, exactly like the old
//! file writer, which stripped them before saving.

use std::{collections::HashMap, path::Path};

use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value};

use crate::{
    config::AppConfig,
    constants::{CONFIG_FILE_NAME, CONFIG_MIGRATION_NOTE},
    db::Database,
    paths::AppPaths,
};

/// Shipped defaults (settings + pre-selected subscriptions) seeding a fresh
/// database on first run. Kept as the documented example file so the seed
/// stays in sync with what users read.
const DEFAULT_CONFIG_TEMPLATE: &str = include_str!("../configs.example.yaml");

/// What [`load_or_migrate`] did, for startup logging.
#[derive(Debug)]
pub enum StartupOutcome {
    /// A legacy file was parsed into the database and neutered.
    Migrated { subscriptions: usize },
    /// No preferences existed anywhere: example defaults were seeded.
    SeededDefaults,
    /// Stored rows were loaded as-is.
    Loaded,
}

/// Resolve user preferences at startup, in order:
///
/// 1. Legacy file with real content → parse (yaml or json), store in the
///    database, then replace the file with [`CONFIG_MIGRATION_NOTE`].
///    Runs in every mode (TUI, `--no-tui`, `--once`, quiet default): one code path.
/// 2. Migration-note file → already migrated; load stored rows. If the
///    database was deleted since, re-seed defaults instead of failing.
/// 3. No file → load stored rows, or seed defaults on first run. A custom
///    `--config` path that never existed still errors like before, unless
///    its sibling database already holds settings.
pub fn load_or_migrate(paths: &AppPaths, db: &Database) -> Result<(AppConfig, StartupOutcome)> {
    if paths.config_path.exists() {
        let content = std::fs::read_to_string(&paths.config_path).with_context(|| {
            format!(
                "unable to read legacy config {}",
                paths.config_path.display()
            )
        })?;
        if content.trim() == CONFIG_MIGRATION_NOTE {
            if db.has_settings()? {
                return Ok((load_app_config(db)?, StartupOutcome::Loaded));
            }
            let config = seed_defaults(db)?;
            return Ok((config, StartupOutcome::SeededDefaults));
        }

        let config = parse_legacy_file(&paths.config_path)?;
        let subscription_count = config.subscriptions.len();
        save_app_config(db, &config)?;
        // The database commit above is the point of no return: only neuter
        // the file once every value is stored, so a crash mid-migration
        // simply retries from the intact file on the next start.
        std::fs::write(&paths.config_path, format!("{CONFIG_MIGRATION_NOTE}\n")).with_context(
            || {
                format!(
                    "migrated settings to the database but unable to neuter {}",
                    paths.config_path.display()
                )
            },
        )?;
        return Ok((
            config,
            StartupOutcome::Migrated {
                subscriptions: subscription_count,
            },
        ));
    }

    if db.has_settings()? {
        return Ok((load_app_config(db)?, StartupOutcome::Loaded));
    }
    if !paths.generated_config {
        return Err(anyhow!(
            "config file does not exist: {}; create it or omit --config to use {}",
            paths.config_path.display(),
            paths.root_dir.join(CONFIG_FILE_NAME).display()
        ));
    }
    let config = seed_defaults(db)?;
    Ok((config, StartupOutcome::SeededDefaults))
}

/// Persist the whole config (settings + subscriptions) to the database.
/// Used by the TUI, the dashboard, and the legacy migration alike.
pub fn save_app_config(db: &Database, config: &AppConfig) -> Result<()> {
    let config = persistable_config(config);
    let value =
        serde_json::to_value(&config).context("unable to serialize config to settings rows")?;
    db.save_settings(&flatten_settings(&value))?;
    db.save_subscriptions(&config.subscriptions)?;
    Ok(())
}

/// Load the whole config back, filling anything missing with current
/// defaults (no backfill step: serde defaults apply on every load) and
/// running the same validation the file loader ran.
pub fn load_app_config(db: &Database) -> Result<AppConfig> {
    let rows = db.load_settings()?;
    let value = unflatten_settings(&rows)?;
    let mut config: AppConfig =
        serde_json::from_value(value).context("stored settings failed to parse")?;
    config.subscriptions = db.load_subscriptions()?;
    crate::config::validate_config(config)
}

/// Parse a legacy yaml/json config file: same formats, same validation as
/// the old startup loader. A `token: true` still generates a token at parse
/// time; unlike the file era it needs no re-save pass — the value lands in
/// the database with everything else.
fn parse_legacy_file(path: &Path) -> Result<AppConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("unable to read {}", path.display()))?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let config: AppConfig = match extension.as_str() {
        "json" => serde_json::from_str(&content).context("invalid JSON config")?,
        "yaml" | "yml" | "" => serde_yaml::from_str(&content).context("invalid YAML config")?,
        other => {
            return Err(anyhow!(
                "unsupported config extension '.{other}'; use .yaml, .yml, or .json"
            ));
        }
    };
    crate::config::validate_config(config)
}

/// Seed a fresh database from the shipped example template.
fn seed_defaults(db: &Database) -> Result<AppConfig> {
    let config: AppConfig = serde_yaml::from_str(DEFAULT_CONFIG_TEMPLATE)
        .context("default config template is valid")?;
    let config = crate::config::validate_config(config)?;
    save_app_config(db, &config)?;
    Ok(config)
}

/// The shipped defaults, parsed and validated: the single source of truth
/// for seeds, resets, and upgrade merges. `configs.example.yaml` is the
/// default — git-tracked, embedded into the binary, no installer or network
/// involved.
pub fn embedded_defaults() -> AppConfig {
    AppConfig::default_for_first_run()
}

/// Content fingerprint of the embedded defaults file (FNV-1a hex, no
/// dependencies): `data_version` decouples "did the defaults change" from
/// "did the app version change", so patch releases with identical defaults
/// skip the merge and dev builds with edited defaults still reconcile.
pub fn defaults_data_version() -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in DEFAULT_CONFIG_TEMPLATE.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Store the current embedded defaults as the merge baseline.
pub fn store_defaults_snapshot(db: &Database) -> Result<()> {
    let defaults = embedded_defaults();
    let json = serde_json::to_string(&defaults).context("defaults serialize to JSON")?;
    db.save_defaults_snapshot(env!("CARGO_PKG_VERSION"), &defaults_data_version(), &json)
}

/// What [`reconcile_with_defaults`] changed, for startup logging.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub settings_added: usize,
    pub settings_updated: usize,
    pub subs_added: usize,
    pub snapshot_stored: bool,
}

impl ReconcileReport {
    pub const fn changed(&self) -> bool {
        self.settings_added + self.settings_updated + self.subs_added > 0
    }
}

/// Reconcile live settings against the embedded defaults (three-way merge):
/// keys the user never touched follow new defaults, keys the user customized
/// are left alone, missing keys are added, and embedded subscriptions the
/// user never had are appended by URL. A subscription present in the baseline
/// but missing from the user's list was deleted on purpose and stays gone.
/// Idempotent: re-running without newer defaults changes nothing.
pub fn reconcile_with_defaults(db: &Database, config: &mut AppConfig) -> Result<ReconcileReport> {
    let mut report = ReconcileReport::default();
    let embedded = embedded_defaults();
    let embedded_version = defaults_data_version();

    let snapshot = db.load_defaults_snapshot()?;
    let Some((_, snapshot_version, old_json)) = snapshot else {
        return legacy_reconcile(db, config, &embedded, &embedded_version, &mut report);
    };
    if snapshot_version == embedded_version {
        return Ok(report);
    }
    let Ok(old) = serde_json::from_str::<AppConfig>(&old_json) else {
        // Corrupt baseline: self-heal through the legacy path (add-only,
        // then re-anchor) instead of aborting the boot.
        return legacy_reconcile(db, config, &embedded, &embedded_version, &mut report);
    };

    let user_map = flatten_config(config)?;
    let old_map = flatten_config(&old)?;
    let new_map = flatten_config(&embedded)?;
    let mut merged = user_map;
    for (key, new_value) in &new_map {
        match (merged.get(key), old_map.get(key)) {
            (None, _) => {
                merged.insert(key.clone(), new_value.clone());
                report.settings_added += 1;
            }
            (Some(current), old) => {
                if old == Some(current) && current != new_value {
                    merged.insert(key.clone(), new_value.clone());
                    report.settings_updated += 1;
                }
            }
        }
    }
    report.subs_added = append_missing_subscriptions(
        &mut config.subscriptions,
        &embedded.subscriptions,
        &old.subscriptions,
    );

    if report.changed() {
        let value = unflatten_settings(&merged)?;
        let mut reconciled: AppConfig =
            serde_json::from_value(value).context("merged settings parse")?;
        reconciled.subscriptions = std::mem::take(&mut config.subscriptions);
        *config = crate::config::validate_config(reconciled)?;
    }
    let json = serde_json::to_string(&embedded).context("defaults serialize to JSON")?;
    db.save_defaults_snapshot(env!("CARGO_PKG_VERSION"), &embedded_version, &json)?;
    report.snapshot_stored = true;
    Ok(report)
}

/// No-baseline path (legacy database, or a seed that predates snapshots):
/// keys already defaulted via serde, so only append subscriptions the user
/// never had, then anchor the baseline. A previously-deleted default returns
/// once here — afterwards the baseline tells deleted apart from new.
fn legacy_reconcile(
    db: &Database,
    config: &mut AppConfig,
    embedded: &AppConfig,
    embedded_version: &str,
    report: &mut ReconcileReport,
) -> Result<ReconcileReport> {
    report.subs_added =
        append_missing_subscriptions(&mut config.subscriptions, &embedded.subscriptions, &[]);
    if report.subs_added > 0 {
        crate::config::canonicalize_subscriptions(&mut config.subscriptions);
    }
    let json = serde_json::to_string(embedded).context("defaults serialize to JSON")?;
    db.save_defaults_snapshot(env!("CARGO_PKG_VERSION"), embedded_version, &json)?;
    report.snapshot_stored = true;
    Ok(std::mem::take(report))
}

/// Append embedded subscriptions whose URL the user never had. A URL present
/// in the baseline but missing from the user's list was deleted on purpose
/// and is not resurrected. Returns the number appended.
fn append_missing_subscriptions(
    user: &mut Vec<crate::config::SubscriptionSource>,
    embedded: &[crate::config::SubscriptionSource],
    baseline: &[crate::config::SubscriptionSource],
) -> usize {
    let mut added = 0;
    for source in embedded {
        if user.iter().any(|own| own.url == source.url) {
            continue;
        }
        if baseline.iter().any(|old| old.url == source.url) {
            continue;
        }
        user.push(source.clone());
        added += 1;
    }
    added
}

/// Restore non-subscription settings from the embedded defaults and re-anchor
/// the merge baseline. Callers keep `config.subscriptions` as they were.
pub fn reset_to_embedded_defaults(db: &Database, config: &mut AppConfig) -> Result<()> {
    let embedded = embedded_defaults();
    let subscriptions = std::mem::take(&mut config.subscriptions);
    *config = embedded;
    config.subscriptions = subscriptions;
    store_defaults_snapshot(db)
}

fn flatten_config(config: &AppConfig) -> Result<HashMap<String, String>> {
    let value = serde_json::to_value(config).context("settings serialize to JSON")?;
    let Value::Object(map) = value else {
        anyhow::bail!("settings serialize to a JSON object");
    };
    let mut rows = Vec::new();
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    for key in keys {
        if key == "subscriptions" {
            continue;
        }
        flatten_value(key, &map[key], &mut rows);
    }
    rows.sort();
    Ok(rows.into_iter().collect())
}

/// Drop runtime-only state before persisting: an auto-detected sing-box path
/// re-resolves every start, and a manual proxy pin is live-only by design.
fn persistable_config(config: &AppConfig) -> AppConfig {
    let mut config = config.clone();
    if config.probe.sing_box_path_auto {
        config.probe.sing_box_path.clear();
        config.probe.sing_box_path_auto = false;
    }
    config
}

/// Flatten a config JSON object to dotted `settings` rows, skipping the
/// `subscriptions` array (it has its own table with ordering).
fn flatten_settings(value: &Value) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    if let Value::Object(map) = value {
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        for key in keys {
            if key == "subscriptions" {
                continue;
            }
            flatten_value(key, &map[key], &mut rows);
        }
    }
    rows.sort();
    rows
}

fn flatten_value(prefix: &str, value: &Value, rows: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                flatten_value(&format!("{prefix}.{key}"), &map[key], rows);
            }
        }
        leaf => rows.push((
            prefix.to_string(),
            serde_json::to_string(leaf).unwrap_or_default(),
        )),
    }
}

/// Rebuild the nested config JSON from `settings` rows. Unknown keys survive
/// the round trip untouched and are ignored by deserialization, so a newer
/// database never breaks an older binary.
fn unflatten_settings(rows: &HashMap<String, String>) -> Result<Value> {
    let mut root = Map::new();
    let mut keys: Vec<&String> = rows.keys().collect();
    keys.sort();
    for key in keys {
        let leaf: Value = serde_json::from_str(&rows[key])
            .with_context(|| format!("stored setting {key} failed to parse"))?;
        let mut node = &mut root;
        let parts: Vec<&str> = key.split('.').collect();
        for part in &parts[..parts.len() - 1] {
            node = node
                .entry((*part).to_string())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .with_context(|| format!("stored setting {key} collides with a section"))?;
        }
        if let Some(last) = parts.last() {
            node.insert((*last).to_string(), leaf);
        }
    }
    Ok(Value::Object(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_temp_db(name: &str) -> (Database, TempGuard) {
        let dir = std::env::temp_dir().join(format!(
            "v2raydar-settings-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("temp dir can be created");
        let db = Database::open(&dir.join("data.db")).expect("db opens");
        (db, TempGuard(dir))
    }

    struct TempGuard(std::path::PathBuf);
    impl Drop for TempGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn legacy_paths(name: &str, file: &str) -> (AppPaths, TempGuard) {
        let dir = std::env::temp_dir().join(format!(
            "v2raydar-migrate-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let data_dir = dir.join(crate::constants::APP_DATA_DIR_NAME);
        std::fs::create_dir_all(&data_dir).expect("temp dir can be created");
        let config_path = dir.join(file);
        let paths = AppPaths::from_config_override(config_path);
        (paths, TempGuard(dir))
    }

    fn assert_configs_equal(expected: &AppConfig, actual: &AppConfig) {
        let mut expected = expected.clone();
        let mut actual = actual.clone();
        // Save-time transforms never persist; normalize before comparing.
        for config in [&mut expected, &mut actual] {
            config.probe.sing_box_path.clear();
            config.probe.sing_box_path_auto = false;
            config.proxy.manual_proxy_uri = None;
        }
        let expected_value = serde_json::to_value(&expected).expect("serializes");
        let actual_value = serde_json::to_value(&actual).expect("serializes");
        assert_eq!(expected_value, actual_value);
        assert_eq!(expected.subscriptions, actual.subscriptions);
    }

    #[test]
    fn round_trip_preserves_every_setting_and_subscription() {
        let (db, _guard) = open_temp_db("roundtrip");
        let mut config = AppConfig::default_for_first_run();
        config.top_n = 25;
        config.probe.batch_size = None;
        config.probe.process_concurrency = Some(3);
        config.probe.accepted_statuses = vec![200];
        config.sharing.token = "secret".to_string();
        config.emergency_config = Some("vless://x@y:443".to_string());
        config.proxy.manual_proxy_uri = Some("live-only".to_string());
        config.probe.sing_box_path = "/tmp/sing-box".to_string();
        config.probe.sing_box_path_auto = true;
        save_app_config(&db, &config).expect("saves");

        let loaded = load_app_config(&db).expect("loads");
        assert_configs_equal(&config, &loaded);
        // Stripped before saving, like the old file writer did.
        assert_eq!(loaded.probe.sing_box_path, "");
        assert_eq!(loaded.proxy.manual_proxy_uri, None);
    }

    #[test]
    fn legacy_yaml_migrates_values_then_neuters_file() {
        let (db, _db_guard) = open_temp_db("yaml");
        let (paths, _dir_guard) = legacy_paths("yaml", "configs.yaml");
        std::fs::write(
            &paths.config_path,
            "bind: 127.0.0.1:28080\ntop_n: 7\nsharing:\n  token: true\nsubscriptions:\n  - {name: b, url: https://example.com/b.txt, enabled: false, priority: 9}\n  - {name: a, url: https://example.com/a.txt, enabled: true, priority: 1}\n  - {name: c, url: https://example.com/c.txt, enabled: true, priority: 1}\n",
        )
        .expect("legacy file writes");

        let (config, outcome) = load_or_migrate(&paths, &db).expect("migrates");
        assert!(matches!(
            outcome,
            StartupOutcome::Migrated { subscriptions: 3 }
        ));
        assert_eq!(config.bind.port(), 28080);
        assert_eq!(config.top_n, 7);
        assert!(!config.sharing.token.is_empty(), "token:true generates");
        // Migration converges to the single order invariant at once: the
        // first return already serves priority order densely renumbered, so
        // dashboard positions and database rows agree from the first boot.
        assert_eq!(names(&config), ["a", "c", "b"]);
        assert_eq!(
            config
                .subscriptions
                .iter()
                .map(|s| s.priority)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(!config.subscriptions[2].enabled);

        let note = std::fs::read_to_string(&paths.config_path).expect("note reads");
        assert_eq!(note.trim(), CONFIG_MIGRATION_NOTE);
        // A second start loads from the database without touching the note.
        let (reloaded, outcome) = load_or_migrate(&paths, &db).expect("reloads");
        assert!(matches!(outcome, StartupOutcome::Loaded));
        assert_eq!(
            names(&reloaded),
            ["a", "c", "b"],
            "priority is the order (lower first, ties keep file order)"
        );
        assert!(!reloaded.subscriptions[2].enabled);
        assert_eq!(reloaded.top_n, 7);
        assert_eq!(reloaded.sharing.token, config.sharing.token);
        // A third start is a stable no-op.
        let (again, outcome) = load_or_migrate(&paths, &db).expect("reloads");
        assert!(matches!(outcome, StartupOutcome::Loaded));
        assert_configs_equal(&reloaded, &again);
    }

    #[test]
    fn legacy_json_migrates() {
        let (db, _db_guard) = open_temp_db("json");
        let (paths, _dir_guard) = legacy_paths("json", "custom.json");
        std::fs::write(
            &paths.config_path,
            r#"{"top_n": 3, "subscriptions": [{"name": "j", "url": "https://example.com/j.txt"}]}"#,
        )
        .expect("legacy file writes");

        let (config, outcome) = load_or_migrate(&paths, &db).expect("migrates");
        assert!(matches!(
            outcome,
            StartupOutcome::Migrated { subscriptions: 1 }
        ));
        assert_eq!(config.top_n, 3);
        assert_eq!(config.subscriptions[0].name, "j");
        // Schema defaults fill what JSON omitted.
        assert!(config.subscriptions[0].enabled);
        assert_eq!(
            std::fs::read_to_string(&paths.config_path)
                .expect("note reads")
                .trim(),
            CONFIG_MIGRATION_NOTE
        );
    }

    #[test]
    fn corrupt_legacy_file_fails_without_neutering() {
        let (db, _db_guard) = open_temp_db("corrupt");
        let (paths, _dir_guard) = legacy_paths("corrupt", "configs.yaml");
        std::fs::write(&paths.config_path, "top_n: [unclosed\n").expect("legacy file writes");

        let error = load_or_migrate(&paths, &db).expect_err("corrupt file must fail");
        assert!(error.to_string().contains("invalid YAML config"));
        assert_eq!(
            std::fs::read_to_string(&paths.config_path).expect("file reads"),
            "top_n: [unclosed\n",
            "a failed migration must leave the file intact for repair"
        );
        assert!(!db.has_settings().expect("db readable"));
    }

    #[test]
    fn missing_first_run_file_seeds_example_defaults() {
        let (db, _db_guard) = open_temp_db("seed");
        let dir = std::env::temp_dir().join(format!(
            "v2raydar-seed-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let data_dir = dir.join(crate::constants::APP_DATA_DIR_NAME);
        std::fs::create_dir_all(&data_dir).expect("temp dir can be created");
        let paths = AppPaths::from_root(data_dir, true);
        assert!(!paths.config_path.exists());

        let (config, outcome) = load_or_migrate(&paths, &db).expect("seeds");
        assert!(matches!(outcome, StartupOutcome::SeededDefaults));
        assert!(!config.subscriptions.is_empty(), "example sources seed");
        assert_eq!(config.top_n, 10);
        assert!(db.has_settings().expect("db readable"));
        // Seeding writes no legacy file: fresh installs stay file-free.
        assert!(!paths.config_path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_custom_file_still_errors_without_stored_settings() {
        let (db, _db_guard) = open_temp_db("custom-missing");
        let (paths, _dir_guard) = legacy_paths("custom-missing", "custom.yaml");
        assert!(!paths.config_path.exists());
        assert!(!paths.generated_config);

        let error = load_or_migrate(&paths, &db).expect_err("must error");
        assert!(error.to_string().contains("config file does not exist"));
    }

    #[test]
    fn cache_clean_keeps_user_data() {
        let (db, _db_guard) = open_temp_db("clean");
        let config = AppConfig::default_for_first_run();
        save_app_config(&db, &config).expect("saves");

        db.delete_all().expect("cache clean runs");
        assert!(db.has_settings().expect("db readable"));
        let loaded = load_app_config(&db).expect("loads");
        assert_configs_equal(&config, &loaded);
    }

    fn names(config: &AppConfig) -> Vec<&str> {
        config
            .subscriptions
            .iter()
            .map(|s| s.name.as_str())
            .collect()
    }

    #[test]
    fn subscription_order_is_priority_order() {
        let (db, _db_guard) = open_temp_db("reorder");
        let mut config = AppConfig::default_for_first_run();
        // Scramble the Vec but keep each entry's priority: reloads must come
        // back sorted by priority, not by insertion order.
        config.subscriptions.reverse();
        save_app_config(&db, &config).expect("saves");

        let loaded = load_app_config(&db).expect("loads");
        let priorities: Vec<u32> = loaded.subscriptions.iter().map(|s| s.priority).collect();
        let mut sorted = priorities.clone();
        sorted.sort_unstable();
        assert_eq!(priorities, sorted, "priority is the list order");
        assert_eq!(loaded.subscriptions.len(), config.subscriptions.len());
    }

    #[test]
    fn equal_priorities_keep_insertion_order() {
        let (db, _db_guard) = open_temp_db("ties");
        let mut config = AppConfig::default_for_first_run();
        config.subscriptions.truncate(3);
        for source in &mut config.subscriptions {
            source.priority = 100;
        }
        let names: Vec<String> = config
            .subscriptions
            .iter()
            .map(|s| s.name.clone())
            .collect();
        save_app_config(&db, &config).expect("saves");

        let loaded = load_app_config(&db).expect("loads");
        assert_eq!(
            loaded
                .subscriptions
                .iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>(),
            names,
            "ties keep insertion order"
        );
    }

    #[test]
    fn legacy_gaps_converge_to_dense_priority_order() {
        let (db, _db_guard) = open_temp_db("gaps");
        let mut config = AppConfig::default_for_first_run();
        config.subscriptions.truncate(3);
        config.subscriptions[0].priority = 30;
        config.subscriptions[1].priority = 10;
        config.subscriptions[2].priority = 20;
        save_app_config(&db, &config).expect("saves");

        // The database serves priority order while the saver kept vec order:
        // loads converge so dashboard positions and mutation indices agree.
        let loaded = load_app_config(&db).expect("loads");
        let priorities: Vec<u32> = loaded.subscriptions.iter().map(|s| s.priority).collect();
        assert_eq!(priorities, vec![1, 2, 3]);
        assert!(
            loaded
                .subscriptions
                .windows(2)
                .all(|w| w[0].priority < w[1].priority),
            "vec order is priority order"
        );
    }

    fn test_source(name: &str, url: &str) -> crate::config::SubscriptionSource {
        crate::config::SubscriptionSource {
            name: name.to_string(),
            url: url.to_string(),
            enabled: true,
            priority: 1,
        }
    }

    fn store_old_snapshot(db: &Database, old: &AppConfig) {
        let json = serde_json::to_string(old).expect("old serializes");
        db.save_defaults_snapshot("0.0.1", "old-fingerprint", &json)
            .expect("snapshot stores");
    }

    #[test]
    fn untouched_settings_follow_new_defaults_customized_are_kept() {
        let (db, _db_guard) = open_temp_db("merge");
        let embedded = embedded_defaults();
        assert!(
            !embedded.subscriptions.is_empty(),
            "example defaults carry subscriptions"
        );
        let keep_url = embedded.subscriptions[0].url.clone();

        // Pretend the previous defaults: top_n differed, subs had one extra.
        let mut old = embedded.clone();
        old.top_n = embedded.top_n.saturating_add(100);
        old.subscriptions = vec![
            test_source("keep", &keep_url),
            test_source("gone", "https://test.invalid/gone.txt"),
        ];

        // The user never touched top_n (= old default) but customized ping
        // and deleted the gone subscription while adding their own.
        let mut user = old.clone();
        user.ping_seconds = embedded.ping_seconds.saturating_add(61);
        user.subscriptions = vec![
            test_source("keep", &keep_url),
            test_source("mine", "https://test.invalid/mine.txt"),
        ];
        store_old_snapshot(&db, &old);

        let report = reconcile_with_defaults(&db, &mut user).expect("reconciles");
        assert_eq!(
            user.top_n, embedded.top_n,
            "untouched key follows the new default"
        );
        assert_eq!(
            user.ping_seconds,
            embedded.ping_seconds.saturating_add(61),
            "customized key is left alone"
        );
        assert_eq!(report.settings_updated, 1);
        let urls: Vec<&str> = user.subscriptions.iter().map(|s| s.url.as_str()).collect();
        assert!(urls.contains(&keep_url.as_str()), "kept sub stays");
        assert!(
            urls.contains(&"https://test.invalid/mine.txt"),
            "user sub stays"
        );
        assert!(
            !urls.contains(&"https://test.invalid/gone.txt"),
            "deleted default is not resurrected"
        );
        for source in &embedded.subscriptions {
            assert!(
                urls.contains(&source.url.as_str()),
                "new default sub is appended"
            );
        }
        assert_eq!(report.subs_added, embedded.subscriptions.len() - 1);
        assert!(report.snapshot_stored);
        assert!(report.changed());

        // Second run with the stored baseline is a no-op.
        let again = reconcile_with_defaults(&db, &mut user).expect("reconciles");
        assert_eq!(again, ReconcileReport::default());
    }

    #[test]
    fn legacy_database_without_snapshot_only_appends() {
        let (db, _db_guard) = open_temp_db("legacy-merge");
        let embedded = embedded_defaults();
        let mut user = embedded.clone();
        user.top_n = user.top_n.saturating_add(5);
        user.subscriptions.truncate(1);
        let kept = user.subscriptions[0].url.clone();
        // No snapshot stored: first contact with the new system.

        let report = reconcile_with_defaults(&db, &mut user).expect("reconciles");
        assert_eq!(
            user.top_n,
            embedded.top_n.saturating_add(5),
            "legacy values are treated as custom, never overwritten"
        );
        for source in &embedded.subscriptions {
            assert!(
                user.subscriptions.iter().any(|s| s.url == source.url),
                "missing default sub is appended"
            );
        }
        assert!(user.subscriptions.iter().any(|s| s.url == kept));
        assert_eq!(report.subs_added, embedded.subscriptions.len() - 1);
        assert!(report.snapshot_stored);
        // Baseline anchored now: the next run is a no-op.
        let again = reconcile_with_defaults(&db, &mut user).expect("reconciles");
        assert_eq!(again, ReconcileReport::default());
    }

    #[test]
    fn reset_restores_settings_keeps_subscriptions_and_reanchors() {
        let (db, _db_guard) = open_temp_db("reset");
        let embedded = embedded_defaults();
        let mut user = embedded.clone();
        user.top_n = user.top_n.saturating_add(5);
        user.subscriptions = vec![test_source("mine", "https://test.invalid/mine.txt")];

        reset_to_embedded_defaults(&db, &mut user).expect("resets");
        assert_eq!(user.top_n, embedded.top_n);
        assert_eq!(user.subscriptions.len(), 1);
        assert_eq!(user.subscriptions[0].url, "https://test.invalid/mine.txt");
        let snapshot = db.load_defaults_snapshot().expect("snapshot reads");
        let (_, version, _) = snapshot.expect("snapshot stored");
        assert_eq!(version, defaults_data_version());
    }

    #[test]
    fn data_version_is_stable_hex() {
        let first = defaults_data_version();
        let second = defaults_data_version();
        assert_eq!(first, second);
        assert_eq!(first.len(), 16);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
