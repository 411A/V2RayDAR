<p align="center">
  <img src="assets/V2RayDAR_logo_v1.png" alt="V2RayDAR logo" width="100" height="100">
</p>

# V2RayDAR Detailed Guide

V2RayDAR is a Rust CLI/TUI application that fetches V2Ray-compatible subscription sources, extracts supported share links, checks which configs work on your current network, ranks the working results, and publishes the best ones through a local subscription endpoint.

The name means **V2Ray Detection And Reconnaissance** and is pronounced like `v2ray` + `radar`.

This document is the detailed user and developer guide. The short, ready-to-use guide is in [README.md](README.md).

## Scope And Responsibility

V2RayDAR does not create, sell, host, or distribute V2Ray-compatible configs. It only scans subscription sources that you configure and republishes the working configs it finds on your own machine.

The app is published as-is, without any warranty. You are responsible for the subscription URLs and configs you scan, import, and connect to. A proxy or server operator may be able to intercept your traffic and read traffic that is not encrypted end-to-end.

## What V2RayDAR Does

At runtime, V2RayDAR:

- loads `configs.yaml` or a custom `.yaml`, `.yml`, or `.json` config file,
- creates the app data folder when needed,
- fetches enabled subscription sources concurrently,
- stores HTTP subscription snapshots in a local cache,
- parses raw text, base64, JSON, and YAML content from HTTP(S), single local-file, and `data:` subscription sources,
- extracts `vmess`, `vless`, `trojan`, `ss`, `ssr`, `hysteria2`, `hy2`, and `tuic` share links,
- validates candidates with either active `sing-box` HTTP probing or diagnostic TCP probing,
- ranks reachable configs by priority, latency, speed-test result, protocol, name, and URI,
- optionally promotes configs that worked across repeated refreshes,
- serves the top working configs at local HTTP endpoints,
- watches the config file and refreshes when relevant settings change,
- provides a TUI for editing settings, subscriptions, sharing, logs, and cache state.

## Requirements

Required:

- A supported operating system: Windows, Linux, or macOS.
- A terminal.
- A V2Ray-compatible client, such as v2rayN, v2rayNG, sing-box, or another client that can consume subscription URLs.

Required for active validation:

- `sing-box` installed locally.
- `probe.sing_box_path` set to the executable path or to a command that works from your terminal PATH. Use `sing-box.exe` on Windows and `sing-box` on Linux, Termux, and macOS.

Optional for building from source:

- Rust toolchain with Cargo.

## Release Artifacts

Release builds are expected to be published as:

- Windows: `v2raydar-windows-x86_64.exe`
- Linux: `v2raydar-linux-x86_64`
- macOS: `v2raydar-macos-universal.app.zip`
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

On first launch without `--config`, V2RayDAR creates `configs.yaml` in the platform app-data location.

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

If `probe.mode` is `active` and no valid `probe.sing_box_path` is configured, the interactive TUI asks for the OS-specific `sing-box` executable path and verifies it with `sing-box version`.

In `--no-tui` or `--once` mode, V2RayDAR cannot run the interactive setup prompt. It prints OS-specific setup instructions and exits until `probe.sing_box_path` is configured manually.

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
| `http://127.0.0.1:27141/results` | JSON runtime state, diagnostics, errors, logs, and ranked configs. |
| `http://127.0.0.1:27141/health` | `ok` health response. |

`/subscription` and `/subscription.txt` wait up to 20 seconds during an active refresh so clients have a chance to receive early working results instead of an empty feed.

Local loopback requests are always allowed. LAN requests to `/subscription`, `/subscription.txt`, and `/results` are blocked unless `sharing.enabled` is true; `/health` is only a reachability check.

## Client Setup

For a client on the same machine, keep the default bind address:

```yaml
bind: 127.0.0.1:27141
```

Then add this subscription URL in the client:

```text
http://127.0.0.1:27141/subscription
```

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

The generated default file is based on [configs.example.yaml](configs.example.yaml).

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
| `scan_all_configs` | Boolean | `false` | When `false`, active probing can stop early after enough working configs are found. |
| `fetch_timeout_ms` | Integer milliseconds | `30000` | Per-source HTTP fetch timeout. |
| `fetch_concurrency` | Integer | `8` | Number of subscription sources fetched in parallel. |
| `max_subscription_bytes` | Integer bytes | `33554432` | Maximum accepted body size per subscription source. |
| `use_cache_only` | Boolean | `false` | Skips fresh subscription fetches and tests only cached HTTP snapshots. |
| `emergency_config` | String or null | `null` | Optional working share link used as a bridge proxy when HTTP subscription fetches fail. |
| `sharing` | Object | See below | LAN sharing and URL token settings. |
| `probe` | Object | See below | Validation mode, timeouts, concurrency, and active-test settings. |
| `subscriptions` | Array | Two example entries | Sources to fetch and scan. |

