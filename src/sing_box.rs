use std::process::Stdio;

use anyhow::{Context, Result, anyhow};
use tokio::process::Command;

use crate::{
    config::{AppConfig, ProbeMode},
    constants::SING_BOX_DOWNLOAD_URL,
    paths::AppPaths,
};

#[derive(Debug, Clone, Copy)]
pub struct SetupGuide {
    pub platform: &'static str,
    pub release_asset: &'static str,
    pub executable_name: &'static str,
    pub example_paths: &'static [&'static str],
    pub notes: &'static [&'static str],
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
        release_asset: "sing-box-{version}-windows-amd64.zip",
        executable_name: "sing-box.exe",
        example_paths: &[
            r"C:\Tools\sing-box\sing-box.exe",
            r"C:\Program Files\v2rayN\sing-box.exe",
            "sing-box.exe",
        ],
        notes: &[
            "Use the .exe file inside the Windows zip.",
            "If you already use v2rayN, its installation folder may already contain sing-box.exe.",
            "A command name is accepted only when it works from your terminal PATH.",
        ],
    }
}

#[cfg(target_os = "macos")]
pub fn setup_guide() -> SetupGuide {
    SetupGuide {
        platform: "macOS",
        release_asset: "sing-box-{version}-darwin-arm64.tar.gz for Apple Silicon, or darwin-amd64 for Intel",
        executable_name: "sing-box",
        example_paths: &[
            "/opt/homebrew/bin/sing-box",
            "/usr/local/bin/sing-box",
            "/Users/you/Downloads/sing-box/sing-box",
            "sing-box",
        ],
        notes: &[
            "Use the sing-box file inside the Darwin archive, not a Windows .exe.",
            "After extracting manually, run chmod +x sing-box if the file is not executable.",
            "A command name is accepted only when it works from your terminal PATH.",
        ],
    }
}

#[cfg(target_os = "android")]
pub fn setup_guide() -> SetupGuide {
    SetupGuide {
        platform: "Termux / Android",
        release_asset: "sing-box-{version}-android-arm64.tar.gz for most phones, or the Android archive matching your CPU",
        executable_name: "sing-box",
        example_paths: &[
            "/data/data/com.termux/files/usr/bin/sing-box",
            "$HOME/bin/sing-box",
            "sing-box",
        ],
        notes: &[
            "Use an Android archive, not a Linux desktop archive and not a Windows .exe.",
            "After extracting manually, run chmod +x sing-box if the file is not executable.",
            "A command name is accepted only when it works from your Termux PATH.",
        ],
    }
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
pub fn setup_guide() -> SetupGuide {
    SetupGuide {
        platform: "Linux",
        release_asset: "sing-box-{version}-linux-amd64.tar.gz for x86_64, or linux-arm64 for ARM64",
        executable_name: "sing-box",
        example_paths: &[
            "/usr/local/bin/sing-box",
            "/usr/bin/sing-box",
            "/home/you/bin/sing-box",
            "sing-box",
        ],
        notes: &[
            "Use the sing-box file inside the Linux archive, not the archive itself.",
            "WSL2 Ubuntu is Linux: extract the Linux archive and point to the extracted 'sing-box' binary.",
            "After extracting manually, run chmod +x sing-box if the file is not executable.",
            "A command name is accepted only when it works from your terminal PATH.",
        ],
    }
}

#[cfg(not(any(target_os = "windows", unix)))]
pub fn setup_guide() -> SetupGuide {
    SetupGuide {
        platform: "this operating system",
        release_asset: "the archive matching your operating system and CPU",
        executable_name: "sing-box",
        example_paths: &["/full/path/to/sing-box", "sing-box"],
        notes: &[
            "Use the executable file for your operating system, not a Windows .exe unless you are on Windows.",
            "A command name is accepted only when it works from your terminal PATH.",
        ],
    }
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
    let status = Command::new(&path)
        .arg("version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .with_context(|| {
            format!(
                "unable to run '{path}'. On {}, use '{}' from {}; enter its full path or a PATH command. Download: {SING_BOX_DOWNLOAD_URL}",
                guide.platform, guide.executable_name, guide.release_asset
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
    }
}
