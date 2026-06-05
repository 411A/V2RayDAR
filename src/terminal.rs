use crate::model::RuntimeState;

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

fn truncate(value: &str, width: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(width).collect::<String>();
    if chars.next().is_some() && width > 1 {
        format!("{}~", truncated.chars().take(width - 1).collect::<String>())
    } else {
        truncated
    }
}
