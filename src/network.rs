use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    process::Command,
    sync::{Mutex, OnceLock},
    time::Instant,
};

use crate::{
    constants::{INTERFACE_CACHE_TTL, ROUTE_PROBE_ADDR},
    model::RuntimeConfig,
};

type InterfaceIpCache = Option<(Instant, Vec<IpAddr>)>;

static INTERFACE_IP_CACHE: OnceLock<Mutex<InterfaceIpCache>> = OnceLock::new();

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SharingStatus {
    pub sharing: &'static str,
    pub discoverable: String,
    pub subscription_url: Option<String>,
    pub firewall: String,
}

pub fn sharing_status(config: &RuntimeConfig) -> SharingStatus {
    let hosts = discoverable_hosts(config);
    sharing_status_from_hosts(config, &hosts)
}

fn sharing_status_from_hosts(config: &RuntimeConfig, hosts: &[String]) -> SharingStatus {
    let sharing = if config.sharing_enabled { "on" } else { "off" };
    let subscription_url = config
        .sharing_enabled
        .then(|| format_discoverable_url(config, hosts))
        .filter(|url| !url.is_empty());
    let discoverable = match (config.sharing_enabled, hosts.is_empty()) {
        (true, false) => format!(
            "yes {}",
            subscription_url
                .as_deref()
                .expect("discoverable host should format a URL")
        ),
        (true, true) => "no reachable LAN IP found".to_string(),
        (false, _) => "no".to_string(),
    };
    let firewall = if config.sharing_enabled && !hosts.is_empty() {
        if config.proxy_enabled && config.proxy_discoverable {
            format!("allowed TCP {}, {}", config.bind.port(), config.proxy_port)
        } else {
            format!("allowed TCP {}", config.bind.port())
        }
    } else if config.proxy_enabled && config.proxy_discoverable {
        format!("allowed TCP {}", config.proxy_port)
    } else {
        "not required for local-only bind".to_string()
    };

    SharingStatus {
        sharing,
        discoverable,
        subscription_url,
        firewall,
    }
}

pub fn discoverable_hosts(config: &RuntimeConfig) -> Vec<String> {
    let bind_ip = config.bind.ip();
    if is_lan_reachable(bind_ip) && !bind_ip.is_unspecified() {
        return vec![bind_ip.to_string()];
    }

    if bind_ip.is_loopback() && config.sharing_enabled {
        return primary_lan_ip()
            .map(|ip| vec![ip.to_string()])
            .unwrap_or_default();
    }

    if !bind_ip.is_unspecified() {
        return Vec::new();
    }

    discoverable_hosts_from_ips(detected_lan_ips())
}

pub fn primary_lan_ip() -> Option<IpAddr> {
    detected_lan_ips()
        .into_iter()
        .find(|ip| is_lan_reachable(*ip))
}

fn detected_lan_ips() -> Vec<IpAddr> {
    let mut ips = Vec::new();
    if let Some(ip) = route_local_ip().filter(|ip| is_lan_reachable(*ip)) {
        ips.push(ip);
    }
    ips.extend(interface_lan_ips());
    ips
}

fn discoverable_hosts_from_ips(ips: Vec<IpAddr>) -> Vec<String> {
    let mut seen = HashSet::new();
    ips.into_iter()
        .filter(|ip| is_lan_reachable(*ip))
        .filter(|ip| seen.insert(*ip))
        .take(1)
        .map(|ip| ip.to_string())
        .collect()
}

pub fn discoverable_subscription_url(config: &RuntimeConfig) -> Option<String> {
    discoverable_hosts(config)
        .first()
        .map(|host| config.subscription_url(host, true))
}

fn format_discoverable_url(config: &RuntimeConfig, hosts: &[String]) -> String {
    hosts
        .first()
        .map(|host| config.subscription_url(host, true))
        .unwrap_or_default()
}

fn route_local_ip() -> Option<IpAddr> {
    let remote: SocketAddr = ROUTE_PROBE_ADDR.parse().ok()?;
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect(remote).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

const fn is_lan_reachable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_broadcast()
                && !ip.is_link_local()
                && !ip.is_multicast()
                && !ip.is_documentation()
        }
        IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_unicast_link_local()
        }
    }
}