## Sharing Settings

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `sharing.enabled` | Boolean | `false` | Allows non-loopback LAN clients to access the endpoints. |
| `sharing.require_token` | Boolean | `false` | Requires `?token=...` for LAN endpoint requests. |
| `sharing.token` | String, boolean, or null | `null` | `null`/empty disables token text, `true` generates a token, and a string uses that exact token. |

If `sharing.token: true` is configured, V2RayDAR generates a URL-safe token and saves it back into the config file.

Token checks apply only to LAN requests. Local requests from `127.0.0.1` are allowed even when token protection is enabled.

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
| `probe.sing_box_path` | String or null | `null` | Full path to the `sing-box` executable. |
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
- YAML strings at any depth.

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

## Cache Behavior

HTTP and HTTPS subscription responses are cached in:

```text
v2raydar_data/cache/
```

The cache contains:

- timestamped `.txt` snapshot files,
- `metadata.json`, which maps subscription URLs to snapshot files and content hashes,
- `stable_top.json`, the previous run's saved top-N URIs used by `prioritize_stability` (created at the end of each refresh and deleted on every app startup and shutdown so it never survives across runs).

When a fresh HTTP subscription fetch succeeds, V2RayDAR writes a cache snapshot. If a fetched body is identical to an existing snapshot, it reuses the existing file and moves that snapshot to the newest position in metadata.

Local files and `data:` subscriptions are not used by cache fallback. Cache fallback supports HTTP and HTTPS subscription URLs.

## Restricted-Network Behavior

On very restricted networks, do not delete the cache unless you intentionally want to remove old subscription snapshots. The app can test cached HTTP subscription snapshots when fresh HTTP subscription URLs are unreachable.

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

To intentionally test only cached HTTP snapshots:

```yaml
use_cache_only: true
```

## Ranking

The final ranked list always puts reachable configs before failed configs. When `prioritize_stability: true`, reachable configs that were in the previous run's saved top-N are promoted before the remaining tie-breakers, so a higher-ping config that already proved working last refresh stays ahead of a newly discovered low-ping config. When `prioritize_stability: false`, the ranking simply prefers any working low-ping config without any carry-over.

The saved top-N is written to `stable_top.json` in the cache folder at the end of each refresh, re-pinged at the start of the next refresh, and deleted on app startup and shutdown so each fresh run begins with no stability carry-over.

The remaining tie-breakers are:

1. Lower `priority` values first.
2. Lower `latency_ms` first.
3. Higher `download_mbps` first, when speed testing is enabled.
4. Protocol.
5. Name.
6. URI.

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
| `cache/` | Cached HTTP subscription snapshots and the in-session top-N file. |
| `cache/metadata.json` | Cache index mapping subscription URLs to snapshots. |
| `cache/YYYY-MM-DD_HH-MM-SS.sss.txt` | Cached HTTP subscription body snapshot. |
| `cache/stable_top.json` | Previous run's saved top-N URIs used by `prioritize_stability`; deleted on every app startup and shutdown. |
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
| `src/probe.rs` | TCP probing, active `sing-box` probing, outbound conversion, speed testing. |
| `src/sing_box.rs` | `sing-box` availability/setup helpers and temporary proxy execution. |
| `src/server.rs` | Axum HTTP server, endpoint responses, LAN authorization. |
| `src/network.rs` | LAN IP discovery and sharing status. |
| `src/terminal.rs` | Plain terminal startup, progress, and summary output. |
| `src/model.rs` | Runtime state, ranked config, candidate, and serialized response models. |
| `src/tui.rs` and `src/tui/*` | Ratatui UI, input handling, config editing, panels, firewall integration. |
| `build.rs` | Windows resource/icon embedding. |

## Refresh Pipeline

The refresh pipeline in `src/main.rs` is roughly:

