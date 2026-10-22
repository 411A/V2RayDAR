use std::{collections::HashSet, path::Path, sync::Mutex};

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use crate::model::{Candidate, RankedConfig};

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database at {}", path.display()))?;
        restrict_file_permissions(path);

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;",
        )?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS configs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                dedup_key TEXT NOT NULL UNIQUE,
                uri TEXT NOT NULL,
                source TEXT NOT NULL,
                priority INTEGER NOT NULL DEFAULT 100,
                protocol TEXT NOT NULL,
                name TEXT NOT NULL,
                endpoint_host TEXT NOT NULL,
                endpoint_port INTEGER NOT NULL,
                reachable INTEGER NOT NULL DEFAULT 0,
                validation TEXT NOT NULL DEFAULT '',
                latency_ms INTEGER,
                http_status INTEGER,
                download_mbps REAL,
                download_bytes INTEGER,
                country_code TEXT,
                stability_count INTEGER NOT NULL DEFAULT 0,
                last_online TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_configs_last_online ON configs(last_online);
            -- The ranking query (reachable = 1 ORDER BY stability_count DESC,
            -- latency_ms ASC) ran a full scan and a sort of the whole table on
            -- every cache-only refresh and every database fallback.
            CREATE INDEX IF NOT EXISTS idx_configs_ranking
                ON configs(reachable, stability_count DESC, latency_ms ASC);

            CREATE TABLE IF NOT EXISTS stable_top (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                keys_json TEXT NOT NULL
            );

            -- User preferences migrated from `configs.yaml`: dotted setting
            -- keys (`bind`, `probe.mode`, `sharing.token`, ...) mapped to
            -- compact JSON literals (`\"127.0.0.1:27141\"`, `true`, `900`,
            -- `null`, `[204,200]`), so any leaf round-trips exactly with no
            -- per-field mapping code. New settings persist automatically.
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            -- Subscription sources: `priority` is both the probe ranking
            -- weight and the list order (lower runs first, ties keep
            -- insertion order), in the TUI and on the dashboard alike.
            -- Drag-and-drop renumbers priorities, so no second column.
            -- URLs are unique: later twins of an earlier URL drop out on
            -- open (first row wins), then the unique index refuses any new
            -- twin the entry guards missed.
            CREATE TABLE IF NOT EXISTS subscriptions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                url TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                priority INTEGER NOT NULL DEFAULT 100
            );
            DELETE FROM subscriptions WHERE id NOT IN (
                SELECT MIN(id) FROM subscriptions GROUP BY TRIM(url)
            );
            CREATE UNIQUE INDEX IF NOT EXISTS subscriptions_url_unique ON subscriptions(TRIM(url));

            -- Embedded-defaults snapshot: one row holding the shipped
            -- defaults file (as a single JSON blob) plus the app version
            -- that wrote it and a content fingerprint of the defaults file
            -- itself. Upgrades three-way-merge live settings against this
            -- baseline (untouched keys follow new defaults, customized keys
            -- are left alone); reset re-anchors it. The binary owns this
            -- table — installers never touch SQLite.
            CREATE TABLE IF NOT EXISTS default_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                app_version TEXT NOT NULL,
                data_version TEXT NOT NULL DEFAULT '',
                defaults_json TEXT NOT NULL
            );",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::significant_drop_tightening
    )]
    pub fn upsert_configs(&self, configs: &[RankedConfig]) -> Result<()> {
        if configs.is_empty() {
            return Ok(());
        }
        let now = Utc::now().to_rfc3339();
        let mut conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        // One transaction per refresh batch: a single commit instead of one
        // per row (fewer fsyncs at checkpoint, less WAL churn — kinder to
        // SSDs/SD cards), and a killed process can never leave a
        // half-persisted refresh behind.
        let tx = conn.transaction()?;

        let mut stmt = tx
            .prepare(
                "INSERT INTO configs (
                    dedup_key, uri, source, priority, protocol, name,
                    endpoint_host, endpoint_port, reachable, validation,
                    latency_ms, http_status, download_mbps, download_bytes,
                    country_code, stability_count, last_online, created_at, updated_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6,
                    ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13, ?14,
                    ?15, ?16, ?17, ?18, ?18
                )
                ON CONFLICT(dedup_key) DO UPDATE SET
                    uri = excluded.uri,
                    source = excluded.source,
                    priority = excluded.priority,
                    protocol = excluded.protocol,
                    name = excluded.name,
                    endpoint_host = excluded.endpoint_host,
                    endpoint_port = excluded.endpoint_port,
                    reachable = excluded.reachable,
                    validation = excluded.validation,
                    latency_ms = excluded.latency_ms,
                    http_status = excluded.http_status,
                    download_mbps = excluded.download_mbps,
                    download_bytes = excluded.download_bytes,
                    country_code = excluded.country_code,
                    stability_count = excluded.stability_count,
                    last_online = CASE
                        WHEN excluded.reachable = 1 THEN excluded.last_online
                        ELSE configs.last_online
                    END,
                    updated_at = excluded.updated_at",
            )
            .context("failed to prepare upsert statement")?;

        for config in configs {
            let reachable_i64 = i64::from(config.reachable);
            stmt.execute(params![
                config.dedup_key,
                config.uri,
                config.source,
                config.priority,
                config.protocol,
                config.name,
                config.endpoint.host,
                config.endpoint.port,
                reachable_i64,
                config.validation,
                config.latency_ms.map(|v| v as i64),
                config.http_status.map(i64::from),
                config.download_mbps,
                config.download_bytes.map(|v| v as i64),
                config.country_code,
                config.stability_count,
                now,
                now,
            ])
            .with_context(|| {
                format!(
                    "failed to upsert config with dedup_key={}",
                    config.dedup_key
                )
            })?;
        }
        drop(stmt);
        tx.commit()?;

        Ok(())
    }

    pub fn load_stable_top_keys(&self) -> Result<HashSet<String>> {
        let result: Option<String> = {
            let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
            conn.query_row("SELECT keys_json FROM stable_top WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()
            .context("failed to read stable_top keys")?
        };

        match result {
            Some(json) => {
                let keys: Vec<String> =
                    serde_json::from_str(&json).context("failed to parse stable_top keys JSON")?;
                Ok(keys.into_iter().collect())
            }
            None => Ok(HashSet::new()),
        }
    }

    #[allow(clippy::significant_drop_tightening)]
    pub fn save_stable_top_keys(&self, keys: &[String]) -> Result<()> {
        let json = serde_json::to_string(keys).context("failed to serialize stable_top keys")?;
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT INTO stable_top (id, keys_json) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET keys_json = excluded.keys_json",
            params![json],
        )?;
        Ok(())
    }

    #[allow(clippy::significant_drop_tightening)]
    pub fn delete_stable_top_keys(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute("DELETE FROM stable_top", [])?;
        Ok(())
    }

    #[allow(clippy::significant_drop_tightening)]
    pub fn clean_offline_configs(&self, after_days: u32) -> Result<usize> {
        let days_str = format!("-{after_days} days");
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        // `datetime(last_online)` normalizes the stored RFC3339 stamps
        // (`…T…+00:00`) to the same `YYYY-MM-DD HH:MM:SS` shape as the
        // cutoff: a raw string comparison would misorder stamps sharing
        // the cutoff's calendar day (`T` > ` `) and skew retention by up
        // to a day on the boundary.
        let deleted = conn
            .execute(
                "DELETE FROM configs WHERE datetime(last_online) < datetime('now', ?1)",
                params![days_str],
            )
            .context("failed to clean offline configs")?;
        Ok(deleted)
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        clippy::significant_drop_tightening
    )]
    pub fn load_ranked_configs(&self, limit: usize) -> Result<Vec<RankedConfig>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, dedup_key, uri, source, priority, protocol, name,
                        endpoint_host, endpoint_port, reachable, validation,
                        latency_ms, http_status, download_mbps, download_bytes,
                        country_code, stability_count
                 FROM configs
                 WHERE reachable = 1
                 ORDER BY stability_count DESC, latency_ms ASC NULLS LAST
                 LIMIT ?1",
            )
            .context("failed to prepare ranked configs query")?;

        let rows = stmt
            .query_map(params![limit as i64], map_ranked_row)
            .context("failed to query ranked configs")?;

        let mut configs = Vec::new();
        for (index, row) in rows.enumerate() {
            let mut config = row.context("failed to read ranked config row")?;
            config.rank = index + 1;
            configs.push(config);
        }

        Ok(configs)
    }

    /// Pool of previously-seen configs the current cycle did not test.
    ///
    /// Order (what makes a config worth re-testing):
    /// - configs that previously connected successfully come first (they are
    ///   proven on this network), the most stable and fastest of them first;
    /// - then the never-tested rows: the configs a feed served but no cycle
    ///   ever probed, because the refresh early-stops after `top_n`. They
    ///   are the same fresh pool a fetch would check next, so they outrank
    ///   the known failures;
    /// - then the failed rows, last: they were probed and did not work.
    ///
    /// Within a group the most recently seen config comes first (the
    /// `last_online` / `updated_at` stamps), so a fresh sighting beats a
    /// stale failure and a config a feed still serves beats one it dropped.
    ///
    /// The refresh uses this when it cannot fill `top_n` working configs on
    /// its own: every source failed to fetch, or too many probes failed.
    /// The ping uses it to backfill a cycle that verified too little. The
    /// pool is consumed in batches until the target is met, so the first
    /// batch has to hold the likeliest configs.
    #[allow(clippy::significant_drop_tightening)]
    pub fn load_fallback_candidates(
        &self,
        exclude: &HashSet<String>,
        limit: usize,
    ) -> Result<Vec<RankedConfig>> {
        let mut sql = String::from(
            "SELECT id, dedup_key, uri, source, priority, protocol, name,
                    endpoint_host, endpoint_port, reachable, validation,
                    latency_ms, http_status, download_mbps, download_bytes,
                    country_code, stability_count
             FROM configs",
        );
        if !exclude.is_empty() {
            let placeholders = vec!["?"; exclude.len()].join(", ");
            sql.push_str(" WHERE dedup_key NOT IN (");
            sql.push_str(&placeholders);
            sql.push(')');
        }
        // Veterans (reachable) first; among the rest, never-tested
        // (`validation = ''`, sighted insert-only but never probed) before
        // failed (`validation != ''`, probed and failed, even with history).
        // The recency stamps break the ties inside each group.
        sql.push_str(
            " ORDER BY reachable DESC, (validation = '') DESC, stability_count DESC,
                     latency_ms ASC NULLS LAST, last_online DESC, updated_at DESC LIMIT ?",
        );

        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn
            .prepare(&sql)
            .context("failed to prepare fallback candidates query")?;

        let limit_param = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut values: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(exclude.len() + 1);
        for key in exclude {
            values.push(key);
        }
        values.push(&limit_param);
        let rows = stmt
            .query_map(values.as_slice(), map_ranked_row)
            .context("failed to query fallback candidates")?;

        let mut configs = Vec::new();
        for (index, row) in rows.enumerate() {
            let mut config = row.context("failed to read fallback candidate row")?;
            config.rank = index + 1;
            configs.push(config);
        }

        Ok(configs)
    }

    /// Cache fetch sightings without touching known rows: untested
    /// candidates become backfill-eligible pool entries (reachable 0,
    /// no measurements); rows already in the database keep their tested
    /// results exactly as-is, so a re-fetch can never clobber a working
    /// config back to untested. Pruning still removes configs whose
    /// sources stop serving them (they are never re-inserted).
    #[allow(clippy::significant_drop_tightening)]
    pub fn insert_new_candidates(&self, candidates: &[Candidate]) -> Result<()> {
        if candidates.is_empty() {
            return Ok(());
        }
        let now = Utc::now().to_rfc3339();
        let mut conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        // Same single-commit batching as `upsert_configs`: fetch sightings
        // can number in the thousands per refresh.
        let tx = conn.transaction()?;

        let mut stmt = tx
            .prepare(
                "INSERT INTO configs (
                    dedup_key, uri, source, priority, protocol, name,
                    endpoint_host, endpoint_port, reachable, validation,
                    latency_ms, http_status, download_mbps, download_bytes,
                    country_code, stability_count, last_online, created_at, updated_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6,
                    ?7, ?8, 0, '',
                    NULL, NULL, NULL, NULL,
                    NULL, 0, ?9, ?9, ?9
                )
                ON CONFLICT(dedup_key) DO UPDATE SET
                    -- A sighting means the source still offers this config, so
                    -- the last-seen-online stamp has to move. Only the stamp
                    -- does: the probe results of a known row stay untouched.
                    -- Without this the stamp stayed at the first sighting and
                    -- the offline cleanup pruned a config the source was
                    -- still serving.
                    last_online = excluded.last_online,
                    updated_at = excluded.updated_at",
            )
            .context("failed to prepare sighting insert statement")?;

        for candidate in candidates {
            stmt.execute(params![
                candidate.dedup_key,
                candidate.uri,
                candidate.source,
                candidate.priority,
                candidate.protocol,
                candidate.name,
                candidate.endpoint.host,
                candidate.endpoint.port,
                now,
            ])
            .with_context(|| {
                format!(
                    "failed to insert sighting with dedup_key={}",
                    candidate.dedup_key
                )
            })?;
        }
        drop(stmt);
        tx.commit()?;

        Ok(())
    }

    #[allow(clippy::significant_drop_tightening)]
    pub fn delete_all(&self) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM configs", [])?;
        tx.execute("DELETE FROM stable_top", [])?;
        // `settings` and `subscriptions` are user data, not probe cache:
        // a cache clean must never wipe preferences or sources.
        tx.commit()?;
        drop(conn);
        Ok(())
    }

    /// Store a setting the current schema does not know.
    ///
    /// Test helper: it stands in for a setting a newer release added, so a
    /// test can check that an older save leaves it alone.
    #[cfg(test)]
    #[allow(clippy::significant_drop_tightening)]
    pub fn save_extra_setting_for_test(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Age every stored "last seen" stamp by whole days.
    ///
    /// Test helper: the stamps are written with the current time, so a test
    /// has to be able to move them into the past to exercise the cleanup.
    #[cfg(test)]
    #[allow(clippy::significant_drop_tightening)]
    pub fn age_last_online_for_test(&self, days: i64) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let changed = conn.execute(
            "UPDATE configs SET last_online = datetime(last_online, ?1)",
            params![format!("{days} days")],
        )?;
        Ok(changed)
    }

    /// True once user preferences live here (seeded or migrated).
    /// An empty table means first run: seed defaults instead of loading.
    #[allow(clippy::significant_drop_tightening)]
    pub fn has_settings(&self) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))
            .context("failed to count settings rows")?;
        Ok(count > 0)
    }

    /// Replace the settings **and** the subscription list in one transaction.
    ///
    /// The two used to be written separately, so a crash between them left a
    /// torn state: the settings half was committed while the subscription
    /// list kept its old rows. The first-run sentinel only counts the
    /// settings rows, so the next boot then saw no settings, treated the
    /// database as fresh and re-seeded it -- overwriting the list.
    #[allow(clippy::significant_drop_tightening)]
    pub fn save_settings_and_subscriptions(
        &self,
        entries: &[(String, String)],
        sources: &[crate::config::SubscriptionSource],
    ) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let tx = conn.transaction()?;
        // Delete only the settings this config owns. A row for a setting a
        // newer release added must survive: the read path keeps unknown keys
        // ("a newer database never breaks an older binary"), so the write
        // path has to as well. Clearing the whole table instead meant an
        // older binary that saved wiped the newer settings, and the newer
        // release lost the user's values.
        {
            let mut stmt = tx
                .prepare("DELETE FROM settings WHERE key = ?1")
                .context("failed to prepare settings delete")?;
            for (key, _) in entries {
                stmt.execute(params![key])
                    .with_context(|| format!("failed to clear setting {key}"))?;
            }
        }
        {
            let mut stmt = tx
                .prepare("INSERT INTO settings (key, value) VALUES (?1, ?2)")
                .context("failed to prepare settings insert")?;
            for (key, value) in entries {
                stmt.execute(params![key, value])
                    .with_context(|| format!("failed to save setting {key}"))?;
            }
        }
        tx.execute("DELETE FROM subscriptions", [])
            .context("failed to clear subscriptions")?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO subscriptions (name, url, enabled, priority)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .context("failed to prepare subscriptions insert")?;
            for source in sources {
                stmt.execute(params![
                    source.name,
                    source.url,
                    i64::from(source.enabled),
                    i64::from(source.priority),
                ])
                .with_context(|| format!("failed to save subscription {}", source.name))?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    #[allow(clippy::significant_drop_tightening)]
    pub fn load_settings(&self) -> Result<std::collections::HashMap<String, String>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn
            .prepare("SELECT key, value FROM settings")
            .context("failed to prepare settings query")?;
        let rows = stmt
            .query_map([], |row| {
                let key: String = row.get(0)?;
                let value: String = row.get(1)?;
                Ok((key, value))
            })
            .context("failed to query settings")?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (key, value) = row.context("failed to read settings row")?;
            map.insert(key, value);
        }
        Ok(map)
    }

    /// Store the embedded-defaults snapshot (single row, id 1): the app
    /// version that wrote it, the content fingerprint of the defaults file,
    /// and the defaults themselves as one JSON blob.
    #[allow(clippy::significant_drop_tightening)]
    pub fn save_defaults_snapshot(
        &self,
        app_version: &str,
        data_version: &str,
        defaults_json: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT INTO default_settings (id, app_version, data_version, defaults_json)
             VALUES (1, ?1, ?2, ?3)
             ON CONFLICT (id) DO UPDATE SET
               app_version = excluded.app_version,
               data_version = excluded.data_version,
               defaults_json = excluded.defaults_json",
            params![app_version, data_version, defaults_json],
        )
        .context("failed to save defaults snapshot")?;
        Ok(())
    }

    /// Load the snapshot row: `(app_version, data_version, defaults_json)`.
    /// `None` on fresh databases and pre-snapshot upgrades alike.
    pub fn load_defaults_snapshot(&self) -> Result<Option<(String, String, String)>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.query_row(
            "SELECT app_version, data_version, defaults_json FROM default_settings WHERE id = 1",
            [],
            |row| {
                let app_version: String = row.get(0)?;
                let data_version: String = row.get(1)?;
                let defaults_json: String = row.get(2)?;
                Ok((app_version, data_version, defaults_json))
            },
        )
        .optional()
        .context("failed to read defaults snapshot")
    }

    #[allow(clippy::significant_drop_tightening)]
    pub fn load_subscriptions(&self) -> Result<Vec<crate::config::SubscriptionSource>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn
            .prepare(
                "SELECT name, url, enabled, priority FROM subscriptions
                 ORDER BY priority ASC, id ASC",
            )
            .context("failed to prepare subscriptions query")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(crate::config::SubscriptionSource {
                    name: row.get(0)?,
                    url: row.get(1)?,
                    enabled: row.get::<_, i64>(2)? != 0,
                    priority: row.get::<_, u32>(3)?,
                })
            })
            .context("failed to query subscriptions")?;
        let mut sources = Vec::new();
        for row in rows {
            sources.push(row.context("failed to read subscription row")?);
        }
        Ok(sources)
    }
}

