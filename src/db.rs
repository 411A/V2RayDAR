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

            CREATE TABLE IF NOT EXISTS stable_top (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                keys_json TEXT NOT NULL
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

    /// Pool for ping backfill: previously-seen configs the current cycle
    /// did not test, veterans first (working, then stable, then fast),
    /// then never-tested rows, then failed rows. Never-tested outranks
    /// failed because the refresh early-stops: the fetch sights thousands
    /// of configs insert-only but only probes until `top_n`, so the
    /// unprobed sightings are the same fresh pool a fetch would check next —
    /// grinding stale failures first is why ping alone found worse configs
    /// than a fetch. The ping only dips into this when it verifies fewer
    /// than `top_n` working configs.
    #[allow(clippy::significant_drop_tightening)]
    pub fn load_backfill_candidates(
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
        sql.push_str(
            " ORDER BY reachable DESC, (validation = '') DESC, stability_count DESC, latency_ms ASC NULLS LAST LIMIT ?",
        );

        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn
            .prepare(&sql)
            .context("failed to prepare backfill candidates query")?;

        let limit_param = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut values: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(exclude.len() + 1);
        for key in exclude {
            values.push(key);
        }
        values.push(&limit_param);
        let rows = stmt
            .query_map(values.as_slice(), map_ranked_row)
            .context("failed to query backfill candidates")?;

        let mut configs = Vec::new();
        for (index, row) in rows.enumerate() {
            let mut config = row.context("failed to read backfill candidate row")?;
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
                ON CONFLICT(dedup_key) DO NOTHING",
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
        tx.commit()?;
        drop(conn);
        Ok(())
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

/// Restrict the SQLite file to owner-only (0600 on Unix).
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
    fn backfill_orders_never_tested_before_failed() {
        // Veterans first, then never-tested sightings (validation ''), then
        // failed rows — even when a failed row carries stability history.
        let dir = std::env::temp_dir().join(format!(
            "v2raydar-backfill-order-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("temp dir can be created");
        let db = Database::open(&dir.join("data.db")).expect("db opens");
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
            .load_backfill_candidates(&HashSet::new(), 10)
            .expect("backfill loads")
            .into_iter()
            .map(|item| item.dedup_key)
            .collect();
        assert_eq!(order, vec!["veteran", "fresh", "failed-history"]);
    }

    fn open_temp_db(name: &str) -> Database {
        let dir = std::env::temp_dir().join(format!(
            "v2raydar-upsert-batch-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("temp dir can be created");
        Database::open(&dir.join("data.db")).expect("db opens")
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
}
