<p align="center">
  <a href="https://deepwiki.com/411A/V2RayDAR">
    <img src="https://deepwiki.com/badge.svg" alt="Ask DeepWiki About V2RayDAR">
  </a>
</p>

<p align="center">
  <img src="assets/V2RayDAR_logo_v1.png" alt="V2RayDAR logo" width="200" height="200">
</p>

<h1 align="center">V2RayDAR</h1>

<p align="center">
  <em>V2Ray Detection And Reconnaissance — pronounced like <code>v2ray</code> + <code>radar</code>.</em>
</p>

<p align="center">
  A fast Rust CLI/TUI that fetches V2Ray subscription sources, validates them through your real network with <code>sing-box</code>, ranks the configs that actually work, and re-publishes the best ones at a local subscription URL your v2rayN / v2rayNG / sing-box client can point to.
</p>

<p align="center">
  📘 <a href="README_detailed.md">Read the detailed developer guide</a>
</p>

🖥️ Windows TUI Preview:

<p align="center">
  <img src="assets/Windows_TUI_v0.2.3.png" alt="Windows TUI" width="100%">
</p>

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

## Why V2RayDAR

- Pulls subscriptions in parallel from any number of sources you list.
- Parses raw, base64, JSON, and YAML feeds — and `vmess`, `vless`, `trojan`, `ss`, `ssr`, `hysteria2`, `hy2`, `tuic` share-links.
- **Parses Clash/Mihomo YAML configs** — add a Mihomo subscription URL and V2RayDAR extracts all proxy entries automatically.
- **Bidirectional format conversion** — converts between V2Ray share-links and Clash/Mihomo YAML proxy entries.
- Validates each candidate through your current network with `sing-box` (it actually loads a test URL through the proxy).
- **Dual-format output** — serves working configs as V2Ray share-links (`/subscription`) **and** as full Mihomo YAML configs (`/mihomo.yaml`), so any client can use them.
- Re-exposes the top working configs at a local URL so any compatible client just sees one always-fresh subscription.
- Survives restricted networks via previously-probed configs in the database, an in-network bridge config, or an `emergency_config`.
- Optional LAN sharing with optional token protection, so the phone in your pocket can use the same feed.

> [!WARNING]
> ### 🚧 Alpha Release
>
> This software is currently in **Alpha**.
>
> - Breaking changes may occur at any time.
> - Documentation may be incomplete.
> - Bugs and instability are expected.
> - Not recommended for production environments.
>
> Please report issues and feedback.

## Quick start

