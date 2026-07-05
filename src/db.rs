use std::{collections::HashSet, path::Path, sync::Mutex};

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use crate::model::RankedConfig;

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database at {}", path.display()))?;

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
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;

        let mut stmt = conn
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
        let deleted = conn
            .execute(
                "DELETE FROM configs WHERE last_online < datetime('now', ?1)",
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
            .query_map(params![limit as i64], |row| {
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
            })
            .context("failed to query ranked configs")?;

        let mut configs = Vec::new();
        for (index, row) in rows.enumerate() {
            let mut config = row.context("failed to read ranked config row")?;
            config.rank = index + 1;
            configs.push(config);
        }

        Ok(configs)
    }

    pub fn delete_all(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute("DELETE FROM configs", [])?;
        conn.execute("DELETE FROM stable_top", [])?;
        drop(conn);
        Ok(())
    }
}