fn interface_lan_ips() -> Vec<IpAddr> {
    os_interface_ips()
        .into_iter()
        .filter(|ip| is_lan_reachable(*ip))
        .collect()
}

fn os_interface_ips() -> Vec<IpAddr> {
    let now = Instant::now();
    let cache = INTERFACE_IP_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock()
        && let Some((cached_at, ips)) = &*guard
        && now.duration_since(*cached_at) <= INTERFACE_CACHE_TTL
    {
        return ips.clone();
    }

    let ips = read_os_interface_ips();
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((now, ips.clone()));
    }
    ips
}

fn read_os_interface_ips() -> Vec<IpAddr> {
    if cfg!(target_os = "windows") {
        return command_output("ipconfig", &[])
            .map(|output| parse_ipconfig_ips(&output))
            .unwrap_or_default();
    }

    let mut ips = command_output("ip", &["-o", "addr", "show", "scope", "global"])
        .map(|output| parse_ip_addr_ips(&output))
        .unwrap_or_default();
    if ips.is_empty() {
        ips = command_output("ifconfig", &["-a"])
            .map(|output| parse_ifconfig_ips(&output))
            .unwrap_or_default();
    }
    ips
}

fn command_output(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
}

fn parse_ipconfig_ips(output: &str) -> Vec<IpAddr> {
    output
        .lines()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("ipv4") || lower.contains("ipv6")
        })
        .filter_map(|line| line.split_once(':').map(|(_, value)| value.trim()))
        .filter_map(|value| value.split_whitespace().next())
        .filter_map(parse_ip_token)
        .collect()
}

fn parse_ip_addr_ips(output: &str) -> Vec<IpAddr> {
    let mut ips = Vec::new();
    for line in output.lines() {
        let mut tokens = line.split_whitespace();
        while let Some(token) = tokens.next() {
            if matches!(token, "inet" | "inet6")
                && let Some(address) = tokens.next()
                && let Some(ip) = parse_ip_token(address)
            {
                ips.push(ip);
            }
        }
    }
    ips
}

fn parse_ifconfig_ips(output: &str) -> Vec<IpAddr> {
    let mut ips = Vec::new();
    let mut tokens = output.split_whitespace();
    while let Some(token) = tokens.next() {
        if matches!(token, "inet" | "inet6")
            && let Some(address) = tokens.next()
            && let Some(ip) = parse_ip_token(address)
        {
            ips.push(ip);
        }
    }
    ips
}

