use std::process::Stdio;

use anyhow::{Context, Result, anyhow};
use tokio::process::Command;

use crate::{
    config::{AppConfig, ProbeMode},
    constants::{SING_BOX_VERSION, sing_box_download_url},
    paths::AppPaths,
};

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

fn should_setup_path(value: &str) -> bool {
    let trimmed = normalize_path(value);
    trimmed.is_empty()
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

pub fn recommended_version() -> &'static str {
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

    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".tar.xz")
        || lower.ends_with(".zip")
        || lower.ends_with(".7z")
    {
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
}