1. **Get sing-box**. Active probing needs a working `sing-box` executable. Use the installer with a `_with_singbox` release archive to get pinned `sing-box` 1.13.13 bundled beside V2RayDAR, or install it yourself. Termux users should install `sing-box=1.13.13` with `pkg`.
2. **Install V2RayDAR**. Use the one-liner installer above, grab a release binary from [Releases](https://github.com/411A/V2RayDAR/releases/latest), or build from source with `cargo run --release`.
3. **First launch** creates `configs.yaml`; portable mode keeps it in the `V2RayDAR` folder, while user-installed mode uses the platform app-data folder. If no bundled, Termux-package, or configured `sing-box` executable is found while `probe.mode: active`, the TUI asks for the full path.
4. **Point your client** at one of the local URLs below.

### Local URLs (default `127.0.0.1:27141`)

| Endpoint | Use for |
| --- | --- |
| `http://127.0.0.1:27141/subscription` | base64 subscription feed — what v2rayN / v2rayNG expect |
| `http://127.0.0.1:27141/subscription.txt` | the same, but plain newline-separated share-links |
| `http://127.0.0.1:27141/mihomo.yaml` | full Mihomo YAML config (raw, importable by Clash Verge / Mihomo) |
| `http://127.0.0.1:27141/results` | JSON diagnostics for the last refresh |
| `http://127.0.0.1:27141/health` | reachability check — returns `ok` |

### Run modes

```bash
# normal — TUI + local subscription endpoint
v2raydar

# headless — no TUI, just the endpoint and logs
v2raydar --no-tui

# one-shot — refresh once, print results, then exit
v2raydar --once

# ping — test config URIs and print latency results
v2raydar --ping "vless://uuid@server:443?security=tls#name"
v2raydar --ping-file configs.txt

# use a custom config file
v2raydar --config path/to/configs.yaml

# keep all data next to the executable
v2raydar --portable

# remove app data and owned firewall rules
v2raydar --uninstall
```

Windows users replace `v2raydar` with `v2raydar.exe`. On macOS open the bundled `.app` once and Gatekeeper will remember it.

## Default config at a glance

<details>
  <summary>👣 <strong>configs.yaml</strong> — table of every key, default, and what it does. Full explanations live in the <a href="README_detailed.md">detailed guide</a>.</summary>

| Key | Default | Purpose |
| --- | --- | --- |
| `bind` | `127.0.0.1:27141` | Local HTTP bind address for `/subscription`, `/subscription.txt`, `/results`, and `/health`. |
| `top_n` | `10` | Number of working configs published to clients. |
| `refresh_seconds` | `300` | Auto-refresh interval in seconds; `0` disables the timer. |
| `encoded_subscription` | `true` | Returns `/subscription` as base64 (v2rayN / v2rayNG friendly). |
| `prioritize_stability` | `true` | Re-pings the previous run's saved top-N first and keeps them at the front, even if new low-ping configs appear. When `false`, prefers any working low-ping config. |
| `return_configs_asap` | `false` | When `true`, publishes working configs to the endpoint and `Current Found Configs` as soon as they are found, up to `top_n`; early configs may not have the lowest ping or best stability. |
| `scan_all_configs` | `false` | When `true`, validates every loaded config instead of stopping after enough have been confirmed. |
| `fetch_timeout_ms` | `30000` | Per-source fetch timeout. |
| `fetch_concurrency` | `8` | Subscription sources fetched in parallel. |
| `max_subscription_bytes` | `33554432` | Size cap per fetched subscription source (32 MiB). |
| `use_cache_only` | `false` | When `true`, skip fresh fetches and load previously-probed configs from the database — useful on heavily restricted networks. |
| `emergency_config` | `null` | Optional working share-link used through `sing-box` as a bridge when HTTP subscription fetches fail. |
| `clean_offlines_after_days` | `7` | Days after which unreachable configs are removed from the database. |
| `sharing.enabled` | `false` | Lets LAN clients read the endpoints. |
| `sharing.require_token` | `false` | Requires `?token=...` for LAN requests. |
| `sharing.token` | `null` | Leave empty, set `true` to auto-generate, or supply a string. |
| `probe.mode` | `active` | `active` uses `sing-box`; `tcp` is diagnostic only. |
| `probe.sing_box_path` | `null` | Optional path to `sing-box`. Leave `null` for desktop `_with_singbox` builds or Termux's package path. |
| `probe.connect_timeout_ms` | `5000` | TCP connect timeout for diagnostic probing. |
| `probe.active_timeout_ms` | `30000` | HTTP test timeout in active mode. |
| `probe.startup_timeout_ms` | `5000` | Wait time for the temporary proxy to come up. |
| `probe.concurrency` | `16` | Base active-probing concurrency. |
| `probe.batch_size` | `20` | Initial active-probing batch size. |
| `probe.process_concurrency` | `null` | `sing-box` batch processes allowed at once; auto-scales when empty. |
| `probe.test_url` | `https://www.gstatic.com/generate_204` | URL loaded through each candidate. |
| `probe.accepted_statuses` | `[204, 200]` | HTTP statuses counted as success. |
| `probe.download_url` | `null` | Optional throughput-test target. |
| `probe.download_bytes_limit` | `1048576` | Upper bound for the optional download test. |
| `geoip_db_path` | `null` | Optional path to a `GeoLite2-Country.mmdb` file. If `null`, uses the embedded database for country detection. |
| `subscriptions` | _(two demo entries)_ | List of `{ name, url, enabled, priority }` sources. |

</details>

## Notes for restricted networks

- If you are on a very restricted network, previously-probed configs are stored in the database and can be used via `use_cache_only: true`.
- By default, if some HTTP subscription URLs don't connect on your network but one config is reachable, the app uses that config to retry those failed HTTP subscriptions too. And if there are no working configs on your network but you have one working config yourself, you can bring it into `configs.yaml`'s `emergency_config` so the app uses it to retry failed HTTP subscription fetches.

## Pointing common clients at V2RayDAR

- **v2rayN (same PC)** — keep `bind: 127.0.0.1:27141` and add `http://127.0.0.1:27141/subscription` as a subscription URL.
- **v2rayNG / phone on the same Wi-Fi** — bind to the PC's LAN IP (e.g. `192.168.1.23:27141`), turn on `sharing.enabled`, then use `http://192.168.1.23:27141/subscription` on the phone. Visit `/health` from the phone first to confirm reachability.

Full client walkthroughs, token-protected sharing, and OS-specific firewall details are in the [detailed guide](README_detailed.md).

## Contributing

PRs are welcome.

## Roadmap

- Add a cross-platform GUI app beside the TUI using Tauri.
- Extract V2Ray configs from the body of any website — preferably from non-JS-heavy sites, with Obscura as a fallback for the JS-heavy ones.
- Private endpoints with password requirements and authentication: when a subscription endpoint is private and password-protected, users can get their private endpoint that fetches the configs through a national reachable endpoint that has internet access.

## Warranty and responsibility

The app is published as-is, without any warranty.

The developer will not, by itself, create or distribute V2Ray-compatible configs, and is not responsible for the V2Ray subscriptions the user scans and connects to. The owner of the V2Ray server you connect to may be able to intercept your traffic and read your unencrypted data.
