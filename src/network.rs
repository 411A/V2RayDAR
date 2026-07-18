use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    process::Command,
    sync::{Mutex, OnceLock},
    time::Instant,
};

use crate::{
    constants::{
        DHCP_CONFLICT_CACHE_TTL, DHCP_CONFLICT_RANGE, GATEWAY_CHECK_TTL, INTERFACE_CACHE_TTL,
        ROUTE_PROBE_ADDR, ROUTE_PROBE_ADDR_FALLBACK,
    },
    model::RuntimeConfig,
};

type InterfaceIpCache = Option<(Instant, Vec<IpAddr>)>;
type GatewayCache = Option<(Instant, bool)>;
type ConflictCache = Option<(Instant, IpAddr, Vec<IpAddr>)>;

static INTERFACE_IP_CACHE: OnceLock<Mutex<InterfaceIpCache>> = OnceLock::new();
static GATEWAY_CACHE: OnceLock<Mutex<GatewayCache>> = OnceLock::new();
static CONFLICT_CACHE: OnceLock<Mutex<ConflictCache>> = OnceLock::new();

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SharingStatus {
    pub sharing: &'static str,
    pub discoverable: String,
    pub subscription_url: Option<String>,
    pub firewall: String,
    pub lan_conflicts: Vec<IpAddr>,
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

    let lan_conflicts = hosts
        .first()
        .and_then(|h| h.parse::<IpAddr>().ok())
        .map(cached_dhcp_conflicts)
        .unwrap_or_default();

    SharingStatus {
        sharing,
        discoverable,
        subscription_url,
        firewall,
        lan_conflicts,
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
    // Route-based detection gives the IP the OS routing table selected.
    // If it passes is_lan_reachable (private subnet, not loopback, etc.),
    // it is the correct answer — no further OS commands needed.
    if let Some(ip) = route_local_ip()
        && is_lan_reachable(ip)
    {
        return Some(ip);
    }

    // Fallback: interface enumeration. These need a gateway check because
    // virtual adapters may survive the name filter.
    let gw_ok = cached_has_default_gateway();
    detected_lan_ips()
        .into_iter()
        .find(|ip| is_lan_reachable(*ip) && (!gw_ok || has_default_gateway_cached()))
}

fn detected_lan_ips() -> Vec<IpAddr> {
    let mut ips = Vec::new();

    if let Some(ip) = route_local_ip().filter(|ip| is_lan_reachable(*ip)) {
        ips.push(ip);
    }

    for ip in interface_lan_ips() {
        if !ips.contains(&ip) {
            ips.push(ip);
        }
    }

    ips
}

fn discoverable_hosts_from_ips(ips: Vec<IpAddr>) -> Vec<String> {
    ips.into_iter()
        .next()
        .filter(|ip| is_lan_reachable(*ip))
        .map(|ip| ip.to_string())
        .into_iter()
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

// ---------------------------------------------------------------------------
// Route-based detection: sub-millisecond, zero process spawn.
// Binds a UDP socket and asks the OS routing table which local IP would be
// used to reach a remote address. No packets are sent.
// ---------------------------------------------------------------------------

fn route_local_ip() -> Option<IpAddr> {
    // Try primary probe address first
    if let Some(ip) = try_route_probe(ROUTE_PROBE_ADDR) {
        return Some(ip);
    }
    // Fallback: Cloudflare DNS (some networks block Google)
    if let Some(ip) = try_route_probe(ROUTE_PROBE_ADDR_FALLBACK) {
        return Some(ip);
    }
    // Last resort: try common gateway IPs on common subnets.
    // The connect() call itself triggers the routing table lookup even if
    // the target is unreachable — no packets are sent on UDP.
    for probe in &["192.168.1.1:80", "10.0.0.1:80", "192.168.0.1:80"] {
        if let Some(ip) = try_route_probe(probe) {
            return Some(ip);
        }
    }
    None
}

fn try_route_probe(addr: &str) -> Option<IpAddr> {
    let remote: SocketAddr = addr.parse().ok()?;
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect(remote).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

// ---------------------------------------------------------------------------
// IP validation — const fn, zero cost
// ---------------------------------------------------------------------------

const fn is_lan_reachable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_broadcast()
                && !ip.is_link_local()
                && !ip.is_multicast()
                && !ip.is_documentation()
                && is_private_lan_subnet(ip)
        }
        IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_unicast_link_local()
        }
    }
}

