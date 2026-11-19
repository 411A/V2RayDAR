//! Dashboard password protection.
//!
//! This is a **change gate**, not a data gate: read-only endpoints and the
//! subscription feeds stay available, while the dashboard UI and every
//! mutating API request require a session. Credentials are stored as Argon2id
//! PHC strings in the settings table, never in the config payload.
//!
//! Sessions live only in this process: a reload keeps the browser signed in,
//! while stopping and starting the app invalidates every old cookie. The
//! secret itself is never stored in the browser; only an `HttpOnly`, `SameSite`
//! session id is.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

use crate::db::Database;

pub const PASSWORD_KEY: &str = "dashboard.password_hash";
pub const PHRASE_KEY: &str = "dashboard.reset_phrase_hash";
pub const SESSION_COOKIE: &str = "v2raydar_session";

pub const MIN_PASSWORD_CHARS: usize = 12;
pub const MIN_PHRASE_CHARS: usize = 8;
pub const SESSION_SECONDS: u64 = 30 * 24 * 60 * 60;
pub const RESET_COOLDOWN: Duration = Duration::from_hours(1);
const LOGIN_BASE_COOLDOWN: Duration = Duration::from_mins(1);
const MAX_SESSIONS: usize = 64;
const MAX_LOGIN_TRACKERS: usize = 512;
const SESSION_BYTES: usize = 32;

#[derive(Debug, Default)]
struct LoginFailure {
    count: u32,
    blocked_until: Option<Instant>,
}

#[derive(Debug, Default)]
struct Inner {
    password_hash: Option<String>,
    phrase_hash: Option<String>,
    sessions: HashMap<String, Instant>,
    login_failures: HashMap<IpAddr, LoginFailure>,
    reset_failed_at: Option<Instant>,
}

#[derive(Debug)]
pub struct DashboardAuth {
    inner: Mutex<Inner>,
    reset_cooldown: Duration,
    login_base_cooldown: Duration,
}

impl Default for DashboardAuth {
    fn default() -> Self {
        Self::new()
    }
}

