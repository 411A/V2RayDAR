<p align="center">
  <a href="https://deepwiki.com/411A/V2RayDAR">
    <img src="https://deepwiki.com/badge.svg" alt="Ask DeepWiki About V2RayDAR">
  </a>
</p>

<p align="center">
  <img src="../assets/V2RayDAR_logo_v1.png" alt="V2RayDAR logo" width="200" height="200">
</p>

# V2RayDAR Detailed Guide

V2RayDAR is a Rust CLI/TUI application that fetches V2Ray / Clash / Mihomo subscription sources, extracts supported share links, checks which configs work on your current network, ranks the working results, and publishes the best ones through a local subscription endpoint — both as V2Ray share-links and as full Mihomo YAML configs.

The name means **V2Ray Detection And Reconnaissance** and is pronounced like `v2ray` + `radar`.

This document is the detailed user and developer guide. The short, ready-to-use guide is in [README.md](../README.md).


## Quick Install

Copy the command for your OS into a terminal. The installer detects your platform, downloads the latest release with bundled `sing-box`, and sets everything up. Portable mode installs into `Desktop/V2RayDAR` when a Desktop folder exists, otherwise `~/V2RayDAR`. User mode installs the binary to `~/.local/bin`.

**Portable** (recommended) — everything in one folder, run with `--portable`:
```bash
# Linux
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh

# macOS
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh

# Windows
irm https://raw.githubusercontent.com/411A/V2RayDAR/main/install.ps1 | iex
```

**User install** — binary to `~/.local/bin`, data in home:
```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh -s -- --user

# Windows
irm https://raw.githubusercontent.com/411A/V2RayDAR/main/install.ps1 | iex
# Then choose option 2 when prompted
```

**Android / Termux:**
```bash
# Same Linux binary — install sing-box, then run the installer
pkg update -y && pkg install -y curl tar sing-box=1.13.13
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh
```