const fn is_private_lan_subnet(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    match octets[0] {
        10 => true,
        172 => octets[1] >= 16 && octets[1] <= 31,
        192 => octets[1] == 168,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Gateway check — cached with TTL (only runs OS command once per 60s)
// ---------------------------------------------------------------------------

fn cached_has_default_gateway() -> bool {
    has_default_gateway_cached() || has_default_gateway()
}

fn has_default_gateway_cached() -> bool {
    let cache = GATEWAY_CACHE.get_or_init(|| Mutex::new(None));
    let now = Instant::now();
    if let Ok(guard) = cache.lock()
        && let Some((cached_at, result)) = &*guard
        && now.duration_since(*cached_at) <= GATEWAY_CHECK_TTL
    {
        return *result;
    }
    let result = has_default_gateway();
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((now, result));
    }
    result
}

fn has_default_gateway() -> bool {
    if cfg!(target_os = "windows") {
        return command_output("route", &["print", "-4"]).is_none_or(|output| {
            output
                .lines()
                .any(|l| l.contains("0.0.0.0") && l.contains("0.0.0.0"))
        });
    }

    if cfg!(target_os = "macos") {
        return command_output("netstat", &["-rn"]).is_none_or(|output| {
            output
                .lines()
                .any(|l| l.starts_with("default") || l.starts_with("0.0.0.0"))
        });
    }

    // Linux, Android/Termux, BSD
    if let Some(output) = command_output("ip", &["route", "show", "default"]) {
        return !output.trim().is_empty();
    }
    if let Some(output) = command_output("route", &["-n"]) {
        return output
            .lines()
            .any(|l| l.contains("0.0.0.0") && l.contains("UG"));
    }
    if let Some(output) = command_output("netstat", &["-rn"]) {
        return output
            .lines()
            .any(|l| l.starts_with("default") || l.contains("UG"));
    }
    true
}

// ---------------------------------------------------------------------------
// DHCP conflict detection — cached with TTL (arp only once per 30s)
// ---------------------------------------------------------------------------

fn cached_dhcp_conflicts(ip: IpAddr) -> Vec<IpAddr> {
    let cache = CONFLICT_CACHE.get_or_init(|| Mutex::new(None));
    let now = Instant::now();
    if let Ok(guard) = cache.lock()
        && let Some((cached_at, cached_ip, result)) = &*guard
        && now.duration_since(*cached_at) <= DHCP_CONFLICT_CACHE_TTL
        && *cached_ip == ip
    {
        return result.clone();
    }
    let result = detect_dhcp_conflicts(ip);
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((now, ip, result.clone()));
    }
    result
}

pub fn detect_dhcp_conflicts(ip: IpAddr) -> Vec<IpAddr> {
    let IpAddr::V4(ipv4) = ip else {
        return Vec::new();
    };

    // `arp -a` works on all platforms without needing interface name
    let arp_output = command_output("arp", &["-a"]);

    let Some(output) = arp_output else {
        return Vec::new();
    };

    let mut conflicts = Vec::new();
    let target_octets = ipv4.octets();

    for line in output.lines() {
        let Some(ip_str) = line.split_whitespace().next() else {
            continue;
        };
        let Ok(found) = ip_str.parse::<Ipv4Addr>() else {
            continue;
        };
        let found_octets = found.octets();

        if found_octets[0..3] == target_octets[0..3] {
            let diff = (i32::from(found_octets[3]) - i32::from(target_octets[3])).unsigned_abs();
            #[allow(clippy::cast_possible_truncation)]
            if diff > 0 && diff <= u32::from(DHCP_CONFLICT_RANGE) {
                conflicts.push(IpAddr::V4(found));
            }
        }
    }

    conflicts
}