impl DashboardAuth {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            reset_cooldown: RESET_COOLDOWN,
            login_base_cooldown: LOGIN_BASE_COOLDOWN,
        }
    }

    /// Test/small-instance constructor with shorter rate-limit windows.
    #[cfg(test)]
    #[must_use]
    pub fn with_cooldowns(reset: Duration, login: Duration) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            reset_cooldown: reset,
            login_base_cooldown: login,
        }
    }

    /// Load stored hashes from the settings table. A missing row disables the
    /// password (the default state); a malformed row stays stored so a local
    /// reset or a new password can repair it instead of silently disabling.
    pub fn load(db: &Database) -> Result<Self> {
        let password_hash = db
            .load_setting(PASSWORD_KEY)
            .context("failed to read dashboard password hash")?
            .filter(|value| !value.trim().is_empty());
        let phrase_hash = db
            .load_setting(PHRASE_KEY)
            .context("failed to read dashboard recovery phrase hash")?
            .filter(|value| !value.trim().is_empty());
        let auth = Self::new();
        if let Ok(mut inner) = auth.inner.lock() {
            inner.password_hash = password_hash;
            inner.phrase_hash = phrase_hash;
        }
        Ok(auth)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[must_use]
    pub fn enabled(&self) -> bool {
        self.lock().password_hash.is_some()
    }

    #[must_use]
    pub fn phrase_configured(&self) -> bool {
        self.lock().phrase_hash.is_some()
    }

    #[must_use]
    pub fn password_hash(&self) -> Option<String> {
        self.lock().password_hash.clone()
    }

    #[must_use]
    pub fn phrase_hash(&self) -> Option<String> {
        self.lock().phrase_hash.clone()
    }

    pub fn install_hashes(&self, password_hash: String, phrase_hash: String) {
        let mut inner = self.lock();
        inner.password_hash = Some(password_hash);
        inner.phrase_hash = Some(phrase_hash);
        inner.sessions.clear();
        inner.reset_failed_at = None;
    }

    pub fn clear_hashes(&self) {
        let mut inner = self.lock();
        inner.password_hash = None;
        inner.phrase_hash = None;
        inner.sessions.clear();
        inner.login_failures.clear();
        inner.reset_failed_at = None;
    }

    /// Fresh random session id. Expired sessions are pruned first; the store
    /// is capped so a long-running instance cannot accumulate tokens.
    #[allow(clippy::significant_drop_tightening)]
    pub fn create_session(&self) -> Result<String> {
        let mut bytes = [0_u8; SESSION_BYTES];
        getrandom::fill(&mut bytes).map_err(|error| anyhow!("no secure random source: {error}"))?;
        let token = URL_SAFE_NO_PAD.encode(bytes);
        let now = Instant::now();
        let mut inner = self.lock();
        inner.sessions.retain(|_, expiry| *expiry > now);
        if inner.sessions.len() >= MAX_SESSIONS
            && let Some(oldest) = inner
                .sessions
                .iter()
                .min_by_key(|(_, expiry)| **expiry)
                .map(|(token, _)| token.clone())
        {
            inner.sessions.remove(&oldest);
        }
        inner
            .sessions
            .insert(token.clone(), now + Duration::from_hours(24 * 30));
        Ok(token)
    }

    #[must_use]
    #[allow(clippy::significant_drop_tightening)]
    pub fn valid_session(&self, token: Option<&str>) -> bool {
        let Some(token) = token else {
            return false;
        };
        let now = Instant::now();
        let mut inner = self.lock();
        let Some(expiry) = inner.sessions.get(token).copied() else {
            return false;
        };
        if expiry <= now {
            inner.sessions.remove(token);
            return false;
        }
        true
    }

    pub fn revoke_session(&self, token: Option<&str>) {
        if let Some(token) = token {
            self.lock().sessions.remove(token);
        }
    }

    pub fn revoke_all_sessions(&self) {
        self.lock().sessions.clear();
    }

    /// Remaining login cooldown for an address. Expired entries are cleaned
    /// up lazily so the tracker map stays bounded.
    #[must_use]
    #[allow(clippy::significant_drop_tightening)]
    pub fn login_retry_after(&self, ip: IpAddr) -> Option<Duration> {
        let now = Instant::now();
        let inner = self.lock();
        inner
            .login_failures
            .get(&ip)
            .and_then(|failure| failure.blocked_until)
            .filter(|until| *until > now)
            .map(|until| until.saturating_duration_since(now))
    }

    #[allow(clippy::significant_drop_tightening)]
    pub fn note_login_failure(&self, ip: IpAddr) {
        let now = Instant::now();
        let base = self.login_base_cooldown;
        let mut inner = self.lock();
        if inner.login_failures.len() >= MAX_LOGIN_TRACKERS {
            inner.login_failures.clear();
        }
        let failure = inner.login_failures.entry(ip).or_default();
        failure.count = failure.count.saturating_add(1);
        if failure.count.is_multiple_of(5) {
            let multiplier = match failure.count {
                0..=5 => 1,
                6..=10 => 5,
                _ => 15,
            };
            failure.blocked_until = Some(now + base.saturating_mul(multiplier));
        }
    }

    pub fn clear_login_failures(&self, ip: IpAddr) {
        self.lock().login_failures.remove(&ip);
    }

    /// One reset attempt per window after a wrong recovery phrase. The
    /// window is global to this process so rotating addresses cannot speed up
    /// a brute-force attempt.
    #[must_use]
    pub fn reset_retry_after(&self) -> Option<Duration> {
        let now = Instant::now();
        self.lock()
            .reset_failed_at
            .and_then(|failed_at| {
                self.reset_cooldown
                    .checked_sub(now.saturating_duration_since(failed_at))
            })
            .filter(|remaining| !remaining.is_zero())
    }

    pub fn note_reset_failure(&self) {
        self.lock().reset_failed_at = Some(Instant::now());
    }

    pub fn clear_reset_failure(&self) {
        self.lock().reset_failed_at = None;
    }

    /// Live session count (tests and diagnostics).
    #[cfg(test)]
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.lock().sessions.len()
    }
}

/// Minimum password policy: length over forced character classes (NIST
/// guidance), with a hard upper bound so a huge body cannot make hashing
/// expensive.
pub fn validate_password(password: &str) -> Result<(), String> {
    let length = password.chars().count();
    if length < MIN_PASSWORD_CHARS {
        return Err(format!(
            "password must be at least {MIN_PASSWORD_CHARS} characters"
        ));
    }
    if length > 256 {
        return Err("password must be at most 256 characters".to_string());
    }
    if password.chars().all(char::is_whitespace) {
        return Err("password cannot be only whitespace".to_string());
    }
    Ok(())
}

