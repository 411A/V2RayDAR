use serde_yaml::Value as YamlValue;

use crate::constants::SUPPORTED_URI_SCHEMES;
use crate::convert::{clash_proxy_to_uri, decode_base64_to_string};
use crate::model::Candidate;
use crate::parser::parse_share_link;

/// Try to parse a subscription body as a Clash/Mihomo config.
///
/// Returns `Some(candidates)` if the body looks like a Clash config with a
/// `proxies` list, `None` otherwise so the caller can fall through to generic
/// extraction.
pub fn try_parse_clash_subscription(
    source: &str,
    priority: u32,
    body: &[u8],
) -> Option<Vec<Candidate>> {
    let text = String::from_utf8_lossy(body);

    // Try raw YAML first, then base64-decoded.
    if let Some(candidates) = try_parse_clash_yaml(source, priority, &text) {
        return Some(candidates);
    }

    let trimmed = text.trim();
    if let Some(decoded) = decode_base64_to_string(trimmed)
        && let Some(candidates) = try_parse_clash_yaml(source, priority, &decoded)
    {
        return Some(candidates);
    }

    None
}

fn try_parse_clash_yaml(source: &str, priority: u32, text: &str) -> Option<Vec<Candidate>> {
    let yaml: YamlValue = serde_yaml::from_str(text).ok()?;
    extract_clash_proxies(source, priority, &yaml)
}

fn extract_clash_proxies(source: &str, priority: u32, yaml: &YamlValue) -> Option<Vec<Candidate>> {
    let root = yaml.as_mapping()?;
    let proxies = root.get(YamlValue::String("proxies".into()))?;
    let seq = proxies.as_sequence()?;

    let mut candidates = Vec::new();
    for entry in seq {
        if let Some(uri) = proxy_entry_to_uri(entry)
            && SUPPORTED_URI_SCHEMES
                .iter()
                .any(|scheme| uri.to_ascii_lowercase().starts_with(scheme))
            && let Ok(candidate) = parse_share_link(source, priority, &uri)
        {
            candidates.push(candidate);
        }
    }

    if candidates.is_empty() {
        None
    } else {
        Some(candidates)
    }
}