// ---------------------------------------------------------------------------
// Interface enumeration — cached with 5s TTL
// ---------------------------------------------------------------------------

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

    if cfg!(target_os = "macos") {
        return command_output("ifconfig", &["-a"])
            .map(|output| parse_ifconfig_ips(&output))
            .unwrap_or_default();
    }

    // Linux, Android/Termux, BSD
    let mut ips = command_output("ip", &["-o", "addr", "show", "scope", "global"])
        .map(|output| parse_ip_addr_ips(&output))
        .unwrap_or_default();
    if ips.is_empty() {
        ips = command_output("ifconfig", &["-a"])
            .map(|output| parse_ifconfig_ips(&output))
            .unwrap_or_default();
    }
    if ips.is_empty() {
        ips = command_output("hostname", &["-I"])
            .map(|output| {
                output
                    .split_whitespace()
                    .filter_map(|token| token.parse::<IpAddr>().ok())
                    .filter(|ip| is_lan_reachable(*ip))
                    .collect()
            })
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

// ---------------------------------------------------------------------------
// Windows ipconfig parser — filters virtual/disconnected adapters
// ---------------------------------------------------------------------------

fn parse_ipconfig_ips(output: &str) -> Vec<IpAddr> {
    let mut ips = Vec::new();
    let mut adapter_is_physical = true;

    for line in output.lines() {
        let trimmed = line.trim();

        if trimmed.ends_with(':') && !trimmed.starts_with("IPv4") && !trimmed.starts_with("IPv6") {
            adapter_is_physical = is_physical_adapter(trimmed.trim_end_matches(':'));
            continue;
        }

        if !adapter_is_physical {
            continue;
        }

        let lower = trimmed.to_ascii_lowercase();
        if lower.contains("media disconnected") || lower.contains("disconnected") {
            adapter_is_physical = false;
            continue;
        }

        if (lower.contains("ipv4") || lower.contains("ipv6"))
            && let Some(value) = trimmed.split_once(':').map(|(_, v)| v.trim())
            && let Some(ip_str) = value.split_whitespace().next()
            && let Some(ip) = parse_ip_token(ip_str)
        {
            ips.push(ip);
        }
    }

    ips
}

fn is_physical_adapter(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();

    let excluded = [
        "loopback",
        "pseudo",
        "tunnel",
        "teredo",
        "isatap",
        "6to4",
        "bluetooth",
        "virtual",
        "vmware",
        "hyper-v",
        "docker",
        "wsl",
        "hamachi",
        "npcap",
        "windivert",
        "vethernet",
        "cellular",
        "mobile",
        "wwan",
        "cell",
        "vpn",
        "tap",
        "tun",
        "wan miniport",
        "debug",
        "ras async",
        "code puppet",
    ];

    !excluded.iter().any(|pattern| lower.contains(pattern))
}

// ---------------------------------------------------------------------------
// Linux `ip` command parser — filters virtual interfaces
// ---------------------------------------------------------------------------

fn parse_ip_addr_ips(output: &str) -> Vec<IpAddr> {
    let mut ips = Vec::new();
    for line in output.lines() {
        let mut tokens = line.split_whitespace();

        // Format: "<index>: <iface> inet <addr>/..."
        let _index = tokens.next();

        let Some(iface) = tokens.next() else {
            continue;
        };

        if is_virtual_interface(iface) {
            continue;
        }

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

fn is_virtual_interface(name: &str) -> bool {
    let excluded_prefixes = [
        "lo", "dummy", "virbr", "docker", "br-", "veth", "tap", "tun", "wg", "sit", "gre", "ip6",
        "he-", "bond", "macvlan", "vlan", "awdl",    // macOS AirDrop
        "utun",    // macOS VPN tunnels
        "bridge",  // macOS bridges
        "llw",     // macOS low-latency wireless
        "p2p",     // macOS peer-to-peer
        "ap1",     // macOS access point (specific, not "ap")
        "rmnet",   // Android mobile data
        "ccmni",   // Android CCM network
        "ifb",     // Linux intermediate functional block
        "teql",    // Linux traffic equalizer
        "tunl",    // Linux IP-in-IP tunnel
        "erspan",  // Linux ERSPAN tunnel
        "gretap",  // Linux GRE tunnel
        "ip_vti",  // Linux VTI tunnel
        "ip6_vti", // Linux VTI IPv6 tunnel
    ];

    let lower = name.to_ascii_lowercase();
    excluded_prefixes
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

// ---------------------------------------------------------------------------
// ifconfig parser (macOS, BSD, Termux fallback) — filters virtual interfaces
// ---------------------------------------------------------------------------

fn parse_ifconfig_ips(output: &str) -> Vec<IpAddr> {
    let mut ips = Vec::new();
    let mut iface_is_physical = true;
    let mut tokens = output.split_whitespace();

    while let Some(token) = tokens.next() {
        if token.ends_with(':') && !matches!(token, "inet" | "inet6" | "addr:") {
            iface_is_physical = !is_virtual_interface(token.trim_end_matches(':'));
            continue;
        }

        if !iface_is_physical {
            continue;
        }

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

    #[test]
    fn filters_virtual_windows_adapters() {
        let ips = parse_ipconfig_ips(
            r"
Wireless LAN adapter Wi-Fi:
    IPv4 Address. . . . . . . . . . . : 192.168.1.87(Preferred)
    Subnet Mask . . . . . . . . . . . : 255.255.255.0

Ethernet adapter Bluetooth Network Connection:
    Media disconnected.

Tunnel adapter Teredo Tunneling Pseudo-Interface:
    IPv4 Address. . . . . . . . . . . : 192.168.1.100

Ethernet adapter VMware Network Adapter VMnet1:
    IPv4 Address. . . . . . . . . . . : 192.168.197.1

Loopback Pseudo-Interface 1:
    IPv4 Address. . . . . . . . . . . : 127.0.0.1
",
        );

        assert!(ips.contains(&"192.168.1.87".parse::<IpAddr>().expect("valid IP")));
        assert!(!ips.contains(&"192.168.1.100".parse::<IpAddr>().expect("valid IP")));
        assert!(!ips.contains(&"192.168.197.1".parse::<IpAddr>().expect("valid IP")));
        assert!(!ips.contains(&"127.0.0.1".parse::<IpAddr>().expect("valid IP")));
    }

    #[test]
    fn filters_disconnected_adapters() {
        let ips = parse_ipconfig_ips(
            r"
Ethernet adapter Ethernet:
    Media disconnected.

Wireless LAN adapter Wi-Fi:
    IPv4 Address. . . . . . . . . . . : 10.0.0.5
",
        );

        assert_eq!(ips, vec!["10.0.0.5".parse::<IpAddr>().expect("valid IP")]);
    }

    #[test]
    fn filters_virtual_linux_interfaces() {
        let ips = parse_ip_addr_ips(
            "1: lo    inet 127.0.0.1/8 scope host lo\n\
             2: docker0    inet 172.17.0.1/16 brd 172.17.255.255 scope global docker0\n\
             3: wlan0    inet 192.168.1.87/24 brd 192.168.1.255 scope global wlan0\n\
             4: virbr0    inet 192.168.122.1/24 brd 192.168.122.255 scope global virbr0\n",
        );

        assert_eq!(
            ips,
            vec!["192.168.1.87".parse::<IpAddr>().expect("valid IP")]
        );
    }

    #[test]
    fn filters_virtual_ifconfig_interfaces() {
        let ips = parse_ifconfig_ips(
            "lo: flags=73<UP,LOOPBACK,RUNNING>  mtu 65536\n\
             \tinet 127.0.0.1  netmask 255.0.0.0\n\n\
             docker0: flags=4163<UP,BROADCAST,RUNNING,MULTICAST>  mtu 1500\n\
             \tinet 172.17.0.1  netmask 255.255.0.0\n\n\
             wlan0: flags=4163<UP,BROADCAST,RUNNING,MULTICAST>  mtu 1500\n\
             \tinet 192.168.1.87  netmask 255.255.255.0\n",
        );

        assert_eq!(
            ips,
            vec!["192.168.1.87".parse::<IpAddr>().expect("valid IP")]
        );
    }

    #[test]
    fn is_physical_adapter_excludes_known_virtual() {
        assert!(is_physical_adapter("Wireless LAN adapter Wi-Fi"));
        assert!(is_physical_adapter("Ethernet adapter Ethernet"));
        assert!(!is_physical_adapter("Teredo Tunneling Pseudo-Interface"));
        assert!(!is_physical_adapter("Hyper-V Virtual Ethernet Adapter"));
        assert!(!is_physical_adapter("VMware Network Adapter VMnet1"));
        assert!(!is_physical_adapter("Docker Desktop VPN Tunnel"));
        assert!(!is_physical_adapter("Loopback Pseudo-Interface 1"));
        assert!(!is_physical_adapter("Bluetooth Network Connection"));
        assert!(!is_physical_adapter("Hamachi Network Interface"));
    }

    #[test]
    fn is_virtual_interface_excludes_known_virtual() {
        assert!(is_virtual_interface("lo"));
        assert!(is_virtual_interface("docker0"));
        assert!(is_virtual_interface("br-abc123"));
        assert!(is_virtual_interface("veth1234"));
        assert!(is_virtual_interface("virbr0"));
        assert!(is_virtual_interface("wg0"));
        assert!(!is_virtual_interface("wlan0"));
        assert!(!is_virtual_interface("eth0"));
        assert!(!is_virtual_interface("enp3s0"));
    }

    #[test]
    fn is_private_lan_subnet_covers_common_ranges() {
        assert!(is_private_lan_subnet("10.0.0.1".parse().unwrap()));
        assert!(is_private_lan_subnet("172.16.0.1".parse().unwrap()));
        assert!(is_private_lan_subnet("172.31.255.255".parse().unwrap()));
        assert!(is_private_lan_subnet("192.168.1.1".parse().unwrap()));
        assert!(!is_private_lan_subnet("8.8.8.8".parse().unwrap()));
        assert!(!is_private_lan_subnet("172.15.0.1".parse().unwrap()));
        assert!(!is_private_lan_subnet("172.32.0.1".parse().unwrap()));
        assert!(!is_private_lan_subnet("192.169.0.1".parse().unwrap()));
    }

    #[test]
    fn lan_conflicts_detected_in_sharing_status() {
        let config = runtime_config("192.168.1.87:27141", true);
        let hosts = vec!["192.168.1.87".to_string()];

        let status = sharing_status_from_hosts(&config, &hosts);
        assert_eq!(status.lan_conflicts.len(), 0);
    }
}
