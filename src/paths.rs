use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use tokio::fs;

use crate::constants::{APP_DATA_DIR_NAME, APP_NAME, CACHE_DIR_NAME, CONFIG_FILE_NAME};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root_dir: PathBuf,
    pub config_path: PathBuf,
    pub cache_dir: PathBuf,
    pub portable: bool,
    pub generated_config: bool,
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
            .join(APP_DATA_DIR_NAME);
        Ok(Self::from_root(root_dir, true))
    }

    pub fn from_config_override(config_path: PathBuf) -> Self {
        let config_parent = config_path
            .parent()
            .map_or_else(|| PathBuf::from("."), PathBuf::from);
        let root_dir = config_parent
            .ancestors()
            .find(|path| path.file_name().and_then(|name| name.to_str()) == Some(APP_DATA_DIR_NAME))
            .map_or_else(|| config_parent.join(APP_DATA_DIR_NAME), PathBuf::from);
        Self::from_root_with_config(root_dir, config_path, false, false)
    }

    pub async fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.root_dir)
            .await
            .with_context(|| format!("unable to create {}", self.root_dir.display()))?;
        Ok(())
    }

    fn from_root(root_dir: PathBuf, portable: bool) -> Self {
        let config_path = root_dir.join(CONFIG_FILE_NAME);
        Self::from_root_with_config(root_dir, config_path, portable, true)
    }

    fn from_root_with_config(
        root_dir: PathBuf,
        config_path: PathBuf,
        portable: bool,
        generated_config: bool,
    ) -> Self {
        Self {
            config_path,
            cache_dir: root_dir.join(CACHE_DIR_NAME),
            root_dir,
            portable,
            generated_config,
        }
    }
}

fn installed_root_dir() -> Result<PathBuf> {
    if cfg!(target_os = "windows") {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            return Ok(installed_data_root(&PathBuf::from(local_app_data)));
        }

        return Ok(installed_data_root(
            &home_dir()?.join("AppData").join("Local"),
        ));
    }

    if cfg!(target_os = "macos") {
        return Ok(installed_data_root(
            &home_dir()?.join("Library").join("Application Support"),
        ));
    }

    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(installed_data_root(&PathBuf::from(data_home)));
    }

    Ok(installed_data_root(
        &home_dir()?.join(".local").join("share"),
    ))
}

fn installed_data_root(base_dir: &Path) -> PathBuf {
    base_dir.join(APP_NAME).join(APP_DATA_DIR_NAME)
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("unable to resolve user home directory"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::APP_DATA_DIR_NAME;

    #[test]
    fn default_paths_keep_config_and_cache_inside_data_root() {
        let root = PathBuf::from(APP_DATA_DIR_NAME);
        let paths = AppPaths::from_root(root.clone(), false);

        assert_eq!(paths.root_dir, root);
        assert_eq!(paths.config_path, paths.root_dir.join(CONFIG_FILE_NAME));
        assert_eq!(paths.cache_dir, paths.root_dir.join(CACHE_DIR_NAME));
        assert!(paths.generated_config);
    }

    #[test]
    fn installed_data_root_keeps_data_under_app_directory() {
        let paths = installed_data_root(&PathBuf::from("LocalAppData"));

        assert_eq!(
            paths,
            PathBuf::from("LocalAppData")
                .join(APP_NAME)
                .join(APP_DATA_DIR_NAME)
        );
    }

    #[test]
    fn config_override_uses_sibling_data_root_for_app_state() {
        let config_path = PathBuf::from("custom").join("configs.yaml");
        let paths = AppPaths::from_config_override(config_path.clone());

        assert_eq!(paths.config_path, config_path);
        assert_eq!(
            paths.root_dir,
            PathBuf::from("custom").join(APP_DATA_DIR_NAME)
        );
        assert_eq!(paths.cache_dir, paths.root_dir.join(CACHE_DIR_NAME));
        assert!(!paths.generated_config);
    }

    #[test]
    fn config_override_reuses_data_root_when_config_is_inside_it() {
        let config_path = PathBuf::from(APP_DATA_DIR_NAME).join("custom.yaml");
        let paths = AppPaths::from_config_override(config_path.clone());

        assert_eq!(paths.config_path, config_path);
        assert_eq!(paths.root_dir, PathBuf::from(APP_DATA_DIR_NAME));
        assert_eq!(paths.cache_dir, paths.root_dir.join(CACHE_DIR_NAME));
        assert!(!paths.generated_config);
    }

    #[test]
    fn config_override_reuses_data_root_when_config_is_nested_inside_it() {
        let config_path = PathBuf::from(APP_DATA_DIR_NAME)
            .join("profiles")
            .join("custom.yaml");
        let paths = AppPaths::from_config_override(config_path.clone());

        assert_eq!(paths.config_path, config_path);
        assert_eq!(paths.root_dir, PathBuf::from(APP_DATA_DIR_NAME));
        assert_eq!(paths.cache_dir, paths.root_dir.join(CACHE_DIR_NAME));
        assert!(!paths.generated_config);
    }
}
