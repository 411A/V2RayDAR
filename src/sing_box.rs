use std::{path::PathBuf, process::Stdio};

use anyhow::{Context, Result, anyhow};
use tokio::process::Command;

use crate::{
    config::{AppConfig, ProbeMode},
    constants::{SING_BOX_VERSION, sing_box_download_url},
    paths::AppPaths,
};

#[cfg(target_os = "android")]
const TERMUX_SING_BOX_PATH: &str = "/data/data/com.termux/files/usr/bin/sing-box";

#[derive(Debug, Clone)]
pub struct SetupGuide {
    pub platform: &'static str,
    pub release_asset: String,
    pub executable_name: &'static str,
    pub example_paths: &'static [&'static str],
    pub notes: Vec<String>,
}

pub async fn active_probe_needs_setup(config: &AppConfig, _paths: &AppPaths) -> bool {
    if config.probe.mode != ProbeMode::Active {
        return false;
    }

    if should_setup_path(&config.probe.sing_box_path) {
        return true;
    }

    verify_path(&config.probe.sing_box_path).await.is_err()
}

pub fn apply_runtime_sing_box_path(config: &mut AppConfig) {
    if !should_setup_path(&config.probe.sing_box_path) {
        config.probe.sing_box_path = normalize_path(&config.probe.sing_box_path);
        config.probe.sing_box_path_auto = false;
        return;
    }

    if let Some(path) = bundled_sing_box_path().or_else(platform_default_sing_box_path) {
        config.probe.sing_box_path = path.to_string_lossy().to_string();
        config.probe.sing_box_path_auto = true;
    }
}

fn should_setup_path(value: &str) -> bool {
    let trimmed = normalize_path(value);
    trimmed.is_empty()
}

fn bundled_sing_box_path() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let executable_dir = executable.parent()?;
    let candidate = executable_dir.join(bundled_sing_box_file_name());
    candidate.is_file().then_some(candidate)
}

#[cfg(target_os = "windows")]
const fn bundled_sing_box_file_name() -> &'static str {
    "sing-box.exe"
}

#[cfg(not(target_os = "windows"))]
fn bundled_sing_box_file_name() -> &'static str {
    "sing-box"
}

#[cfg(target_os = "android")]
fn platform_default_sing_box_path() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(prefix) = std::env::var_os("PREFIX") {
        candidates.push(PathBuf::from(prefix).join("bin").join("sing-box"));
    }
    candidates.push(PathBuf::from(TERMUX_SING_BOX_PATH));

    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(not(target_os = "android"))]
const fn platform_default_sing_box_path() -> Option<PathBuf> {
    None
}

#[cfg(target_os = "windows")]
pub fn setup_guide() -> SetupGuide {
    SetupGuide {
        platform: "Windows",
        release_asset: format!("sing-box-{SING_BOX_VERSION}-windows-amd64.zip"),
        executable_name: "sing-box.exe",
        example_paths: &[
            r"C:\Tools\sing-box\sing-box.exe",
            r"C:\Program Files\v2rayN\sing-box.exe",
            "sing-box.exe",
        ],
        notes: vec![
            "Use the .exe file inside the Windows zip.".to_string(),
            "If you already use v2rayN, its installation folder may already contain sing-box.exe."
                .to_string(),
            "A command name is accepted only when it works from your terminal PATH.".to_string(),
        ],
    }
}

#[cfg(target_os = "macos")]
pub fn setup_guide() -> SetupGuide {
    SetupGuide {
        platform: "macOS",
        release_asset: format!(
            "sing-box-{SING_BOX_VERSION}-darwin-arm64.tar.gz for Apple Silicon, or darwin-amd64 for Intel"
        ),
        executable_name: "sing-box",
        example_paths: &[
            "/opt/homebrew/bin/sing-box",
            "/usr/local/bin/sing-box",
            "/Users/you/Downloads/sing-box/sing-box",
            "sing-box",
        ],
        notes: vec![
            "Use the sing-box file inside the Darwin archive, not a Windows .exe.".to_string(),
            "After extracting manually, run chmod +x sing-box if the file is not executable."
                .to_string(),
            "A command name is accepted only when it works from your terminal PATH.".to_string(),
        ],
    }
}

