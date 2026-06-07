use std::path::Path;

use crate::{
    config::AppConfig,
    constants::{LOCALHOST_IP, SETTING_GUIDES},
    model::RuntimeState,
    paths::AppPaths,
};

pub fn print_summary(state: &RuntimeState, top_n: usize) {
    println!(
        "Checked {} configs; {} reachable; {} fetch errors.",
        state.total_candidates,
        state.reachable_candidates,
        state.fetch_errors.len()
    );

    if !state.fetch_errors.is_empty() {
        for error in &state.fetch_errors {
            println!("fetch error: {error}");
        }
    }

    println!(
        "{:<5} {:<8} {:<10} {:<28} {:<22} {:<12} {:>10}",
        "rank", "prio", "proto", "name", "endpoint", "validation", "latency"
    );

    for item in state
        .ranked
        .iter()
        .filter(|item| item.reachable)
        .take(top_n)
    {
        let endpoint = format!("{}:{}", item.endpoint.host, item.endpoint.port);
        let latency = item
            .latency_ms
            .map(|value| format!("{value} ms"))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:<5} {:<8} {:<10} {:<28} {:<22} {:<12} {:>10}",
            item.rank,
            item.priority,
            truncate(&item.protocol, 10),
            truncate(&item.name, 28),
            truncate(&endpoint, 22),
            truncate(&item.validation, 12),
            latency
        );
    }
}

pub fn print_startup(config: &AppConfig, paths: &AppPaths) {
    let local_url = config.subscription_url(LOCALHOST_IP, false);

    println!("V2RayDAR");
    println!(
        "Mode: {}",
        if paths.portable {
            "portable"
        } else {
            "installed"
        }
    );
    println!("App folder: {}", display_path(&paths.root_dir));
    println!("Config: {}", display_path(&paths.config_path));
    println!("Local subscription: {local_url}");

    if config.sharing.enabled {
        println!(
            "LAN sharing: enabled ({})",
            if config.sharing.require_token {
                "token required"
            } else {
                "open on LAN"
            }
        );
        println!(
            "LAN URL: use this machine's LAN IP with port {}",
            config.bind.port()
        );
    } else {
        println!("LAN sharing: disabled");
    }

    println!("Settings guide:");
    for guide in SETTING_GUIDES {
        println!("  {:<22} {}", guide.label, guide.help);
    }

    println!();
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn truncate(value: &str, width: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(width).collect::<String>();
    if chars.next().is_some() && width > 1 {
        format!("{}~", truncated.chars().take(width - 1).collect::<String>())
    } else {
        truncated
    }
}
