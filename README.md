<p align="center">
  <a href="https://deepwiki.com/411A/V2RayDAR">
    <img src="https://deepwiki.com/badge.svg" alt="Ask DeepWiki About V2RayDAR">
  </a>
</p>

<p align="center">
  <strong>🌐 Available in</strong><br>
  <strong><a href="README.md">English</a></strong>
  • <strong><a href="docs/README.fa.md">فارسی</a></strong>
  • <strong><a href="docs/README.zh-CN.md">简体中文</a></strong>
  • <strong><a href="docs/README.ru.md">Русский</a></strong>
  • <strong><a href="docs/README.fr.md">Français</a></strong>
</p>

<p align="center">
  <img src="assets/V2RayDAR_logo_v1.png" alt="V2RayDAR logo" width="200" height="200">
</p>

<h1 align="center">V2RayDAR</h1>

<p align="center">
  <em>V2Ray Detection And Reconnaissance — pronounced like <code>v2ray</code> + <code>radar</code>.</em><br>
  <a href="https://github.com/411A/V2RayDAR/releases/latest"><img src="https://img.shields.io/github/v/release/411A/V2RayDAR" alt="Latest release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/411A/V2RayDAR" alt="License: AGPL-3.0"></a>
  <a href="https://github.com/411A/V2RayDAR/actions/workflows/rust.yml"><img src="https://github.com/411A/V2RayDAR/actions/workflows/rust.yml/badge.svg" alt="Rust CI status"></a>
</p>

<p align="center">
  <strong>Run it once on any always-on device — an old phone, PC, Raspberry Pi, or home server — and V2RayDAR continuously finds, verifies, and serves the best working configs to every device on your LAN. It also exposes a standard SOCKS5/HTTP proxy, so every device on your LAN gets a working V2Ray connection — no V2Ray client needed.</strong>
</p>

<p align="center">
  A fast Rust service with a built-in web dashboard that fetches V2Ray / Clash / Mihomo subscription sources, validates them through your real network with <code>sing-box</code>, ranks the configs that actually work, and re-publishes the best ones at a local subscription URL your v2rayN / v2rayNG / sing-box / Clash Verge / Mihomo client can point to. An optional terminal UI (<code>--tui</code>) covers a few extra maintenance actions.
</p>

<p align="center">
  📘 <a href="docs/guide.md">Detailed guide</a>
  • 🧠 <a href="https://deepwiki.com/411A/V2RayDAR">Ask DeepWiki</a>
  • 📡 <a href="#-subscription-endpoints">Endpoints</a>
  • 🌐 <a href="#-persistent-proxy">Proxy</a>
</p>

## 📖 Table of Contents