1. Load runtime config into shared state.
2. Fetch enabled subscriptions directly unless `use_cache_only` is true.
3. Parse fetched subscription bodies into candidates.
4. Probe candidates with `probe_candidates`. When `prioritize_stability: true`, the scheduler re-pings the previous run's saved top-N first.
5. If direct fetches failed, retry failed HTTP sources through `emergency_config` or a working config when active mode is available, then probe newly loaded retry candidates.
6. If no fresh subscription source was fetched successfully, try cached HTTP snapshots and probe cached candidates.
7. Apply stability ranking (carry the previous run's saved top-N to the front when enabled, otherwise rank purely by low ping).
8. Publish ranked state to `/subscription`, `/subscription.txt`, and `/results`.
9. Persist the new top-N to `stable_top.json` in the cache folder (when stability ranking is on) so the next refresh re-pings it first.
10. Record refresh duration, errors, byte counters, logs, and consecutive-top-N counters.

The refresh loop starts immediately on app launch. Later refreshes are driven by the timer or relevant config-file changes.

## HTTP Server Behavior

The HTTP server uses Axum and binds the configured `bind` address.

When `sharing.enabled` is true and the configured bind is loopback, the server also attempts to start a LAN listener on the primary detected LAN IP using the same port.

Authorization rules for `/subscription`, `/subscription.txt`, and `/results`:

- Loopback requests are accepted.
- LAN requests are rejected with `403` when sharing is disabled.
- LAN requests are rejected with `401` when token protection is enabled and the token is missing or incorrect.

Subscription response format:

- `/subscription` uses `encoded_subscription`.
- `/subscription.txt` is always raw.
- Both include only reachable configs and only the top `top_n` entries.
- The body ends with a trailing newline when at least one config is present.

## Cache Implementation

Cache snapshots are written only for successful HTTP/HTTPS fetches.

The cache metadata format is JSON and stores a map from subscription URL to snapshot records:

```json
{
  "subscriptions": {
    "https://example.com/sub": [
      {
        "file": "2026-06-10_12-30-00.000.txt",
        "hash": "0123456789abcdef"
      }
    ]
  }
}
```

The hash is an internal FNV-style body hash used for deduplicating identical snapshots. It is not a cryptographic integrity check.

Cache fallback loads the newest readable snapshot for each HTTP URL.

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
cargo test
```

Run formatting check:

```bash
cargo fmt --check
```

Run Clippy:

```bash
cargo clippy --all-targets --all-features -- -D warnings
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

Download from <https://github.com/SagerNet/sing-box/releases/latest>. Use the Windows archive for `sing-box.exe`, the Linux archive for Linux, the Android archive for Termux, and the Darwin archive for macOS. If you already use v2rayN on Windows, check the v2rayN installation folder for `sing-box.exe`.

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
- Fetches failed and no cache snapshots exist.

Check:

```text
http://127.0.0.1:27141/results
```

Look at `fetch_errors`, `ranked`, `last_error`, `tested_candidates`, and `reachable_candidates`.

### Subscription URLs fail on a restricted network

Keep the cache. Do not clean it unless you know you no longer need old snapshots.

Add a known working config:

```yaml
emergency_config: vless://uuid@example.com:443?security=tls&sni=example.com#bridge
```

Then keep active mode enabled so V2RayDAR can use that config through `sing-box` to retry failed HTTP subscription fetches.

### Config reload does not change the port

This is expected. Changing `bind` requires restarting V2RayDAR.

### Cache-only mode finds nothing

`use_cache_only` can only load cached HTTP/HTTPS subscription snapshots. It cannot load local files or `data:` URLs from cache fallback.

Disable cache-only mode:

```yaml
use_cache_only: false
```

Run one successful online refresh first so snapshots exist.

## Sample Minimal Config

```yaml
bind: 127.0.0.1:27141
top_n: 10
refresh_seconds: 300
encoded_subscription: true
prioritize_stability: true
scan_all_configs: false
fetch_timeout_ms: 30000
fetch_concurrency: 8
max_subscription_bytes: 33554432
use_cache_only: false
emergency_config: null

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

- Main README: [README.md](README.md)
- Example config: [configs.example.yaml](configs.example.yaml)
- Release guide: [RELEASE.md](RELEASE.md)
- License: [LICENSE](LICENSE)
- sing-box releases: <https://github.com/SagerNet/sing-box/releases/latest>
- sing-box configuration docs: <https://sing-box.sagernet.org/configuration/>
- v2rayN: <https://github.com/2dust/v2rayN>
- v2rayNG: <https://github.com/2dust/v2rayNG>
