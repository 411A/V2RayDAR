use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use tokio::fs;

use crate::constants::{APP_DIR_NAME, APP_DIR_NAME_LOWER, CACHE_DIR_NAME, CONFIG_FILE_NAME};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root_dir: PathBuf,
    pub config_path: PathBuf,
    pub cache_dir: PathBuf,
    pub portable: bool,
}

impl AppPaths {
    pub fn installed() -> Result<Self> {
        let root_dir = installed_root_dir()?;
        Ok(Self::from_root(root_dir, false))
    }

    pub fn portable() -> Result<Self> {
        let executable = std::env::current_exe().context("unable to locate current executable")?;
        let root_dir = executable
            .parent()
            .ok_or_else(|| anyhow!("unable to resolve executable directory"))?
            .join(APP_DIR_NAME);
        Ok(Self::from_root(root_dir, true))
    }

    pub fn from_config_override(config_path: PathBuf) -> Self {
        let root_dir = config_path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        Self::from_root_with_config(root_dir, config_path, false)
    }

    pub async fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.root_dir)
            .await
            .with_context(|| format!("unable to create {}", self.root_dir.display()))?;
        Ok(())
    }

    fn from_root(root_dir: PathBuf, portable: bool) -> Self {
        let config_path = root_dir.join(CONFIG_FILE_NAME);
        Self::from_root_with_config(root_dir, config_path, portable)
    }

    fn from_root_with_config(root_dir: PathBuf, config_path: PathBuf, portable: bool) -> Self {
        Self {
            config_path,
            cache_dir: root_dir.join(CACHE_DIR_NAME),
            root_dir,
            portable,
        }
    }
}

fn installed_root_dir() -> Result<PathBuf> {
    if cfg!(target_os = "windows") {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            return Ok(PathBuf::from(local_app_data).join(APP_DIR_NAME));
        }

        return Ok(home_dir()?.join("AppData").join("Local").join(APP_DIR_NAME));
    }

    if cfg!(target_os = "macos") {
        return Ok(home_dir()?
            .join("Library")
            .join("Application Support")
            .join(APP_DIR_NAME));
    }

    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(data_home).join(APP_DIR_NAME_LOWER));
    }

    Ok(home_dir()?
        .join(".local")
        .join("share")
        .join(APP_DIR_NAME_LOWER))
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("unable to resolve user home directory"))
}
