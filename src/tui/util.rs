use std::path::Path;

use anyhow::{Context, Result};

use crate::{config::AppConfig, constants::BYTE_UNITS};

pub fn save_config(path: &Path, config: &AppConfig) -> Result<()> {
    let content = serde_yaml::to_string(config).context("unable to serialize config")?;
    std::fs::write(path, content)
        .with_context(|| format!("unable to write config to {}", path.display()))
}

pub fn human_bytes(bytes: u64) -> String {
    let mut value = bytes as f64;
    let mut unit = 0_usize;
    while value >= 1024.0 && unit < BYTE_UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", BYTE_UNITS[unit])
    } else {
        format!("{value:.2} {}", BYTE_UNITS[unit])
    }
}

pub fn bool_text(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}