- [✨ Features](#-features)
- [🖥️ Interfaces](#-interfaces)
- [📦 Installation](#-installation)
  - [<img src="https://cdn.svglogos.dev/logos/linux-tux.svg" alt="" width="16" height="16"> Linux / macOS](#-linux--macos)
  - [<img src="https://cdn.svglogos.dev/logos/microsoft-windows-icon.svg" alt="" width="16" height="16"> Windows](#-windows-powershell)
  - [<img src="https://cdn.svglogos.dev/logos/android-icon.svg" alt="" width="16" height="16"> Android / Termux](#-android--termux)
- [🚀 Quick Start](#-quick-start)
- [⚙️ Configuration](#-configuration)
- [📡 Client Setup](#-client-setup)
- [🌐 Persistent Proxy](#-persistent-proxy)
- [🔒 Restricted Networks](#-restricted-networks)
- [🤝 Contributing](#-contributing)
- [⚠️ Disclaimer & Security](#-disclaimer--security)
- [☕ Support](#-support)
- [📄 License](#-license)

## ✨ Features

### 🔎 Discovery & Validation

- **Parallel fetching** — pulls any number of subscription sources concurrently.
- **Multi-format parsing** — reads raw, base64, JSON, and YAML feeds.
- **Share-link protocols** — extracts `vmess`, `vless`, `trojan`, `ss`, `ssr`, `hysteria2`, `hy2`, and `tuic` links.
- **Real-network validation** — tests each candidate through your current network with `sing-box` by loading a test URL through the proxy.
- **Smart ranking** — ranks the configs that actually work by priority, latency, stability, and speed.

### 🔄 Format Conversion & Output

- **Clash/Mihomo input** — add a Mihomo subscription URL and V2RayDAR extracts all proxy entries automatically.
- **Bidirectional conversion** — converts between V2Ray share-links and Clash/Mihomo YAML proxy entries.
- **Dual-format output** — serves working configs as V2Ray share-links (`/subscription`) **and** as full Mihomo YAML configs (`/mihomo.yaml`), so any client can use them.
- **One fresh feed** — re-exposes the top working configs at a local URL so any compatible client just sees one always-fresh subscription.

### 🌐 Persistent Proxy

- **Always-on proxy** — keeps a `sing-box` process running with the best config, exposing a local mixed SOCKS5/HTTP proxy any app can use — no V2Ray client needed.
- **Automatic failover** — switches to the next best config when the current one fails, and to a better config on each refresh cycle.
- **Rotation control** — `proxy.rotating_proxy` selects lowest-ping switching or sticky-config behavior.

### 📱 LAN Sharing & QR Onboarding

- **LAN subscription sharing** — with optional token protection, so the phone in your pocket can use the same feed.
- **LAN proxy sharing** — bind `0.0.0.0` with automatic firewall rules, plus one-tap Telegram setup.
- **QR code sheet** — scannable codes for the LAN subscription and the Telegram proxy: dashboard Share tab, per-config QR buttons, or the TUI menu (desktop, saved to `v2raydar_data/QRCodes.jpg`).

### 🔒 Restricted Networks

- **Probed-config database** — previously verified configs survive network restrictions and can refill the feed via `use_cache_only`.
- **In-network bridge** — a reachable config retries failed subscription fetches through `sing-box`.
- **Emergency config** — your own known-working link as an explicit fallback.

## 🖥️ Interfaces

### 🌐 Web Dashboard (default)

The web dashboard is the default interface and covers daily use end to end from a single browser page: live Overview stats, ranked Configs with per-row QR codes, Subscriptions management (add, edit, on/off, delete, drag-and-drop reorder), every setting on the Settings tab, the Proxy tab, LAN sharing, live Logs, and a QR sheet for onboarding phones.

After starting the app, open http://127.0.0.1:27141 in your browser.

<p align="center">
  <img src="assets/WebUI_v0.9.0.png" alt="V2RayDAR web dashboard" width="100%">
</p>

<p align="center">
  <img src="assets/WebUI-Mobile_v0.9.0.png" alt="V2RayDAR web dashboard on mobile" width="300">
</p>

### 🖥️ Optional TUI (`--tui`)

Run with `v2raydar --tui` for the classic terminal UI alongside the dashboard and the endpoint. It additionally offers cache cleanup and reset-to-defaults — everything else lives in the dashboard too.

<p align="center">
  <img src="assets/Windows_TUI_v0.9.0.png" alt="V2RayDAR TUI on Windows" width="100%">
</p>

## 📦 Installation

Run the installer for your platform. It uses default settings (press Enter to accept) or answers step by step if you decline; updating in place keeps your data.

What the installer does: detects your platform, downloads the latest release with bundled `sing-box`, verifies SHA-256 checksums, detects existing installations and offers to update (preserving `data.db` and `v2raydar_data/`), and never requires sudo by default.

#### <img src="https://cdn.svglogos.dev/logos/linux-tux.svg" alt="Linux" width="20" height="20" align="texttop"> Linux / macOS

```bash
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh
```

#### <img src="https://cdn.svglogos.dev/logos/microsoft-windows-icon.svg" alt="Windows" width="20" height="20" align="texttop"> Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/411A/V2RayDAR/main/install.ps1 | iex
```

#### <img src="https://cdn.svglogos.dev/logos/android-icon.svg" alt="Android" width="20" height="20" align="texttop"> Android / Termux

```bash
pkg update -y && apt update && apt full-upgrade -y && pkg install -y curl tar && curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | bash && cd ~/V2RayDAR && ./v2raydar
```

### 👤 User Install

Binary to `~/.local/bin`, data in home:

```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh -s -- --user

# Windows
irm https://raw.githubusercontent.com/411A/V2RayDAR/main/install.ps1 | iex
# Then choose option 2 when prompted
```

* **Stop:** `Ctrl + C`
* **Start:** `cd ~/V2RayDAR && ./v2raydar`

### 📥 Manual Download

Grab the archive for your OS from [Releases](https://github.com/411A/V2RayDAR/releases/latest) and run it.

**Portable** (recommended) — everything in one folder. A self-contained folder (bundled `sing-box` or an existing `v2raydar_data/` beside the executable) is detected automatically, so double-clicking just works — `--portable` forces it anywhere. Portable mode installs into `Desktop/V2RayDAR` when a Desktop folder exists, otherwise `~/V2RayDAR`.

## 🚀 Quick Start

After installing with the script above, run `v2raydar` (or `v2raydar.exe` on Windows). On first launch it initializes `data.db` with default settings and a set of pre-selected subscription sources to get you started. Upgrading keeps everything: an existing `configs.yaml` is migrated into `data.db` automatically.

1. **Wait for it to populate.** The app fetches your subscription sources in parallel, probes each config through your real network, and ranks the ones that work. The endpoint is live from the start — your client can point to it immediately.
2. **Point your client** at one of the [subscription endpoints](#-subscription-endpoints) below.
3. **Use the dashboard** at `http://127.0.0.1:27141` — Overview, Configs, Subscriptions, Settings, Proxy, Logs, and Share tabs. Everything saves instantly to the database and the live runtime; settings that affect refresh apply on the next cycle (the dashboard says so on save), while feed-list changes re-fetch at once.
4. **Change settings** from the dashboard Settings tab (or the TUI Configurations screen with `--tui`) — changes save immediately, with refresh-affecting settings taking effect on the next scheduled cycle or a manual refresh. Key settings: `top_n`, `refresh_seconds`, `ping_seconds`, `sharing.enabled`, `probe.mode`. Settings added by newer versions default automatically; your stored values are never overwritten.
5. **Exit** with `Ctrl + C`. The endpoint stops when the app exits.

### 📡 Subscription Endpoints

Three output formats, one for each client family:

| Client | Endpoint |
| --- | --- |
| v2rayN / v2rayNG | `http://127.0.0.1:27141/subscription` (base64) |
| sing-box | `http://127.0.0.1:27141/subscription.txt` (plain) |
| Clash Verge / Mihomo | `http://127.0.0.1:27141/mihomo.yaml` |

### ⚙️ Run Modes

```bash
v2raydar                # quiet — only a browser hint, no logs
v2raydar --no-tui       # headless with details and logs, no TUI
v2raydar --tui          # TUI + local subscription endpoint
v2raydar --once         # refresh once, print results, exit
v2raydar --portable     # keep all data next to the executable (auto-detected in portable folders)
v2raydar --uninstall    # remove app data and owned firewall rules
```

Windows users replace `v2raydar` with `v2raydar.exe`. On macOS open the bundled `.app` once and Gatekeeper will remember it.

<details>
  <summary>🖥️ <strong>TUI keyboard controls</strong></summary>

| Key | Action |
| --- | --- |
| `↑` / `↓` or `j` / `k` | Navigate |
| `Enter` | Select / toggle / confirm |
| `Esc` / `Ctrl+H` | Go back |
| `Space` | Toggle subscription on/off |
| `e` | Edit selected subscription |
| `Ctrl+R` | Manual refresh — re-fetch once, unless a refresh is running |
| `Ctrl+P` | Manual re-ping of cached configs, unless any cycle is running |
| `q` | Quit |
| `:` | Command mode — `:q` quit, `:w` save, `:a` add, `:d` delete, `:n` rename, `:u` URL, `:p` priority, `:r` refresh, `:ping` |

</details>

## ⚙️ Configuration

<details>
  <summary>👣 <strong>View all settings and defaults</strong></summary>

| Key | Default | Purpose |
| --- | --- | --- |
| `bind` | `127.0.0.1:27141` | Local HTTP bind address for `/subscription`, `/subscription.txt`, `/results`, and `/health`. |
| `top_n` | `10` | Number of working configs published to clients. |
| `refresh_seconds` | `900` | Auto-refresh interval in seconds; `0` disables the timer. |
| `ping_seconds` | `300` | Re-ping interval in seconds for cached configs without re-fetching; `0` disables. When the cache verifies fewer than `top_n`, the ping also probes previously-seen database configs to refill. Both count toward Sub Usage. |
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
| `proxy.enabled` | `false` | Starts a persistent `sing-box` process exposing a mixed SOCKS5/HTTP proxy. |
| `proxy.port` | `27910` | Port for the mixed SOCKS5/HTTP proxy. |
| `proxy.discoverable` | `false` | Binds to `0.0.0.0` and adds a firewall rule for LAN access. |
| `proxy.rotating_proxy` | `true` | `true` switches the proxy to the lowest-ping config on each cycle; `false` keeps the current config while it stays reachable. |
| `proxy.health_check_url` | `https://cp.cloudflare.com` | URL tested through the proxy to verify it's alive. |
| `proxy.health_check_interval_seconds` | `60` | Seconds between proxy health checks. Auto-failover on failure. |
| `probe.mode` | `active` | `active` uses `sing-box`; `tcp` is diagnostic only. |
| `probe.sing_box_path` | `null` | Optional path to `sing-box`. Leave `null` for desktop `_with_singbox` or Termux builds with bundled `sing-box`. |
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
| `geoip_db_path` | `null` | Optional path to a `GeoLite2-Country.mmdb` file or a country-zone directory (`zones.txt`, or legacy `<cc>.zone` files). If `null`, uses `<data-root>/geoip` (MaxMind database first, zone fallback; both refreshed by the installer). Country data: GeoLite2 by MaxMind (CC BY-SA 4.0); fallback zones by ipdeny. |
| `subscriptions` | _(pre-selected sources)_ | List of `{ name, url, enabled, priority }` sources. Add your own for better results. |

</details>

### 🗄️ Viewing the database

You can view or change (not recommended) the database file (`data.db`) by downloading & using this free software: https://sqlitebrowser.org/dl

Note that changing the database directly may cause issues in the system.

For detailed behavior, examples, migration notes, and advanced configuration, see the [detailed guide](docs/guide.md).

## 📡 Client Setup

### v2rayN (same PC)

Keep `bind: 127.0.0.1:27141` and add `http://127.0.0.1:27141/subscription` as a subscription URL.

### v2rayNG / phone on the same Wi-Fi

Bind to the PC's LAN IP (e.g. `192.0.2.23:27141`), turn on `sharing.enabled`, then use `http://192.0.2.23:27141/subscription` on the phone. Visit `/health` from the phone first to confirm reachability.

### sing-box (plain share-link format)

```text
http://127.0.0.1:27141/subscription.txt
```

### Clash Verge / Mihomo (YAML config format)

```text
http://127.0.0.1:27141/mihomo.yaml
```

Import this URL directly in your Clash client's profile/subscription settings. V2RayDAR generates a complete Mihomo config with proxy entries, a `url-test` proxy group, and a catch-all `MATCH` rule.

Full client walkthroughs, token-protected sharing, and OS-specific firewall details are in the [detailed guide](docs/guide.md).

## 🌐 Persistent Proxy

V2RayDAR can run a persistent SOCKS5/HTTP proxy alongside the subscription endpoint. Any app on the system — Telegram, browsers, curl, Python — can route traffic through it without a separate VPN client.

**Enable from the dashboard Proxy tab or the TUI Proxy row**
(`enabled: true`, port `27910`, `discoverable: true` = LAN access + firewall rule).

### Local usage (on the device running V2RayDAR)

```bash
# SOCKS5
curl --socks5 127.0.0.1:27910 https://api.ipify.org

# HTTP
curl --proxy http://127.0.0.1:27910 https://api.ipify.org
```

### LAN usage (phone on same Wi-Fi)

1. Set `proxy.discoverable: true` — V2RayDAR adds a firewall rule and binds to `0.0.0.0`.
2. Find your PC's LAN IP in the dashboard Overview tab under **Network** (or the TUI's **Current Configuration** panel, or run `ipconfig` / `ip addr`). For example `192.0.2.2`.

### Telegram

Replace `YOUR_LAN_IP` with your actual LAN IP and open this URL on your phone:

```text
https://t.me/socks?server=YOUR_LAN_IP&port=27910
```

For example, if your LAN IP is `192.0.2.2`:

```text
https://t.me/socks?server=192.0.2.2&port=27910
```

Or manually: Telegram → Settings → Data and Storage → Proxy Settings → Add Proxy:
- Type: **SOCKS5** or **HTTP**
- Host: `YOUR_LAN_IP` (the IP shown in the dashboard or the TUI panel)
- Port: `27910`

### Android system proxy

Settings → WiFi → long-press your network → Modify → Advanced → Proxy → Manual → Server: `YOUR_LAN_IP`, Port: `27910`.

## 🔒 Restricted Networks

Three fallback mechanisms, in order:

1. **Previously-probed configs** — stored in the database and usable via `use_cache_only: true`.
2. **In-network bridge** — if some subscription URLs don't connect but one config is reachable, the app uses that config to retry those failed fetches. By default this happens automatically; if there are no working configs on your network but you have one working config yourself, set it as `emergency_config` (dashboard Settings tab or TUI Configurations screen) so the app uses it to retry failed fetches.
3. **Emergency config** — your own known-working share-link as an explicit fallback.

See [Restricted-Network Behavior](docs/guide.md#restricted-network-behavior) in the detailed guide for the full retry order and examples.

## 🤝 Contributing

Contributions are welcome! Feel free to open an Issue for bugs, feature requests, questions, or suggestions, or submit a Pull Request. Any feedback is greatly appreciated.

🤖 V2RayDAR is engineered and coded by its human maintainer with help from multiple AI assistants — human-reviewed contributions, whether from people or their AI helpers, are equally welcome.

## ⚠️ Disclaimer & Security

### Disclaimer

The app is published as-is, without any warranty.

### Third-party configurations

V2RayDAR does not create or distribute V2Ray-compatible configs. It only scans subscription sources that you configure and republishes the working configs it finds on your own machine. You are responsible for the subscription URLs and configs you scan, import, and connect to.

### Security warning

The owner of the V2Ray server you connect to may be able to intercept your traffic and read your unencrypted data. Prefer `127.0.0.1:27141` for same-machine use, use `sharing.require_token: true` on shared or less trusted LANs, and never expose V2RayDAR's HTTP endpoint to the public internet. Treat subscription URLs and share links as sensitive.

## ☕ Support

### 💬 Contact

<p align="center">
<a href="https://t.me/TechKrakenBot">
  <img src="https://img.shields.io/badge/Telegram-2CA5E0?style=for-the-badge&logo=telegram&logoColor=white" alt="Telegram Bot">
</a>
</p>

### 💎 Donate via TON

If you find this project helpful, you can support its development through donations on the TON blockchain:

```text
ton://transfer/TechKraken.ton
```

```text
UQCGk4IU5nm6dYWjXTx6vSQVOtKO4LQg3m8cRcq1eQo7vhCl
```

## 📄 License

V2RayDAR is licensed under the **GNU Affero General Public License v3.0 (AGPL-3.0)** — see [LICENSE](LICENSE).
