use std::process::Stdio;

use anyhow::{Context, Result, anyhow};
use tokio::process::Command;

use crate::{
    config::{AppConfig, ProbeMode},
    paths::AppPaths,
};

pub const DOWNLOAD_URL: &str = "https://github.com/SagerNet/sing-box/releases";

pub async fn active_probe_needs_setup(config: &AppConfig, _paths: &AppPaths) -> bool {
    if config.probe.mode != ProbeMode::Active {
        return false;
    }

    if should_setup_path(&config.probe.sing_box_path) {
        return true;
    }

    verify_path(&config.probe.sing_box_path).await.is_err()
}

fn should_setup_path(value: &str) -> bool {
    let trimmed = normalize_path(value);
    trimmed.is_empty() || trimmed == "sing-box" || trimmed == "sing-box.exe"
}

pub fn normalize_path(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        return trimmed[1..trimmed.len() - 1].trim().to_string();
    }

    trimmed.to_string()
}

pub async fn verify_path(path: &str) -> Result<()> {
    let path = normalize_path(path);
    if path.is_empty() {
        return Err(anyhow!("sing-box path cannot be empty"));
    }

    let status = Command::new(&path)
        .arg("version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .with_context(|| {
            format!(
                "unable to run '{path}'. Enter the full sing-box executable path, or download it from {DOWNLOAD_URL}"
            )
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "'{path} version' exited with {status}; enter a valid sing-box executable path"
        ))
    }
}