**Manual download** — grab the archive for your OS from [Releases](https://github.com/411A/V2RayDAR/releases/latest) and run with `--portable`.

The installer verifies SHA-256 checksums, detects existing installations and offers to update (preserving `configs.yaml`, `data.db`, and `v2raydar_data/`), and never requires sudo by default.

---

## Scope And Responsibility

V2RayDAR does not create, sell, host, or distribute V2Ray-compatible configs. It only scans subscription sources that you configure and republishes the working configs it finds on your own machine.

The app is published as-is, without any warranty. You are responsible for the subscription URLs and configs you scan, import, and connect to. A proxy or server operator may be able to intercept your traffic and read traffic that is not encrypted end-to-end.

## What V2RayDAR Does

At runtime, V2RayDAR:

- loads `configs.yaml` or a custom `.yaml`, `.yml`, or `.json` config file,
- creates the app data folder when needed,
- fetches enabled subscription sources concurrently,
- stores previously-probed configs in a local SQLite database,
- parses raw text, base64, JSON, and YAML content from HTTP(S), single local-file, and `data:` subscription sources,
- **parses Clash/Mihomo YAML subscription configs** — detects `proxies:` lists and extracts vmess, vless, trojan, and ss proxy entries,
- extracts `vmess`, `vless`, `trojan`, `ss`, `ssr`, `hysteria2`, `hy2`, and `tuic` share links,
- **converts between V2Ray share-link formats and Clash/Mihomo YAML proxy entries** (bidirectional),
- validates candidates with either active `sing-box` HTTP probing or diagnostic TCP probing,
- ranks reachable configs by priority, latency, speed-test result, protocol, name, and URI,
- optionally promotes configs that worked across repeated refreshes,
- **serves working configs as both V2Ray share-links (`/subscription`) and full Mihomo YAML configs (`/mihomo.yaml`)**,
- watches the config file and refreshes when relevant settings change,
- provides a TUI for editing settings, subscriptions, sharing, logs, and database state.

## Requirements

Required:

- A supported operating system: Windows, Linux, macOS, or Termux on Android.
- A terminal.
- A V2Ray-compatible client (such as v2rayN, v2rayNG, sing-box) or a Clash/Mihomo-compatible client (such as Clash Verge, Mihomo, Clash Meta for Android) that can consume subscription URLs.

Required for active validation:

- A working `sing-box` executable. Desktop `_with_singbox` release archives include pinned `sing-box` 1.13.13 beside V2RayDAR. Termux users should install `sing-box=1.13.13` with `pkg`.
- If you are not using a bundled desktop archive or the standard Termux package path, set `probe.sing_box_path` to the executable path or a PATH command.

Optional for building from source:

- Rust toolchain with Cargo.

## Release Artifacts

Release builds are expected to be published as:

- Windows: `v2raydar-windows-x86_64.exe`
- Windows with bundled `sing-box` 1.13.13: `v2raydar-windows-x86_64_with_singbox.zip`
- Linux: `v2raydar-linux-x86_64`
- Linux with bundled `sing-box` 1.13.13: `v2raydar-linux-x86_64_with_singbox.tar.gz`
- macOS: `v2raydar-macos-universal.app.zip`
- macOS with bundled `sing-box` 1.13.13: `v2raydar-macos-universal_with_singbox.zip`
- Termux: `v2raydar-termux-aarch64.tar.gz` and `v2raydar-termux-x86_64.tar.gz`
- Checksum file: `checksums.txt`

Verify downloaded binaries with SHA-256 before running them.

Windows:

```powershell
Get-FileHash .\v2raydar-windows-x86_64.exe -Algorithm SHA256
```

Linux:

```bash
sha256sum ./v2raydar-linux-x86_64
```

macOS:

```bash
shasum -a 256 ./v2raydar-macos-universal.app.zip
```

Checksums verify integrity. They do not prevent Windows SmartScreen or macOS Gatekeeper prompts.

## First Run

On first launch without `--config`, V2RayDAR creates `configs.yaml` with a set of pre-selected subscription sources to get you started. Adding your own sources is recommended for better coverage. The portable installer runs with `--portable`, so config and database stay beside the executable; user-installed mode uses the platform app-data location.

Windows:

```text
%LOCALAPPDATA%\V2RayDAR\v2raydar_data\configs.yaml
```

macOS:

```text
~/Library/Application Support/V2RayDAR/v2raydar_data/configs.yaml
```

Linux:

```text
$XDG_DATA_HOME/V2RayDAR/v2raydar_data/configs.yaml
```

Fallback Linux path:

```text
~/.local/share/V2RayDAR/v2raydar_data/configs.yaml
```

Portable mode path:

```text
v2raydar_data/configs.yaml
```

If `probe.mode` is `active`, V2RayDAR first looks for a bundled `sing-box` beside the executable, then for the standard Termux package path on Android, then for `probe.sing_box_path`. If none is valid, the interactive TUI asks for the OS-specific `sing-box` executable path and verifies it with `sing-box version`.

In `--no-tui` or `--once` mode, V2RayDAR cannot run the interactive setup prompt. It prints OS-specific setup instructions and exits until a bundled, Termux-package, or configured `sing-box` executable is available.

## Run Modes

Run the interactive TUI and local HTTP endpoint:

```bash
v2raydar
```

Run headless with plain terminal progress and the local HTTP endpoint:

```bash
v2raydar --no-tui
```

Run one refresh, print a terminal summary, and exit without starting the endpoint:

```bash
v2raydar --once
```

Use a custom config file:

```bash
v2raydar --config path/to/configs.yaml
```

Keep the data folder beside the executable:

```bash
v2raydar --portable
```

Print detailed fetch/probe logs in plain terminal modes:

```bash
v2raydar --no-tui --verbose
v2raydar --once --verbose
```

Remove V2RayDAR-owned generated files and firewall rules:

```bash
v2raydar --uninstall
v2raydar --portable --uninstall
v2raydar --uninstall --yes
```

Ping config URIs and print latency results:

```bash
v2raydar --ping "vless://uuid@server:443?security=tls#name"
v2raydar --ping-file configs.txt
```

Windows users can replace `v2raydar` with `v2raydar.exe`.

## Source Build Commands

Development run:

```bash
cargo run
```

Development run with a local config:

```bash
cargo run -- --config configs.example.yaml
```

Headless development run:

```bash
cargo run -- --no-tui
```

One-shot development run:

```bash
cargo run -- --once
```

Release build:

```bash
cargo build --release
```

Windows release binary after a local build:

```powershell
target\release\v2raydar.exe
```

Linux/macOS release binary after a local build:

```bash
./target/release/v2raydar
```

## Local HTTP Endpoints

With the default bind address, V2RayDAR serves these URLs:

| Endpoint | Response |
| --- | --- |
| `http://127.0.0.1:27141/subscription` | Top working configs. Base64 when `encoded_subscription: true`. |
| `http://127.0.0.1:27141/subscription.txt` | Top working configs as newline-separated share links. |
| `http://127.0.0.1:27141/mihomo.yaml` | Full Mihomo YAML config with proxies, proxy-groups, and rules (raw). |
| `http://127.0.0.1:27141/results` | JSON runtime state, diagnostics, errors, logs, and ranked configs. |
| `http://127.0.0.1:27141/health` | `ok` health response. |

`/subscription` and `/subscription.txt` wait up to 20 seconds during an active refresh so clients have a chance to receive early working results instead of an empty feed. `/mihomo.yaml` behaves the same way.

Local loopback requests are always allowed. LAN requests to `/subscription`, `/subscription.txt`, `/mihomo.yaml`, and `/results` are blocked unless `sharing.enabled` is true; `/health` is only a reachability check.

## Client Setup

### v2rayN / v2rayNG (share-link format)

For a client on the same machine, keep the default bind address:

```yaml
bind: 127.0.0.1:27141
```

Then add this subscription URL in the client:

```text
http://127.0.0.1:27141/subscription
```

### sing-box (plain share-link format)

For sing-box clients, use the plain-text endpoint:

```text
http://127.0.0.1:27141/subscription.txt
```

### Mihomo / Clash.Meta (YAML config format)

For Mihomo, Clash Verge, or any Clash-compatible client, use the Mihomo endpoint:

```text
http://127.0.0.1:27141/mihomo.yaml
```

Import this URL directly in your Clash client's profile/subscription settings. V2RayDAR generates a complete Mihomo config with proxy entries, a `url-test` proxy group, and a catch-all `MATCH` rule.

For a phone or another device on the same Wi-Fi, enable LAN sharing and use the PC's LAN IP:

```yaml
bind: 127.0.0.1:27141
sharing:
  enabled: true
  require_token: false
  token: null
```

V2RayDAR can keep the local listener on `127.0.0.1` and also open a LAN listener on the primary LAN IP when sharing is enabled. You can also bind directly to a specific LAN IP:

```yaml
bind: 192.168.1.23:27141
sharing:
  enabled: true
```

Check reachability from the phone or another machine:

```text
http://192.168.1.23:27141/health
```

If it returns `ok`, use:

```text
http://192.168.1.23:27141/subscription
```

## Config File Formats

V2RayDAR accepts:

- `.yaml`
- `.yml`
- `.json`
- files without an extension, parsed as YAML

Other config extensions are rejected.

The generated default file is based on [configs.example.yaml](../configs.example.yaml).

When you use `--config path/to/configs.yaml`, V2RayDAR uses that file as the config and stores cache/state in a sibling `v2raydar_data` folder. If the custom config already lives inside a `v2raydar_data` folder, that folder is reused for cache/state.

Example:

```text
custom/configs.yaml
custom/v2raydar_data/cache/
```

## Config Validation Rules

The loader validates values before the app starts or before a live config reload is accepted:

- `top_n` must be greater than `0`.
- `fetch_concurrency` must be greater than `0`.
- `max_subscription_bytes` must be greater than `0`.
- `probe.concurrency` must be greater than `0`.
- `probe.batch_size` must be `null` or greater than `0`.
- `probe.process_concurrency` must be `null` or greater than `0`.
- `probe.connect_timeout_ms`, `probe.active_timeout_ms`, and `probe.startup_timeout_ms` must be greater than `0`.
- `probe.test_url` cannot be empty in active mode.
- `probe.accepted_statuses` cannot be empty in active mode.
- `probe.accepted_statuses` must contain valid HTTP status codes from `100` through `599`.
- `probe.download_bytes_limit` must be greater than `0`.
- Every subscription must have a non-empty `name` and `url`.
- `sharing.require_token: true` requires `sharing.token` to be a string or `true`.

String-like null values such as `null`, `"null"`, empty strings, `"none"`, and `"off"` are normalized for optional fields where supported.

## Config Reference

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `bind` | Socket address | `127.0.0.1:27141` | Primary HTTP bind address. |
| `top_n` | Integer | `10` | Number of reachable configs published to clients. |
| `refresh_seconds` | Integer seconds | `300` | Automatic refresh interval. `0` disables timer refreshes but config changes can still trigger refreshes. |
| `encoded_subscription` | Boolean | `true` | Makes `/subscription` return base64 text. `/subscription.txt` is always raw text. |
| `prioritize_stability` | Boolean | `true` | Re-pings the previous run's saved top-N first and keeps them ahead of newly discovered low-ping configs. When `false`, the ranking simply prefers any working low-ping config. The saved top-N is held in the cache folder and wiped on every fresh run and on quit. |
| `return_configs_asap` | Boolean | `false` | When `true`, publishes each working config to `/subscription`, `/subscription.txt`, `/results`, and the TUI `Current Found Configs` box as soon as it is found, until `top_n` working configs are available. Early configs may not have the lowest ping or best stability. |
| `scan_all_configs` | Boolean | `false` | When `false`, active probing can stop early after enough working configs are found. |
| `fetch_timeout_ms` | Integer milliseconds | `30000` | Per-source HTTP fetch timeout. |
| `fetch_concurrency` | Integer | `8` | Number of subscription sources fetched in parallel. |
| `max_subscription_bytes` | Integer bytes | `33554432` | Maximum accepted body size per subscription source. |
| `use_cache_only` | Boolean | `false` | Skips fresh subscription fetches and loads previously-probed configs from the database. |
| `clean_offlines_after_days` | Integer | `7` | Days after which unreachable configs are removed from the database. |
| `emergency_config` | String or null | `null` | Optional working share link used as a bridge proxy when HTTP subscription fetches fail. |
| `sharing` | Object | See below | LAN sharing and URL token settings. |
| `probe` | Object | See below | Validation mode, timeouts, concurrency, and active-test settings. |
| `geoip_db_path` | String or null | `null` | Optional path to a `GeoLite2-Country.mmdb` file. If `null`, uses the embedded database for country detection. |
| `subscriptions` | Array | Pre-selected sources | Sources to fetch and scan. Add your own for better results. |

## Sharing Settings

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `sharing.enabled` | Boolean | `false` | Allows non-loopback LAN clients to access the endpoints. |
| `sharing.require_token` | Boolean | `false` | Requires `?token=...` for LAN endpoint requests. |
| `sharing.token` | String, boolean, or null | `null` | `null`/empty disables token text, `true` generates a token, and a string uses that exact token. |

If `sharing.token: true` is configured, V2RayDAR generates a URL-safe token and saves it back into the config file.

Token checks apply only to LAN requests. Local requests from `127.0.0.1` are allowed even when token protection is enabled.

#### Proxy settings

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `proxy.enabled` | Boolean | `false` | Starts a persistent `sing-box` process with the best-ranked config, exposing a mixed SOCKS5/HTTP proxy on `proxy.port`. |
| `proxy.port` | Integer | `27910` | Port for the mixed SOCKS5/HTTP proxy. Must not equal `bind` port. |
| `proxy.discoverable` | Boolean | `false` | Binds to `0.0.0.0` instead of `127.0.0.1` and adds a firewall rule for LAN access. |
| `proxy.health_check_url` | String | `https://www.gstatic.com/generate_204` | URL tested through the proxy to verify it's alive. |
| `proxy.health_check_interval_seconds` | Integer | `60` | Seconds between proxy health checks. On failure, auto-failovers to the next ranked config. |

When `proxy.discoverable: true`, other devices on the same LAN can use the proxy. Replace `YOUR_LAN_IP` with the actual LAN IP shown in the TUI's **Current Configuration** panel under **Network**, and open the URL on your phone:

```
https://t.me/socks?server=YOUR_LAN_IP&port=27910
```

For example, if the TUI shows `192.168.1.2`:
```
https://t.me/socks?server=192.168.1.2&port=27910
```

Proxy example:

```yaml
proxy:
  enabled: true
  port: 27910
  discoverable: true
  health_check_url: https://www.gstatic.com/generate_204
  health_check_interval_seconds: 60
```

Token-protected LAN example:

```yaml
sharing:
  enabled: true
  require_token: true
  token: true
```

After startup, use the generated URL shown by the app, or add the token manually:

```text
http://192.168.1.23:27141/subscription?token=GENERATED_TOKEN
```

## Probe Settings

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `probe.mode` | `active` or `tcp` | `active` | Validation strategy. |
| `probe.sing_box_path` | String or null | `null` | Optional path to `sing-box`. Leave `null` for desktop `_with_singbox` builds or Termux's package path. |
| `probe.connect_timeout_ms` | Integer milliseconds | `5000` | TCP connect timeout in `tcp` mode. |
| `probe.active_timeout_ms` | Integer milliseconds | `30000` | HTTP request timeout in active mode. |
| `probe.startup_timeout_ms` | Integer milliseconds | `5000` | Time to wait for temporary `sing-box` proxies to start. |
| `probe.concurrency` | Integer | `16` | Base probe concurrency. |
| `probe.batch_size` | Integer or null | `20` | Initial active-probe batch size. The batch sizer can grow or shrink during a run. |
| `probe.process_concurrency` | Integer or null | `null` | Number of `sing-box` batch processes allowed at once. Auto mode is capped internally. |
| `probe.test_url` | URL string | `https://www.gstatic.com/generate_204` | URL requested through each candidate proxy in active mode. |
| `probe.accepted_statuses` | HTTP status array | `[204, 200]` | HTTP statuses treated as successful active validation. |
| `probe.download_url` | URL string or null | `null` | Optional URL used for speed testing top working configs. |
| `probe.download_bytes_limit` | Integer bytes | `1048576` | Maximum bytes read from `probe.download_url` per speed test. |

## Active Mode

`probe.mode: active` is the normal mode. It uses `sing-box` to create temporary local proxy listeners, routes test HTTP requests through the candidate configs, and marks a config reachable only when the configured `probe.test_url` returns one of `probe.accepted_statuses`.

Active mode can also use an optional `probe.download_url` to measure throughput for selected working configs. The result appears in `/results` as `download_mbps` and `download_bytes`.

Active mode requires a working `sing-box` executable. If `sing-box` is unavailable, candidates are marked failed with an error explaining that `sing-box` could not be run.

## TCP Mode

`probe.mode: tcp` is diagnostic. It only checks whether the candidate endpoint host and port accepts a TCP connection. It does not prove that the V2Ray-compatible config works, authenticates, or can carry traffic.

TCP mode is useful for quick endpoint diagnostics, but active mode is required for reliable shortcut publishing.

Example:

```yaml
probe:
  mode: tcp
  connect_timeout_ms: 5000
```

## Subscription Sources

Each subscription item has:

| Key | Type | Meaning |
| --- | --- | --- |
| `name` | String | Display name and source label in results. |
| `url` | String | HTTP URL, HTTPS URL, single local file path, `file://` file URL, or `data:` URL. |
| `enabled` | Boolean | Whether the source is fetched. Defaults to `true` if omitted. |
| `priority` | Integer | Lower numbers are ranked ahead of higher numbers when other checks are equal. Defaults to `100` if omitted. |

Example:

```yaml
subscriptions:
  - name: primary
    url: https://example.com/subscription.txt
    enabled: true
    priority: 1
  - name: local-file
    url: file:///home/user/subscriptions/private.txt
    enabled: true
    priority: 10
```

Supported source URL forms:

- `https://example.com/subscription`
- `http://example.com/subscription`
- `file:///home/user/sub.txt`
- `/home/user/sub.txt`
- `C:\Users\name\sub.txt`
- `data:,vless://uuid@example.com:443%23demo`
- `data:;base64,dmxlc3M6Ly8uLi4=`

Local file paths must point to one readable file. Directories are not scanned.

## Subscription Content Parsing

V2RayDAR extracts share links from:

- plain newline-separated text,
- base64-encoded newline-separated text,
- JSON strings at any depth,
- YAML strings at any depth,
- **Clash/Mihomo YAML configs** — detects `proxies:` lists with `type`/`server`/`port` entries and extracts proxy entries as share links.

Parsed share-link schemes:

- `vmess://`
- `vless://`
- `trojan://`
- `ss://`
- `ssr://`
- `hysteria2://`
- `hy2://`
- `tuic://`

Duplicate URIs are removed while preserving source order.

When a subscription source contains a Clash/Mihomo config (YAML with a `proxies:` list), V2RayDAR automatically detects it and converts each proxy entry to the corresponding share-link URI. Supported proxy types for extraction are `vmess`, `vless`, `trojan`, and `ss`. Unsupported types (e.g., `wireguard`, `hysteria`) are silently skipped.

## Active Validation Link Support

Active `sing-box` validation currently builds outbound configs for:

- VMess,
- VLESS,
- Trojan,
- Shadowsocks,
- Hysteria2 / HY2,
- TUIC.

SSR links are parsed for discovery and TCP diagnostics, but active `sing-box` probing does not currently convert SSR share links into `sing-box` outbounds.

Supported active transports include:

- TCP or omitted transport,
- WebSocket (`ws` / `websocket`),
- gRPC,
- HTTP/2 (`h2` / `http`),
- HTTP upgrade (`httpupgrade`).

Unsupported transports are skipped per candidate and reported in results instead of failing the whole scan.

## Clash/Mihomo Support

V2RayDAR provides full Clash/Mihomo integration on both input and output sides.

### Input: Parsing Clash/Mihomo Subscription Sources

You can add Clash/Mihomo subscription URLs directly as sources in `configs.yaml`. V2RayDAR automatically detects configs that contain a `proxies:` list with `type`/`server`/`port` entries and extracts proxy entries as share links.

```yaml
subscriptions:
  - name: mihomo-source
    url: https://example.com/mihomo.yaml
    enabled: true
    priority: 1
  - name: v2ray-source
    url: https://example.com/subscription.txt
    enabled: true
    priority: 2
```

Supported proxy types for extraction: `vmess`, `vless`, `trojan`, `ss`. Unsupported types (e.g., `wireguard`, `hysteria`) are silently skipped. The parser handles nested structures like `ws-opts`, `grpc-opts`, `h2-opts`, `reality-opts`, and `tls` settings.

### Output: Serving Working Configs as Mihomo YAML

The `/mihomo.yaml` endpoint generates a complete Mihomo-compatible YAML config on the fly from the current top-N working configs:

```text
http://127.0.0.1:27141/mihomo.yaml
```

The generated config includes:

- A `proxies:` section with each working config converted to its Clash YAML format,
- A `proxy-groups:` section with a `url-test` group pointing at `https://www.gstatic.com/generate_204`,
- A `rules:` section with a catch-all `MATCH` rule routing to the `auto` group.

Import this URL directly in any Clash-compatible client (Mihomo, Clash Verge, Clash Meta for Android, etc.).

### Bidirectional Conversion

The `convert` module handles all conversions between V2Ray share-link formats and Clash/Mihomo YAML proxy entries:

| Direction | Example |
| --- | --- |
| `vless://` URI → Clash YAML entry | `vless://uuid@host:443?security=tls&type=ws` → `type: vless` with `tls: true`, `network: ws`, `ws-opts:` |
| `vmess://` URI → Clash YAML entry | `vmess://<base64>` → `type: vmess` with `uuid`, `alterId`, `cipher`, transport settings |
| Clash YAML entry → `vless://` URI | `type: vless` → `vless://uuid@host:port?params#name` |
| Clash YAML entry → `vmess://` URI | `type: vmess` → `vmess://<base64 json>` |

Supported transport mappings: TCP, WebSocket (`ws`), gRPC, HTTP/2 (`h2`), HTTPUpgrade. Supported TLS features: standard TLS, Reality (with `public-key` and `short-id`), uTLS fingerprinting.

## Database Behavior

Previously-probed configs are stored in a local SQLite database:

```text
v2raydar_data/data.db
```

The database contains:

- `configs` table — all known configs with dedup_key, URI, source, protocol, endpoint, reachability status, latency, stability count, and `last_online` timestamp,
- `stable_top` table — the previous run's saved top-N dedup_keys used by `prioritize_stability`.

When a refresh completes, all probed configs are upserted into the database. Configs that are reachable get their `last_online` timestamp updated. Configs that are unreachable keep their existing `last_online` value. After each refresh, configs not seen online for `clean_offlines_after_days` (default: 7) are removed.

## Restricted-Network Behavior

On very restricted networks, set `use_cache_only: true` to load previously-probed configs from the database instead of fetching fresh subscriptions. The app can test these previously-probed configs when fresh HTTP subscription URLs are unreachable.

Refresh behavior is:

1. Fetch enabled subscriptions directly.
2. Parse and probe the configs that were fetched.
3. If some HTTP subscription sources failed and active probing has at least one bridge config, retry failed HTTP sources through that bridge.
4. If no fresh subscription source was fetched successfully, fall back to cached HTTP snapshots.
5. Probe fallback candidates and publish any reachable results.

The bridge config is selected in this order:

1. `emergency_config`, when set.
2. A reachable config from the current refresh.
3. A reachable config from the previous refresh.

This means that if some HTTP subscription URLs do not connect on your network but one config is reachable, V2RayDAR can use that reachable config through `sing-box` to retry failed HTTP subscription fetches. If none of your configured subscriptions are reachable but you already have one working config, put it in `emergency_config`.

Example:

```yaml
emergency_config: vless://uuid@example.com:443?security=tls&sni=example.com#bridge
```

To intentionally load previously-probed configs from the database:

```yaml
use_cache_only: true
```

## Ranking

The final ranked list always puts reachable configs before failed configs. When `prioritize_stability: true`, reachable configs that were in the previous run's saved top-N are promoted before the remaining tie-breakers, so a higher-ping config that already proved working last refresh stays ahead of a newly discovered low-ping config. When `prioritize_stability: false`, the ranking simply prefers any working low-ping config without any carry-over.

The saved top-N is written to the `stable_top` table in the database at the end of each refresh, re-pinged at the start of the next refresh, and deleted on app startup and shutdown so each fresh run begins with no stability carry-over.

The remaining tie-breakers are:

1. Lower `priority` values first.
2. Lower `latency_ms` first.
3. Higher `download_mbps` first, when speed testing is enabled.
4. Protocol.
5. Name.
6. URI.

When `return_configs_asap: true`, the subscription endpoints and the TUI `Current Found Configs` box are populated one working config at a time during probing until `top_n` working configs have been returned. These live discoveries do not add entries to the TUI `Recent Logs` panel; the normal refresh summary is logged after the refresh completes.

When `scan_all_configs: false`, active mode can stop early after it finds enough working configs for `top_n`. With stability prioritization enabled, the scheduler also re-pings the previous run's saved top-N first, so they are not skipped before they get a chance to be confirmed.

When `scan_all_configs: true`, V2RayDAR attempts to validate every loaded candidate.

## Live Config Reloading

While the app is running, it watches the config file once per second. If the file changes and the changed settings affect fetching, probing, ranking, or subscriptions, V2RayDAR refreshes automatically.

The HTTP bind address is special. If `bind` changes while the app is running, the config is reloaded but the existing listener continues using the original bind address. Restart V2RayDAR to apply a changed `bind`.

If a live reload fails validation, V2RayDAR keeps the previous valid config and logs the error.

## Refresh Timing

The app runs one refresh immediately after startup.

After that:

- `refresh_seconds: 300` refreshes every five minutes.
- `refresh_seconds: 0` disables timer refreshes.
- Relevant config-file changes can still trigger refreshes even when `refresh_seconds` is `0`.

Headless mode prints compact progress by default and a detailed trace with `--verbose`.

## TUI Overview

The default mode starts a terminal UI with:

- a top status area,
- local and LAN subscription URL information,
- sharing status,
- subscription-source management,
- config-value editing,
- cache cleaning,
- live ranked configs,
- recent logs.

Main menu items:

- `Open Configs File`
- `Share subscription URL on LAN`
- `Subscriptions`
- `Clean Cache`
- `Configurations`
- `Live Logs`

The UI is mouse-aware. Clicking rows selects them.

## TUI Keyboard Controls

Global controls:

| Key | Action |
| --- | --- |
| `q` | Quit. |
| `Ctrl+C` | Quit. |
| `Esc` | Go back or cancel input. |
| `Ctrl+H`, `Ctrl+Backspace`, `Ctrl+Delete` | Go back. |
| `j` / Down | Move selection down. |
| `k` / Up | Move selection up. |
| Enter | Activate selected row. |
| `s` | Save editable config state. |
| Space | Toggle the selected subscription where applicable. |
| `e` | Open actions for the selected subscription. |
| `:` | Enter command mode. |

Command mode accepts:

| Command | Action |
| --- | --- |
| `:q`, `:quit` | Quit. |
| `:a`, `:add` | Add a subscription. |
| `:n`, `:name` | Edit selected subscription name. |
| `:u`, `:url` | Edit selected subscription URL. |
| `:p`, `:priority` | Edit selected subscription priority. |
| `:t`, `:toggle` | Enable or disable selected subscription. |
| `:d`, `:delete` | Delete selected subscription. |
| `:w`, `:save` | Save config changes. |

Adding a subscription is a four-step flow:

1. URL.
2. Display name.
3. Priority number.
4. Enabled state.

Boolean prompts accept values such as `yes`, `no`, `true`, `false`, `on`, `off`, `1`, and `0`.

## Config Editing In The TUI

The `Configurations` panel exposes the same settings as `configs.yaml`, including:

- bind address,
- top-N count,
- refresh interval,
- encoded feed toggle,
- stability ranking,
- ASAP result publishing,
- full-scan toggle,
- fetch limits,
- cache-only mode,
- emergency config,
- probe mode and timeouts,
- `sing-box` path,
- active test URL and accepted statuses,
- optional download test,
- sharing token settings,
- reset-to-defaults action.

The reset action keeps the current subscriptions but restores non-subscription settings to defaults. It asks for a short confirmation code before applying.

TUI saves try to preserve the shape and comments of the existing YAML file where possible.

## LAN Sharing And Firewall Handling

LAN sharing is disabled by default.

When sharing is enabled:

- Local requests from the same machine continue to work.
- LAN requests are allowed only when `sharing.enabled` is true.
- LAN token checks are enforced only when `sharing.require_token` is true.
- The app can display a discoverable LAN URL based on the active bind address and detected LAN IP.

The TUI's sharing toggle saves the config and then tries to apply firewall changes.

Windows:

- Uses `netsh advfirewall firewall`.
- Adds or removes a rule named `V2RayDAR Subscription Sharing`.
- May require an elevated terminal.

Linux:

- Uses `ufw` when available.
- Uses `firewall-cmd` when `ufw` is unavailable and firewalld is available.
- Records only V2RayDAR-created rules as owned.
- Leaves pre-existing user-owned rules alone.

macOS and unsupported systems:

- Firewall auto-change is not currently supported.
- You must allow the port manually if needed.

Owned firewall state is stored in:

```text
v2raydar_data/.v2raydar-firewall.json
```

## Runtime Artifacts

Installed mode creates app-owned data under:

```text
V2RayDAR/v2raydar_data/
```

Typical files and folders:

| Artifact | Meaning |
| --- | --- |
| `configs.yaml` | Main config file generated on first run. |
| `data.db` | SQLite database storing previously-probed configs and stable top-N keys. |
| `.v2raydar-firewall.json` | Records firewall rules created by V2RayDAR. |

Legacy marker names are still recognized during cleanup:

- `.v2raydar`
- `.v2raydar-cache`

Build artifacts are generated under Cargo's normal target directory:

```text
target/
```

Release workflow artifacts are staged under:

```text
dist/
```

## Uninstall Behavior

Run:

```bash
v2raydar --uninstall
```

Portable mode:

```bash
v2raydar --portable --uninstall
```

Unattended cleanup:

```bash
v2raydar --uninstall --yes
```

Without `--yes`, the command asks you to type:

```text
DELETE
```

The uninstall command removes V2RayDAR-owned app data and V2RayDAR-owned firewall rules. It does not delete:

- the V2RayDAR executable itself,
- downloaded `sing-box` binaries,
- custom config files passed through `--config` when they live outside `v2raydar_data`,
- unrelated files found beside app data.

Cleanup is conservative:

- If the app directory contains only known V2RayDAR artifacts, the whole app directory can be removed.
- If unknown files are present, only known V2RayDAR files are targeted.
- If a cache directory contains unknown files, only known cache snapshot files and metadata are targeted.
- Custom config cleanup removes the sibling `v2raydar_data` folder but not the custom config file itself.

## Security Notes

Prefer `127.0.0.1:27141` for same-machine use. It is private to the local machine.

Prefer a specific LAN IP such as `192.168.1.23:27141` when sharing to a phone or another device. Avoid `0.0.0.0:27141` unless you intentionally want to listen on every interface.

Do not expose V2RayDAR's HTTP endpoint to the public internet.

Use `sharing.require_token: true` on shared or less trusted LANs. The token is a URL token, not a full authentication system.

Treat subscription URLs and share links as sensitive. Anyone with access to the local subscription endpoint can read the working configs V2RayDAR publishes.

The `emergency_config` is also sensitive because it contains a working proxy config. Do not share logs or config files that include private links.

## Performance Notes

The most important performance settings are:

- `fetch_concurrency`
- `fetch_timeout_ms`
- `max_subscription_bytes`
- `probe.concurrency`
- `probe.batch_size`
- `probe.process_concurrency`
- `return_configs_asap`
- `scan_all_configs`
- `top_n`

For faster results on huge subscriptions, keep:

```yaml
scan_all_configs: false
top_n: 10
```

For exhaustive testing, use:

```yaml
scan_all_configs: true
```

For lower system load, reduce:

```yaml
fetch_concurrency: 4
probe:
  concurrency: 8
  process_concurrency: 1
```

The active probe process concurrency is internally capped to avoid local process and socket congestion.

## Resource Footprint

Measured on a Windows x86_64 machine with 21 subscription sources, 8758 candidates, `--once` mode:

| Metric | Value | Notes |
| --- | --- | --- |
| Release binary size | 16 MB | Stripped + LTO + panic=abort |
| Embedded GeoIP database | 8.5 MB | Inside the binary |
| Startup RSS | ~33 MB | Before any network I/O |
| Peak RSS (fetch phase) | ~55 MB | Parsing candidates from all sources |
| Peak RSS (probe phase) | ~73 MB | Parallel sing-box processes |
| RSS after probe completes | ~42 MB | sing-box processes exited |
| Total CPU time | ~0.8 s | Across all cores |
| Wall clock (fetch + probe) | ~40 s | 8 s fetch + 30 s probe + 2 s retry |
| Database size after run | ~628 KB | 319 ranked configs stored |
| Network per refresh cycle | ~4-5 MB | Subscription body downloads |

**Low-end device guidance:**

| RAM | Recommendation |
| --- | --- |
| 256 MB+ | Default settings work comfortably |
| 128 MB | Lower `probe.process_concurrency: 1-2` to reduce parallel sing-box processes |
| 64 MB | Use `top_n: 5`, `fetch_concurrency: 4`, `probe.concurrency: 8`, `probe.process_concurrency: 1` |

## Developer Architecture

Important modules:

| File | Responsibility |
| --- | --- |
| `src/main.rs` | CLI parsing, startup, refresh loop, config watcher, uninstall, ranking integration. |
| `src/config.rs` | Config schema, defaults, validation, token generation, config loading. |
| `src/constants.rs` | Default values, UI lists, artifact names, supported URI schemes. |
| `src/paths.rs` | Installed, portable, and custom-config path resolution. |
| `src/subscription.rs` | Fetching subscription sources, cache snapshots, cache fallback, proxied retry. |
| `src/parser.rs` | Share-link extraction and endpoint parsing. |
| `src/clash.rs` | Clash/Mihomo YAML subscription parsing. |
| `src/convert.rs` | Bidirectional V2Ray share-link ↔ Clash/Mihomo YAML conversion, shared field extraction, full Clash config generation. |
| `src/probe.rs` | TCP probing, active `sing-box` probing, outbound conversion, speed testing. |
| `src/sing_box.rs` | `sing-box` availability/setup helpers and temporary proxy execution. |
| `src/server.rs` | Axum HTTP server, endpoint responses (`/subscription`, `/mihomo.yaml`, `/results`, `/health`), LAN authorization. |
| `src/network.rs` | LAN IP discovery and sharing status. |
| `src/terminal.rs` | Plain terminal startup, progress, and summary output. |
| `src/model.rs` | Runtime state, ranked config, candidate, and serialized response models. |
| `src/tui.rs` and `src/tui/*` | Ratatui UI, input handling, config editing, panels, firewall integration. |
| `build.rs` | Windows resource/icon embedding. |

## Refresh Pipeline

The refresh pipeline in `src/main.rs` is roughly:

1. Load runtime config into shared state.
2. Fetch enabled subscriptions directly unless `use_cache_only` is true.
3. Parse fetched subscription bodies into candidates — **including Clash/Mihomo YAML configs** which are detected by their `proxies:` structure and converted to share links.
4. Probe candidates with `probe_candidates`. When `prioritize_stability: true`, the scheduler re-pings the previous run's saved top-N first.
5. If direct fetches failed, retry failed HTTP sources through `emergency_config` or a working config when active mode is available, then probe newly loaded retry candidates.
6. If no fresh subscription source was fetched successfully, load previously-probed configs from the database and probe them.
7. Apply stability ranking (carry the previous run's saved top-N to the front when enabled, otherwise rank purely by low ping).
8. Publish ranked state to `/subscription`, `/subscription.txt`, and `/results`.
9. Persist all probed configs to the database and save the new top-N stable keys (when stability ranking is on) so the next refresh re-pings it first.
10. Clean up configs not seen online for `clean_offlines_after_days` days.
11. Record refresh duration, errors, byte counters, logs, and consecutive-top-N counters.

The refresh loop starts immediately on app launch. Later refreshes are driven by the timer or relevant config-file changes.

## HTTP Server Behavior

The HTTP server uses Axum and binds the configured `bind` address.

When `sharing.enabled` is true and the configured bind is loopback, the server also attempts to start a LAN listener on the primary detected LAN IP using the same port.

Authorization rules for `/subscription`, `/subscription.txt`, `/mihomo.yaml`, and `/results`:

- Loopback requests are accepted.
- LAN requests are rejected with `403` when sharing is disabled.
- LAN requests are rejected with `401` when token protection is enabled and the token is missing or incorrect.

Subscription response format:

- `/subscription` uses `encoded_subscription`.
- `/subscription.txt` is always raw.
- `/mihomo.yaml` is always raw YAML.
- Both share-link and Mihomo endpoints include only reachable configs and only the top `top_n` entries.
- With `return_configs_asap: true`, they can fill with working configs during an in-progress refresh before final ranking and speed-test enrichment complete.
- The body ends with a trailing newline when at least one config is present.

The `/mihomo.yaml` endpoint generates a complete Mihomo-compatible YAML config on the fly from the current top-N working configs. Each config is converted to its Clash proxy entry format, placed in a `proxies:` list, wrapped in a `url-test` proxy group, and completed with a catch-all `MATCH` rule.

## Database Implementation

Previously-probed configs are stored in a SQLite database (`data.db`). The database uses WAL journal mode for concurrent read/write access.

The `configs` table stores each config with a unique `dedup_key` (protocol|host|port|transport|tls). On each refresh, configs are upserted — reachable configs get their `last_online` timestamp updated, while unreachable configs keep their existing `last_online` value.

The `stable_top` table stores a single row with the previous run's top-N dedup_keys for stability ranking.

After each refresh, configs not seen online for `clean_offlines_after_days` days are removed from the database.

## Active Probe Implementation

Active probing converts supported share links into temporary `sing-box` outbound definitions. It batches candidates into temporary `sing-box` config files, starts local mixed proxy listeners, sends HTTP requests through those listeners, and records latency/status results.

The active batcher:

- deduplicates equivalent outbound definitions,
- schedules sources fairly by source priority,
- re-pings the previous run's saved top-N first when stability ranking is enabled,
- grows or shrinks batch size based on batch success,
- splits failed batches to isolate invalid candidates,
- caps HTTP and process concurrency internally.

Temporary `sing-box` configs use names beginning with:

```text
v2raydar-sing-box
```

Temporary inbound and outbound tags use:

```text
mixed-in-*
proxy-*
```

Temporary files are intended to be cleaned up after each probe batch.

## TUI Implementation Notes

The TUI uses:

- `ratatui` for rendering,
- `crossterm` for terminal events,
- a shared runtime state for refresh progress,
- an editable copy of `AppConfig`,
- YAML-preserving helpers in `src/tui/util.rs` for saving config changes.

The TUI stores recent logs in memory only. Runtime log buffers are capped by `MAX_TUI_LOGS`.

Opening the config file uses platform-aware editor detection. If no editor can be launched, the TUI displays the config path for manual editing.

## Testing And Checks

Run unit tests:

```bash
cargo test --locked
```

Run formatting check:

```bash
cargo fmt --check
```

Run Clippy:

```bash
cargo clippy --locked --all-targets --all-features -- -D warnings
```

Build a release binary:

```bash
cargo build --release --locked
```

Useful manual checks:

```bash
cargo run -- --once --config configs.example.yaml
cargo run -- --no-tui --config configs.example.yaml
cargo run -- --portable --once
```

If active mode cannot find `sing-box`, either configure `probe.sing_box_path` or temporarily use TCP mode for parser and fetch diagnostics:

```yaml
probe:
  mode: tcp
```

## Release Workflow Notes

The GitHub release workflow builds:

- Windows `x86_64-pc-windows-msvc`,
- Linux `x86_64-unknown-linux-gnu`,
- macOS universal `.app` from `x86_64-apple-darwin` and `aarch64-apple-darwin`.

The macOS release job creates a `V2RayDAR.app` bundle, embeds the PNG icon as an app icon, creates a universal binary with `lipo`, and zips the app with `ditto`.

The release job also creates `checksums.txt` from files in `dist/`.

## Troubleshooting

### `sing-box` setup is required

Active probing needs `sing-box`.

Set the executable for your OS:

```yaml
probe:
  mode: active
  sing_box_path: /full/path/to/sing-box
```

Linux example:

```yaml
probe:
  sing_box_path: /usr/local/bin/sing-box
```

Termux example:

```yaml
probe:
  sing_box_path: /data/data/com.termux/files/usr/bin/sing-box
```

macOS example:

```yaml
probe:
  sing_box_path: /opt/homebrew/bin/sing-box
```

Windows example:

```yaml
probe:
  sing_box_path: C:\Tools\sing-box\sing-box.exe
```

Use a working `sing-box` executable for active probing. Desktop `_with_singbox` releases include pinned `sing-box` 1.13.13 and auto-detect it from beside the V2RayDAR executable. Termux users should prefer `pkg install sing-box=1.13.13`, which installs `/data/data/com.termux/files/usr/bin/sing-box`. If you already use v2rayN on Windows, check the v2rayN installation folder for `sing-box.exe`.

### Port cannot bind

The default port is `27141`.

If binding fails, change:

```yaml
bind: 127.0.0.1:27142
```

On Windows, a port can be reserved even when no app appears to be using it. Check reserved ranges with:

```powershell
netsh interface ipv4 show excludedportrange protocol=tcp
```

### Phone cannot open `/health`

Check:

- The phone and PC are on the same LAN.
- `sharing.enabled` is true.
- The firewall allows TCP on the configured port.
- You are using the PC's LAN IP, not `127.0.0.1`.
- The bind address is either loopback with sharing enabled, a specific LAN IP, or `0.0.0.0`.

### LAN request returns `403`

LAN sharing is disabled. Set:

```yaml
sharing:
  enabled: true
```

### LAN request returns `401`

Token protection is enabled and the token is missing or wrong. Use:

```text
http://LAN_IP:27141/subscription?token=TOKEN
```

### Subscription endpoint is empty

Possible causes:

- No refresh has completed yet.
- No candidates were parsed from the configured sources.
- All candidates failed validation.
- `top_n` is too low only if you expected more than the published set.
- Active mode could not run `sing-box`.
- Fetches failed and no previously-probed configs exist in the database.

Check:

```text
http://127.0.0.1:27141/results
```

Look at `fetch_errors`, `ranked`, `last_error`, `tested_candidates`, and `reachable_candidates`.

### Subscription URLs fail on a restricted network

Previously-probed configs are stored in the database and can be used via `use_cache_only: true`.

Add a known working config:

```yaml
emergency_config: vless://uuid@example.com:443?security=tls&sni=example.com#bridge
```

Then keep active mode enabled so V2RayDAR can use that config through `sing-box` to retry failed HTTP subscription fetches.

### Config reload does not change the port

This is expected. Changing `bind` requires restarting V2RayDAR.

### Cache-only mode finds nothing

`use_cache_only` loads previously-probed configs from the database. Run at least one successful online refresh first so the database has configs to load.

Disable cache-only mode:

```yaml
use_cache_only: false
```

Run at least one successful online refresh first so the database has configs to load.

## Sample Minimal Config

```yaml
bind: 127.0.0.1:27141
top_n: 10
refresh_seconds: 300
encoded_subscription: true
prioritize_stability: true
return_configs_asap: false
scan_all_configs: false
fetch_timeout_ms: 30000
fetch_concurrency: 8
max_subscription_bytes: 33554432
use_cache_only: false
emergency_config: null
geoip_db_path: null
clean_offlines_after_days: 7

sharing:
  enabled: false
  require_token: false
  token: null

probe:
  mode: active
  sing_box_path: null
  connect_timeout_ms: 5000
  active_timeout_ms: 30000
  startup_timeout_ms: 5000
  concurrency: 16
  batch_size: 20
  process_concurrency: null
  test_url: https://www.gstatic.com/generate_204
  accepted_statuses: [204, 200]
  download_url: null
  download_bytes_limit: 1048576

subscriptions:
  - name: primary
    url: https://example.com/subscription.txt
    enabled: true
    priority: 1
```

## Sample LAN Sharing Config

```yaml
bind: 127.0.0.1:27141
sharing:
  enabled: true
  require_token: true
  token: true
```

After the generated token is saved, the URL will look like:

```text
http://192.168.1.23:27141/subscription?token=...
```

## Sample Restricted-Network Config

```yaml
use_cache_only: false
emergency_config: vless://uuid@example.com:443?security=tls&sni=example.com#known-working

probe:
  mode: active
  sing_box_path: /usr/local/bin/sing-box

subscriptions:
  - name: source-a
    url: https://example.com/source-a.txt
    enabled: true
    priority: 1
  - name: source-b
    url: https://example.com/source-b.txt
    enabled: true
    priority: 2
```

If fresh fetching becomes impossible, temporarily switch to:

```yaml
use_cache_only: true
```

## Contributing

PRs are welcome.

Good pull requests should include:

- a clear description of the behavior change,
- focused code changes,
- tests when the change affects parsing, config validation, probing, ranking, paths, cache behavior, server authorization, or TUI saving,
- README updates when user-facing behavior changes.

Avoid adding unrelated refactors to feature or bug-fix PRs.

## Roadmap

- Add a cross-platform GUI app beside the TUI using Tauri.
- Extract V2Ray configs from the body of any website, preferably not JavaScript-heavy websites. JavaScript-heavy extraction can be handled through Obscura later.
- Add private endpoints with password requirements and authentication for private subscription endpoints, so users can fetch their private endpoints through a nationally reachable endpoint that has internet access.

## References

- Main README: [README.md](../README.md)
- Example config: [configs.example.yaml](../configs.example.yaml)
- Release guide: [release.md](release.md)
- License: [LICENSE](../LICENSE)
- sing-box releases: <https://github.com/SagerNet/sing-box/releases>
- sing-box configuration docs: <https://sing-box.sagernet.org/configuration/>
- v2rayN: <https://github.com/2dust/v2rayN>
- v2rayNG: <https://github.com/2dust/v2rayNG>
