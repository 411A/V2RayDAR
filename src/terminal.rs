use std::{path::Path, time::Instant};

use crate::{
    config::AppConfig,
    constants::{LOCALHOST_IP, SETTING_GUIDES},
    model::{ProgressEvent, RuntimeState},
    network::discoverable_subscription_url,
    paths::AppPaths,
};

pub fn print_log(message: impl AsRef<str>) {
    println!(
        "{} {}",
        chrono::Local::now().format("%H:%M:%S"),
        message.as_ref()
    );
}

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
            .map_or_else(|| "-".to_string(), |value| format!("{value} ms"));
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

pub fn print_startup(config: &AppConfig, paths: &AppPaths, verbose: bool) {
    let local_url = config.subscription_url(LOCALHOST_IP, true);

    println!("V2RayDAR");
    println!(
        "Mode: {}",
        if paths.portable {
            "portable"
        } else {
            "installed"
        }
    );
    println!("Data folder: {}", display_path(&paths.root_dir));
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
        if let Some(url) = discoverable_subscription_url(&crate::model::RuntimeConfig::from(config))
        {
            println!("LAN subscription: {url}");
        } else {
            println!(
                "LAN URL: use this machine's LAN IP with port {}",
                config.bind.port()
            );
        }
    } else {
        println!("LAN sharing: disabled");
    }

    println!("Settings guide:");
    for guide in SETTING_GUIDES {
        println!("  {:<22} {}", guide.label, guide.help);
    }
    if !verbose {
        println!("Use --verbose for detailed fetch/probe trace logs.");
    }
    println!();
}

pub struct PlainProgressReporter {
    tested: usize,
    working: usize,
    next_probe_report: usize,
    last_probe_report: Instant,
}

impl PlainProgressReporter {
    pub fn new() -> Self {
        Self {
            tested: 0,
            working: 0,
            next_probe_report: 500,
            last_probe_report: Instant::now(),
        }
    }

    pub fn on_event(&mut self, event: &ProgressEvent) {
        match event {
            ProgressEvent::LiveLog(message) => Self::on_log(message),
            ProgressEvent::ProbeDelta { tested, working } => self.on_probe_delta(*tested, *working),
            ProgressEvent::RankedSnapshot(ranked) => {
                print_log(format!(
                    "Early results ready: {} configs published.",
                    ranked.len()
                ));
            }
            ProgressEvent::WorkingConfigsFound { .. } | ProgressEvent::FetchedDelta(_) => {}
        }
    }

    fn on_log(message: &str) {
        if should_print_plain_progress(message) {
            print_log(message);
        }
    }

    fn on_probe_delta(&mut self, tested: usize, working: usize) {
        self.tested = self.tested.saturating_add(tested);
        self.working = self.working.saturating_add(working);
        if self.tested < self.next_probe_report && self.last_probe_report.elapsed().as_secs() < 5 {
            return;
        }

        print_log(format!(
            "Probe progress: {} checked, {} working.",
            self.tested, self.working
        ));
        while self.next_probe_report <= self.tested {
            self.next_probe_report = self.next_probe_report.saturating_add(500);
        }
        self.last_probe_report = Instant::now();
    }
}

fn should_print_plain_progress(message: &str) -> bool {
    message.starts_with("Refresh started")
        || message.starts_with("Subscription load:")
        || message.starts_with("Loaded subscription")
        || message.starts_with("Subscription loading finished")
        || message.starts_with("Probe skipped")
        || message.starts_with("Probe started")
        || message.starts_with("Prepared active test")
        || message.starts_with("Early publish")
        || message.starts_with("Early stop")
        || message.starts_with("Active test finished")
        || message.starts_with("Probe queue finished")
        || message.starts_with("sing-box unavailable")
        || message.starts_with("sing-box rejected")
        || message.starts_with("Active probe batch failed")
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

pub fn print_ping_results(results: &[crate::model::RankedConfig]) {
    if results.is_empty() {
        println!("No results. Provide valid config URIs with --ping or --ping-file.");
        return;
    }

    let reachable = results.iter().filter(|r| r.reachable).count();
    println!(
        "Pinged {} configs; {} reachable.\n",
        results.len(),
        reachable
    );

    println!(
        "{:<5} {:<10} {:<28} {:<22} {:>10} {:>10}",
        "rank", "proto", "name", "endpoint", "tcp", "http"
    );

    for item in results {
        let endpoint = format!("{}:{}", item.endpoint.host, item.endpoint.port);
        let tcp = item
            .latency_ms
            .map_or_else(|| "-".to_string(), |v| format!("{v} ms"));
        let http = item.http_status.map_or_else(
            || "-".to_string(),
            |s| {
                item.latency_ms
                    .map_or_else(|| format!("HTTP {s}"), |_| format!("HTTP {s}"))
            },
        );
        println!(
            "{:<5} {:<10} {:<28} {:<22} {:>10} {:>10}",
            item.rank,
            truncate(&item.protocol, 10),
            truncate(&item.name, 28),
            truncate(&endpoint, 22),
            tcp,
            http
        );
    }
}