/// Recovery phrase check. The phrase is matched exactly as typed (spaces and
/// case included); only the length is policed.
pub fn validate_phrase(phrase: &str) -> Result<(), String> {
    let length = phrase.chars().count();
    if length < MIN_PHRASE_CHARS {
        return Err(format!(
            "recovery phrase must be at least {MIN_PHRASE_CHARS} characters"
        ));
    }
    if length > 512 {
        return Err("recovery phrase must be at most 512 characters".to_string());
    }
    Ok(())
}

pub async fn hash_secret(secret: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || hash_secret_blocking(&secret))
        .await
        .map_err(|error| format!("password hashing task failed: {error}"))?
}

pub fn hash_secret_blocking(secret: &str) -> Result<String, String> {
    Argon2::default()
        .hash_password(secret.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|error| format!("password hashing failed: {error}"))
}

pub async fn verify_secret(hash: String, secret: String) -> bool {
    tokio::task::spawn_blocking(move || verify_secret_blocking(&hash, &secret))
        .await
        .unwrap_or(false)
}

fn verify_secret_blocking(hash: &str, secret: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(secret.as_bytes(), &parsed)
        .is_ok()
}

/// Delete both credentials (local recovery path). Returns true when anything
/// was stored.
pub fn clear_credentials(db: &Database) -> Result<bool> {
    let password = db.delete_setting(PASSWORD_KEY)?;
    let phrase = db.delete_setting(PHRASE_KEY)?;
    Ok(password || phrase)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn password_hash_round_trips_and_rejects_wrong_input() {
        let hash = hash_secret("correct horse battery staple".to_string())
            .await
            .expect("hash");
        assert!(verify_secret(hash.clone(), "correct horse battery staple".to_string()).await);
        assert!(!verify_secret(hash, "Correct horse battery staple".to_string()).await);
    }

    #[test]
    fn password_policy_prefers_length_with_a_hard_cap() {
        assert!(validate_password("short").is_err());
        assert!(validate_password("twelve chars").is_ok());
        assert!(validate_password("            ").is_err());
        assert!(validate_password(&"x".repeat(257)).is_err());
    }

    #[test]
    fn recovery_phrase_is_exact() {
        assert!(validate_phrase("short").is_err());
        assert!(validate_phrase("John Smith").is_ok());
        assert!(validate_phrase(" John Smith ").is_ok());
        // Comparing is done by Argon2 over the exact bytes, not by trim().
        let hash = hash_secret_blocking("John Smith").expect("hash");
        assert!(verify_secret_blocking(&hash, "John Smith"));
        assert!(!verify_secret_blocking(&hash, "john smith"));
        assert!(!verify_secret_blocking(&hash, " John Smith"));
    }

    #[test]
    fn sessions_are_bounded_and_revocable() {
        let auth = DashboardAuth::new();
        let first = auth.create_session().expect("session");
        assert!(auth.valid_session(Some(&first)));
        assert!(!auth.valid_session(None));
        assert!(!auth.valid_session(Some("not-a-token")));
        auth.revoke_session(Some(&first));
        assert!(!auth.valid_session(Some(&first)));

        for _ in 0..(MAX_SESSIONS + 4) {
            let _ = auth.create_session().expect("session");
        }
        assert!(auth.session_count() <= MAX_SESSIONS);
        auth.revoke_all_sessions();
        assert_eq!(auth.session_count(), 0);
    }

    #[test]
    fn repeated_login_failures_start_a_cooldown() {
        let auth = DashboardAuth::with_cooldowns(Duration::from_hours(1), Duration::from_mins(1));
        let ip: IpAddr = "127.0.0.1".parse().expect("ip");
        for _ in 0..4 {
            auth.note_login_failure(ip);
        }
        assert!(auth.login_retry_after(ip).is_none());
        auth.note_login_failure(ip);
        assert!(auth.login_retry_after(ip).is_some());
        auth.clear_login_failures(ip);
        assert!(auth.login_retry_after(ip).is_none());
    }

    #[test]
    fn a_wrong_recovery_phrase_blocks_the_next_attempt() {
        let auth = DashboardAuth::with_cooldowns(Duration::from_hours(1), Duration::from_mins(1));
        assert!(auth.reset_retry_after().is_none());
        auth.note_reset_failure();
        let remaining = auth.reset_retry_after().expect("cooldown");
        assert!(remaining > Duration::from_secs(3590));
        auth.clear_reset_failure();
        assert!(auth.reset_retry_after().is_none());
    }
}