fn parse_ip_token(value: &str) -> Option<IpAddr> {
    let value = value
        .trim()
        .trim_start_matches("addr:")
        .split('/')
        .next()?
        .split('%')
        .next()?
        .split('(')
        .next()?
        .trim();
    value.parse::<IpAddr>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        constants::{
            DEFAULT_ACCEPTED_STATUSES, DEFAULT_ACTIVE_TIMEOUT_MS, DEFAULT_DOWNLOAD_BYTES_LIMIT,
            DEFAULT_ENCODED_SUBSCRIPTION, DEFAULT_FETCH_CONCURRENCY, DEFAULT_FETCH_TIMEOUT_MS,
            DEFAULT_MAX_SUBSCRIPTION_BYTES, DEFAULT_PRIORITIZE_STABILITY,
            DEFAULT_PROBE_CONCURRENCY, DEFAULT_REFRESH_SECONDS, DEFAULT_RETURN_CONFIGS_ASAP,
            DEFAULT_SCAN_ALL_CONFIGS, DEFAULT_STARTUP_TIMEOUT_MS, DEFAULT_TEST_URL, DEFAULT_TOP_N,
        },
        model::RuntimeConfig,
    };

    fn runtime_config(bind: &str, sharing_enabled: bool) -> RuntimeConfig {
        RuntimeConfig {
            bind: bind.parse().expect("valid bind"),
            top_n: DEFAULT_TOP_N,
            refresh_seconds: DEFAULT_REFRESH_SECONDS,
            encoded_subscription: DEFAULT_ENCODED_SUBSCRIPTION,
            prioritize_stability: DEFAULT_PRIORITIZE_STABILITY,
            return_configs_asap: DEFAULT_RETURN_CONFIGS_ASAP,
            scan_all_configs: DEFAULT_SCAN_ALL_CONFIGS,
            fetch_timeout_ms: DEFAULT_FETCH_TIMEOUT_MS,
            fetch_concurrency: DEFAULT_FETCH_CONCURRENCY,
            max_subscription_bytes: DEFAULT_MAX_SUBSCRIPTION_BYTES,
            sharing_enabled,
            require_token: false,
            token: String::new(),
            probe_mode: "active".to_string(),
            speedtest_enabled: false,
            probe_concurrency: DEFAULT_PROBE_CONCURRENCY,
            probe_batch_size: None,
            active_timeout_ms: DEFAULT_ACTIVE_TIMEOUT_MS,
            startup_timeout_ms: DEFAULT_STARTUP_TIMEOUT_MS,
            test_url: DEFAULT_TEST_URL.to_string(),
            accepted_statuses: DEFAULT_ACCEPTED_STATUSES.to_vec(),
            download_bytes_limit: DEFAULT_DOWNLOAD_BYTES_LIMIT,
            subscription_count: 0,
            enabled_subscription_count: 0,
            proxy_enabled: false,
            proxy_port: 27910,
            proxy_discoverable: false,
        }
    }

    #[test]
    fn uses_specific_lan_bind_as_discoverable_host() {
        let config = runtime_config("192.168.1.87:27141", true);

        assert_eq!(
            discoverable_hosts(&config),
            vec!["192.168.1.87".to_string()]
        );
    }

    #[test]
    fn disabled_loopback_bind_is_not_discoverable() {
        let config = runtime_config("127.0.0.1:27141", false);
        let hosts = discoverable_hosts(&config);

        let status = sharing_status_from_hosts(&config, &hosts);
        assert!(hosts.is_empty());
        assert_eq!(status.discoverable, "no");
        assert_eq!(status.firewall, "not required for local-only bind");
    }

    #[test]
    fn enabled_loopback_bind_can_display_lan_sharing_url() {
        let config = runtime_config("127.0.0.1:27141", true);
        let hosts = vec!["192.168.43.1".to_string()];

        let status = sharing_status_from_hosts(&config, &hosts);
        assert_eq!(
            status.discoverable,
            "yes http://192.168.43.1:27141/subscription.txt"
        );
        assert_eq!(status.firewall, "allowed TCP 27141");
    }

    #[test]
    fn keeps_only_primary_detected_host() {
        let config = runtime_config("0.0.0.0:27141", true);
        let hosts = discoverable_hosts_from_ips(vec![
            "192.168.1.87".parse::<IpAddr>().expect("valid IP"),
            "10.5.0.2".parse::<IpAddr>().expect("valid IP"),
            "192.168.197.1".parse::<IpAddr>().expect("valid IP"),
        ]);

        let status = sharing_status_from_hosts(&config, &hosts);
        assert_eq!(hosts, vec!["192.168.1.87".to_string()]);
        assert_eq!(
            status.discoverable,
            "yes http://192.168.1.87:27141/subscription.txt"
        );
    }

    #[test]
    fn parses_windows_ipconfig_ipv4_lines() {
        let ips = parse_ipconfig_ips(
            r"
Wireless LAN adapter Wi-Fi:
    IPv4 Address. . . . . . . . . . . : 192.168.43.1(Preferred)
    Subnet Mask . . . . . . . . . . . : 255.255.255.0
",
        );

        assert!(ips.contains(&"192.168.43.1".parse::<IpAddr>().expect("valid IP")));
    }

    #[test]
    fn parses_linux_ip_addr_lines() {
        let ips = parse_ip_addr_ips(
            "2: wlan0    inet 192.168.1.87/24 brd 192.168.1.255 scope global wlan0\n",
        );

        assert_eq!(
            ips,
            vec!["192.168.1.87".parse::<IpAddr>().expect("valid IP")]
        );
    }

    #[test]
    fn parses_ifconfig_addr_tokens() {
        let ips = parse_ifconfig_ips(
            "wlan0 Link encap:Ethernet inet addr:192.168.43.1 Bcast:192.168.43.255",
        );

        assert_eq!(
            ips,
            vec!["192.168.43.1".parse::<IpAddr>().expect("valid IP")]
        );
    }
}