#[cfg(target_os = "android")]
pub fn setup_guide() -> SetupGuide {
    SetupGuide {
        platform: "Termux / Android",
        release_asset: format!("Termux package sing-box={SING_BOX_VERSION}"),
        executable_name: "sing-box",
        example_paths: &[
            "/data/data/com.termux/files/usr/bin/sing-box",
            "$HOME/bin/sing-box",
            "sing-box",
        ],
        notes: vec![
            format!("Install with: pkg install sing-box={SING_BOX_VERSION}"),
            "Use the Termux package path first; GitHub Android archives are only a fallback."
                .to_string(),
            "A command name is accepted only when it works from your Termux PATH.".to_string(),
        ],
    }
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
pub fn setup_guide() -> SetupGuide {
    SetupGuide {
        platform: "Linux",
        release_asset: format!(
            "sing-box-{SING_BOX_VERSION}-linux-amd64.tar.gz for x86_64, or linux-arm64 for ARM64"
        ),
        executable_name: "sing-box",
        example_paths: &[
            "/usr/local/bin/sing-box",
            "/usr/bin/sing-box",
            "/home/you/bin/sing-box",
            "sing-box",
        ],
        notes: vec![
            "Use the sing-box file inside the Linux archive, not the archive itself.".to_string(),
            "WSL2 Ubuntu is Linux: extract the Linux archive and point to the extracted 'sing-box' binary.".to_string(),
            "After extracting manually, run chmod +x sing-box if the file is not executable."
                .to_string(),
            "A command name is accepted only when it works from your terminal PATH.".to_string(),
        ],
    }
}

#[cfg(not(any(target_os = "windows", unix)))]
pub fn setup_guide() -> SetupGuide {
    SetupGuide {
        platform: "this operating system",
        release_asset: "the archive matching your operating system and CPU".to_string(),
        executable_name: "sing-box",
        example_paths: &["/full/path/to/sing-box", "sing-box"],
        notes: vec![
            "Use the executable file for your operating system, not a Windows .exe unless you are on Windows.".to_string(),
            "A command name is accepted only when it works from your terminal PATH.".to_string(),
        ],
    }
}

pub const fn recommended_version() -> &'static str {
    SING_BOX_VERSION
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

    if is_archive_path(&path) {
        return Err(anyhow!(
            "'{path}' is an archive, not a sing-box executable. Extract it and point to the file named 'sing-box' inside the archive."
        ));
    }

    let guide = setup_guide();
    let output = Command::new(&path)
        .arg("version")
        .stdin(Stdio::null())
        .output()
        .await
        .with_context(|| {
            format!(
                "unable to run '{path}'. On {}, use '{}' from {}; enter its full path or a PATH command. Download: {}",
                guide.platform,
                guide.executable_name,
                guide.release_asset,
                sing_box_download_url()
            )
        })?;

    if !output.status.success() {
        return Err(anyhow!(
            "'{path} version' exited with {}; enter a valid sing-box executable path",
            output.status
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reported_version = format!("{stdout}\n{stderr}");
    if !reported_version.contains(SING_BOX_VERSION) {
        tracing::warn!(
            sing_box_path = %path,
            recommended_version = SING_BOX_VERSION,
            "sing-box version differs from the recommended embedded version"
        );
    }

    Ok(())
}

fn is_archive_path(path: &str) -> bool {
    let path = std::path::Path::new(path);
    let extension = path.extension().and_then(|value| value.to_str());
    if extension.is_some_and(|ext| {
        ext.eq_ignore_ascii_case("tgz")
            || ext.eq_ignore_ascii_case("zip")
            || ext.eq_ignore_ascii_case("7z")
    }) {
        return true;
    }

    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| {
            name.to_ascii_lowercase().ends_with(".tar.gz")
                || name.to_ascii_lowercase().ends_with(".tar.xz")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_is_required_only_for_empty_paths() {
        assert!(should_setup_path(""));
        assert!(should_setup_path("   "));
        assert!(!should_setup_path("sing-box"));
        assert!(!should_setup_path("sing-box.exe"));
        assert!(!should_setup_path("/usr/local/bin/sing-box"));
    }

    #[test]
    fn guide_uses_platform_executable_name() {
        let guide = setup_guide();

        #[cfg(target_os = "windows")]
        assert_eq!(guide.executable_name, "sing-box.exe");

        #[cfg(not(target_os = "windows"))]
        assert_eq!(guide.executable_name, "sing-box");

        assert!(guide.release_asset.contains(SING_BOX_VERSION));
    }

    #[test]
    fn runtime_path_keeps_user_configured_path() {
        let mut config = AppConfig::default_for_first_run();
        config.probe.sing_box_path = " sing-box ".to_string();

        apply_runtime_sing_box_path(&mut config);

        assert_eq!(config.probe.sing_box_path, "sing-box");
        assert!(!config.probe.sing_box_path_auto);
    }
}