fn proxy_entry_to_uri(entry: &YamlValue) -> Option<String> {
    clash_proxy_to_uri(entry).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clash_yaml_with_vmess_proxy() {
        let yaml = r#"
proxies:
  - name: "Test VMess"
    type: vmess
    server: example.com
    port: 443
    uuid: abc-123
    alterId: 0
    cipher: auto
    tls: true
    network: ws
    ws-opts:
      path: /ws
      headers:
        Host: example.com
    servername: example.com
"#;
        let candidates = try_parse_clash_subscription("test", 1, yaml.as_bytes()).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].protocol, "vmess");
        assert_eq!(candidates[0].endpoint.host, "example.com");
        assert_eq!(candidates[0].endpoint.port, 443);
    }

    #[test]
    fn parses_clash_yaml_with_multiple_proxies() {
        let yaml = r#"
proxies:
  - name: "VMess Node"
    type: vmess
    server: v1.example.com
    port: 443
    uuid: id-1
    alterId: 0
    cipher: auto
  - name: "Trojan Node"
    type: trojan
    server: t1.example.com
    port: 443
    password: pass123
    tls: true
  - name: "SS Node"
    type: ss
    server: s1.example.com
    port: 8388
    cipher: aes-256-gcm
    password: sspass
"#;
        let candidates = try_parse_clash_subscription("test", 1, yaml.as_bytes()).unwrap();
        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[0].protocol, "vmess");
        assert_eq!(candidates[1].protocol, "trojan");
        assert_eq!(candidates[2].protocol, "ss");
    }

    #[test]
    fn returns_none_for_non_clash_yaml() {
        let yaml = r#"
some_key: "not a clash config"
another_key: 123
"#;
        assert!(try_parse_clash_subscription("test", 1, yaml.as_bytes()).is_none());
    }

    #[test]
    fn returns_none_for_invalid_yaml() {
        assert!(try_parse_clash_subscription("test", 1, b"not: valid: yaml: [[[[").is_none());
    }

    #[test]
    fn parses_base64_encoded_clash_config() {
        let yaml = r#"
proxies:
  - name: "Encoded Node"
    type: vmess
    server: encoded.example.com
    port: 443
    uuid: encoded-id
    alterId: 0
    cipher: auto
"#;
        let encoded =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, yaml.as_bytes());
        let candidates = try_parse_clash_subscription("test", 1, encoded.as_bytes()).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].endpoint.host, "encoded.example.com");
    }

    #[test]
    fn skips_unsupported_proxy_types() {
        let yaml = r#"
proxies:
  - name: "Supported VMess"
    type: vmess
    server: v1.example.com
    port: 443
    uuid: id-1
    alterId: 0
    cipher: auto
  - name: "WireGuard"
    type: wireguard
    server: wg.example.com
    port: 51820
    private-key: test
"#;
        let candidates = try_parse_clash_subscription("test", 1, yaml.as_bytes()).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].protocol, "vmess");
    }

    #[test]
    fn parses_real_mihomo_vless_ws_proxy() {
        // This is a real-world proxy entry from a mihomo subscription
        let yaml = r#"
proxies:
  - encryption: none
    name: "CH1 test"
    network: ws
    port: 80
    server: 172.67.128.147
    type: vless
    udp: true
    uuid: 01dfcada-2cef-4b85-86f2-7183df189918
    ws-opts:
      headers:
        Host: example.workers.dev
        User-Agent: Mozilla/5.0
      path: /test-path
    xudp: true
    servername: example.workers.dev
"#;
        let candidates = try_parse_clash_subscription("test", 1, yaml.as_bytes()).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].protocol, "vless");
        assert_eq!(candidates[0].endpoint.host, "172.67.128.147");
        assert_eq!(candidates[0].endpoint.port, 80);
        assert_eq!(candidates[0].name, "CH1 test");

        // Verify the generated URI is valid
        let uri = &candidates[0].uri;
        assert!(uri.starts_with("vless://"));
        assert!(uri.contains("01dfcada-2cef-4b85-86f2-7183df189918"));
        assert!(uri.contains("type=ws"));
        assert!(uri.contains("path=/test-path"));
    }

    #[test]
    fn parses_mihomo_vless_with_tls() {
        let yaml = r#"
proxies:
  - client-fingerprint: chrome
    encryption: none
    name: "IT1 test"
    network: ws
    port: 443
    server: example.workers.dev
    tls: true
    type: vless
    udp: true
    uuid: 2784597c-ef5b-45b2-bdb5-1b046fb9c461
    ws-opts:
      headers:
        Host: example.workers.dev
      path: /
    xudp: true
    servername: example.workers.dev
"#;
        let candidates = try_parse_clash_subscription("test", 1, yaml.as_bytes()).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].protocol, "vless");

        // Verify TLS is preserved in the URI
        let uri = &candidates[0].uri;
        assert!(uri.contains("security=tls"));
        assert!(uri.contains("fp=chrome"));
        assert!(uri.contains("sni=example.workers.dev"));
    }

    #[test]
    fn parses_full_mihomo_config_with_groups_and_rules() {
        let yaml = r#"
proxies:
  - name: "Node1"
    type: vmess
    server: server1.com
    port: 443
    uuid: id-1
    alterId: 0
    cipher: auto
  - name: "Node2"
    type: vless
    server: server2.com
    port: 443
    uuid: id-2
proxy-groups:
  - name: auto
    type: url-test
    proxies:
      - Node1
      - Node2
rules:
  - MATCH,auto
"#;
        let candidates = try_parse_clash_subscription("test", 1, yaml.as_bytes()).unwrap();
        // Should extract both proxies from the proxies: section
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].protocol, "vmess");
        assert_eq!(candidates[1].protocol, "vless");
    }
}