/// Shared row mapping for ranked-config loaders (`load_ranked_configs`,
/// `load_backfill_candidates`); rank is assigned by the caller.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
fn map_ranked_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RankedConfig> {
    Ok(RankedConfig {
        rank: 0,
        stability_count: row.get::<_, u32>(16)?,
        id: format!("{:016x}", row.get::<_, i64>(0)? as u64),
        dedup_key: row.get(1)?,
        source: row.get(3)?,
        priority: row.get(4)?,
        protocol: row.get(5)?,
        name: row.get(6)?,
        endpoint: crate::model::Endpoint {
            host: row.get(7)?,
            port: row.get(8)?,
        },
        uri: row.get(2)?,
        reachable: row.get::<_, i64>(9)? != 0,
        validation: row.get(10)?,
        latency_ms: row.get::<_, Option<i64>>(11)?.map(|v| v as u128),
        http_status: row.get::<_, Option<i64>>(12)?.map(|v| v as u16),
        download_mbps: row.get(13)?,
        download_bytes: row.get::<_, Option<i64>>(14)?.map(|v| v as usize),
        error: None,
        country_code: row.get(15)?,
    })
}

/// Restrict the `SQLite` file to owner-only (0600 on Unix).
/// Best-effort, no-op on Windows; single syscall, no query delay.
#[allow(clippy::missing_const_for_fn)]
fn restrict_file_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Endpoint;

    fn ranked_row(key: &str, reachable: bool, validation: &str, stability: u32) -> RankedConfig {
        RankedConfig {
            rank: 0,
            stability_count: stability,
            id: format!("id-{key}"),
            dedup_key: key.to_string(),
            source: "test".to_string(),
            priority: 1,
            protocol: "vless".to_string(),
            name: key.to_string(),
            endpoint: Endpoint {
                host: "example.com".to_string(),
                port: 443,
            },
            uri: format!("vless://{key}@example.com:443"),
            reachable,
            validation: validation.to_string(),
            latency_ms: None,
            http_status: None,
            download_mbps: None,
            download_bytes: None,
            error: None,
            country_code: None,
        }
    }

    #[test]
    fn fallback_pool_orders_never_tested_before_failed() {
        // Veterans first, then never-tested sightings (validation ''), then
        // failed rows — even when a failed row carries stability history.
        let db = open_temp_db("backfill-order");
        db.upsert_configs(&[
            ranked_row("failed-history", false, "active_http", 5),
            ranked_row("veteran", true, "active_http", 1),
        ])
        .expect("seeded tested rows");
        db.insert_new_candidates(&[Candidate {
            id: "id-fresh".to_string(),
            dedup_key: "fresh".to_string(),
            source: "test".to_string(),
            priority: 1,
            protocol: "vless".to_string(),
            name: "fresh".to_string(),
            endpoint: Endpoint {
                host: "example.com".to_string(),
                port: 443,
            },
            uri: "vless://fresh@example.com:443".to_string(),
        }])
        .expect("seeded sighting");
        let order: Vec<String> = db
            .load_fallback_candidates(&HashSet::new(), 10)
            .expect("fallback pool loads")
            .into_iter()
            .map(|item| item.dedup_key)
            .collect();
        assert_eq!(order, vec!["veteran", "fresh", "failed-history"]);
    }

    /// A temp directory that no other test can share.
    ///
    /// The wall clock is not enough: it only ticks every few milliseconds,
    /// so two tests - or two test processes - that ask for a directory in
    /// the same tick used to get the same one, and then the same database
    /// file, and then a "UNIQUE constraint failed" on the second insert.
    /// The process id separates the processes, the counter the tests.
    fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "v2raydar-{prefix}-{}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        ));
        std::fs::create_dir_all(&dir).expect("temp dir can be created");
        dir
    }

    fn open_temp_db(name: &str) -> Database {
        let dir = unique_temp_dir(&format!("upsert-batch-{name}"));
        Database::open(&dir.join("data.db")).expect("db opens")
    }

    /// Seed one config row with exact ordering keys. The upsert helpers
    /// stamp `last_online` / `updated_at` with "now", so the ordering test
    /// needs to write the timestamps itself.
    #[allow(clippy::too_many_arguments)]
    fn seed_ordered_row(
        db: &Database,
        key: &str,
        reachable: bool,
        validation: &str,
        stability: u32,
        latency_ms: Option<i64>,
        last_online: &str,
        updated_at: &str,
    ) {
        {
            let conn = db.conn.lock().expect("db lock");
            conn.execute(
                "INSERT INTO configs (
                    dedup_key, uri, source, priority, protocol, name,
                    endpoint_host, endpoint_port, reachable, validation,
                    latency_ms, http_status, download_mbps, download_bytes,
                    country_code, stability_count, last_online, created_at, updated_at
                 ) VALUES (?1, ?2, 'test', 1, 'vless', ?1,
                    'example.com', 443, ?3, ?4,
                    ?5, NULL, NULL, NULL,
                    NULL, ?6, ?7, ?7, ?8)",
                params![
                    key,
                    format!("vless://{key}@example.com:443"),
                    i64::from(reachable),
                    validation,
                    latency_ms,
                    i64::from(stability),
                    last_online,
                    updated_at,
                ],
            )
            .expect("row inserts");
            // Release the lock right after its last use.
            drop(conn);
        }
    }

    #[test]
    fn fallback_pool_prefers_proven_working_then_the_most_recently_seen() {
        // The pool is consumed in batches until `top_n` is met, so the first
        // batch has to hold the likeliest configs: the ones that previously
        // connected successfully (most stable, then fastest), then the
        // never-tested sightings (the fresh pool a fetch would check next),
        // and only then the known failures. Inside a group the most
        // recently seen config comes first.
        let db = open_temp_db("fallback-order");
        seed_ordered_row(
            &db,
            "proven-slow",
            true,
            "active_http",
            1,
            Some(500),
            "2026-10-20T10:00:00+00:00",
            "2026-10-20T10:00:00+00:00",
        );
        seed_ordered_row(
            &db,
            "proven-fast",
            true,
            "active_http",
            1,
            Some(30),
            "2026-10-01T10:00:00+00:00",
            "2026-10-01T10:00:00+00:00",
        );
        seed_ordered_row(
            &db,
            "proven-stable",
            true,
            "active_http",
            5,
            Some(900),
            "2026-01-01T10:00:00+00:00",
            "2026-01-01T10:00:00+00:00",
        );
        seed_ordered_row(
            &db,
            "stale-sighting",
            false,
            "",
            0,
            None,
            "2026-01-01T10:00:00+00:00",
            "2026-01-01T10:00:00+00:00",
        );
        seed_ordered_row(
            &db,
            "fresh-sighting",
            false,
            "",
            0,
            None,
            "2026-10-20T09:00:00+00:00",
            "2026-10-20T09:00:00+00:00",
        );
        seed_ordered_row(
            &db,
            "failed-recent",
            false,
            "active_http",
            0,
            None,
            "2026-10-20T09:00:00+00:00",
            "2026-10-20T09:00:00+00:00",
        );

        let order: Vec<String> = db
            .load_fallback_candidates(&HashSet::new(), 10)
            .expect("fallback pool loads")
            .into_iter()
            .map(|item| item.dedup_key)
            .collect();

        assert_eq!(
            order,
            vec![
                "proven-stable",
                "proven-fast",
                "proven-slow",
                "fresh-sighting",
                "stale-sighting",
                "failed-recent",
            ]
        );
    }

    #[test]
    fn fallback_pool_skips_the_configs_the_cycle_already_tested() {
        let db = open_temp_db("fallback-exclude");
        for key in ["kept", "skipped"] {
            seed_ordered_row(
                &db,
                key,
                true,
                "active_http",
                1,
                Some(30),
                "2026-10-20T10:00:00+00:00",
                "2026-10-20T10:00:00+00:00",
            );
        }
        let order: Vec<String> = db
            .load_fallback_candidates(&HashSet::from(["skipped".to_string()]), 10)
            .expect("fallback pool loads")
            .into_iter()
            .map(|item| item.dedup_key)
            .collect();
        assert_eq!(order, vec!["kept"]);
    }

    fn stability_and_last_online(db: &Database, key: &str) -> (u32, String) {
        let conn = db.conn.lock().expect("db lock");
        conn.query_row(
            "SELECT stability_count, last_online FROM configs WHERE dedup_key = ?1",
            params![key],
            |row| {
                let stability: u32 = row.get(0)?;
                let last_online: String = row.get(1)?;
                Ok((stability, last_online))
            },
        )
        .expect("row exists")
    }

    #[test]
    fn upsert_batch_persists_stability_and_gates_last_online() {
        let db = open_temp_db("stability");
        db.upsert_configs(&[
            ranked_row("steady", true, "active_http", 5),
            ranked_row("flaky", true, "active_http", 2),
            ranked_row("dead", false, "active_http", 4),
        ])
        .expect("initial batch persists");

        assert_eq!(stability_and_last_online(&db, "steady").0, 5);
        assert_eq!(stability_and_last_online(&db, "dead").0, 4);
        let seen_before = stability_and_last_online(&db, "steady").1;

        // A failed cycle overwrites the count with the freshly computed
        // value but must NOT move last_online backwards/forwards.
        db.upsert_configs(&[ranked_row("steady", false, "active_http", 5)])
            .expect("failed cycle persists");
        let (stability, seen_after) = stability_and_last_online(&db, "steady");
        assert_eq!(stability, 5);
        assert_eq!(seen_after, seen_before);

        // A working cycle advances last_online monotonically.
        db.upsert_configs(&[ranked_row("steady", true, "active_http", 6)])
            .expect("working cycle persists");
        let (stability, seen_latest) = stability_and_last_online(&db, "steady");
        assert_eq!(stability, 6);
        assert!(seen_latest >= seen_after);
    }

    fn insert_row_with_last_online(db: &Database, key: &str, last_online: &str) {
        let uri = format!("vless://{key}@example.com:443");
        let conn = db.conn.lock().expect("db lock");
        conn.execute(
            "INSERT INTO configs (
                dedup_key, uri, source, priority, protocol, name,
                endpoint_host, endpoint_port, reachable, validation,
                stability_count, last_online, created_at, updated_at
            ) VALUES (?1, ?2, 'test', 1, 'vless', ?1, 'example.com', 443, 0, '', 0, ?3, ?3, ?3)",
            params![key, uri, last_online],
        )
        .expect("row inserts");
    }

    fn row_exists(db: &Database, key: &str) -> bool {
        let conn = db.conn.lock().expect("db lock");
        conn.query_row(
            "SELECT 1 FROM configs WHERE dedup_key = ?1",
            params![key],
            |_| Ok(()),
        )
        .optional()
        .expect("query runs")
        .is_some()
    }

    #[test]
    fn clean_offline_configs_prunes_by_true_age_not_string_shape() {
        use chrono::{Duration, Utc};

        let db = open_temp_db("clean");
        let now = Utc::now();
        // Ancient rows always prune.
        insert_row_with_last_online(&db, "ancient", "2000-01-01T00:00:00+00:00");
        // Seen an hour past the 7-day cutoff: older than retention, but its
        // calendar day matches the cutoff day — a raw string comparison
        // misorders the `T` against the cutoff's space and wrongly keeps it.
        let boundary = (now - Duration::days(7) - Duration::hours(1)).to_rfc3339();
        insert_row_with_last_online(&db, "boundary", &boundary);
        // Seen an hour ago: fresh, must survive.
        let fresh = (now - Duration::hours(1)).to_rfc3339();
        insert_row_with_last_online(&db, "fresh", &fresh);

        let deleted = db.clean_offline_configs(7).expect("cleanup runs");
        assert_eq!(deleted, 2);
        assert!(!row_exists(&db, "ancient"));
        assert!(!row_exists(&db, "boundary"));
        assert!(row_exists(&db, "fresh"));
    }

    fn subscription_row(name: &str, url: &str, priority: i64) -> crate::config::SubscriptionSource {
        crate::config::SubscriptionSource {
            name: name.to_string(),
            url: url.to_string(),
            enabled: true,
            priority: u32::try_from(priority).unwrap_or(100),
        }
    }

    #[test]
    fn subscription_urls_stay_unique_and_legacy_twins_converge() {
        let db = open_temp_db("sub-unique");
        db.save_settings_and_subscriptions(
            &[],
            &[
                subscription_row("a", "https://example.com/a.txt", 1),
                subscription_row("b", "https://example.com/b.txt", 2),
            ],
        )
        .expect("seed persists");
        // A twin URL (even padded) is refused by the unique index.
        let twin = db.save_settings_and_subscriptions(
            &[],
            &[
                subscription_row("a", "https://example.com/a.txt", 1),
                subscription_row("twin", "  https://example.com/a.txt ", 2),
            ],
        );
        assert!(twin.is_err(), "duplicate URL must not persist");

        // Legacy databases that already hold twins converge on open: the
        // first row survives, later twins drop out, and the index lands.
        let dir = unique_temp_dir("sub-twins");
        let path = dir.join("data.db");
        {
            let raw = rusqlite::Connection::open(&path).expect("raw db opens");
            raw.execute_batch(
                "CREATE TABLE subscriptions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    url TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    priority INTEGER NOT NULL DEFAULT 100
                );
                INSERT INTO subscriptions (name, url, enabled, priority) VALUES
                    ('a', 'https://example.com/a.txt', 1, 1),
                    ('twin', 'https://example.com/a.txt', 1, 2),
                    ('b', 'https://example.com/b.txt', 1, 3);",
            )
            .expect("legacy twins seed");
        }
        let db = Database::open(&path).expect("db with twins still opens");
        let names: Vec<String> = db
            .load_subscriptions()
            .expect("subs load")
            .into_iter()
            .map(|source| source.name)
            .collect();
        assert_eq!(names, vec!["a", "b"], "first row wins, twin drops out");
    }
}
