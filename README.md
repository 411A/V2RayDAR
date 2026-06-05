# V2RayDAR

V2RayDAR is a small Rust CLI that fetches V2Ray-style subscriptions, validates configs through the user's current network, ranks the configs that can load a real test URL, and exposes the best configs through a local subscription endpoint.

Project name: V2Ray Detection And Reconnaissance, pronounced like `v2ray` + `radar`.

## Current Phase

Phase 1 is a fast scanner and local subscription server:

- Creates a per-user app folder and `config.yaml` on first run.
- Supports `--portable` and `--config` for self-contained or development runs.
- Fetches subscription sources by priority.
- Parses common share links such as `vmess://`, `vless://`, `trojan://`, `ss://`, `ssr://`, `hysteria2://`, `hy2://`, and `tuic://`.
- Starts `sing-box` for active validation and sends an HTTP request through each candidate config.
- Ranks configs that successfully load the configured test URL through the proxy.
- Exposes the top N configs from a local HTTP endpoint.

Important: the default probe mode is `active`, which requires `sing-box`. V2RayDAR does not publish a config unless `sing-box` can start that config and a real HTTP request succeeds through it. A TCP-only mode still exists for diagnostics, but it is not suitable when you need configs that are actually connectable in v2rayNG or v2rayN.

Active validation currently converts VMess, VLESS, Trojan, Shadowsocks, Hysteria2, and TUIC share links into temporary sing-box configs. SSR links are parsed for diagnostics, but they are not published in active mode unless active conversion support is added later.

## Requirements

- Rust toolchain with Cargo.
- `sing-box` installed and available on `PATH`, or configured with `probe.sing_box_path`.
- Internet access from the machine running V2RayDAR so it can fetch subscription URLs.
- Android and the PC must be on the same network if Android v2rayNG will read the endpoint from the PC.

Install or verify `sing-box` before using active mode:

```bash
sing-box version
```

If `sing-box` is not on `PATH`, set an absolute path:

```yaml
probe:
  mode: active
  sing_box_path: /usr/local/bin/sing-box
```

Windows example:

```yaml
probe:
  mode: active
  sing_box_path: C:\Tools\sing-box\sing-box.exe
```

## Quick Start

Run V2RayDAR. On first run it creates the app folder, default config, and runtime subfolders automatically.

Development:

```bash
cargo run
```

Release binary:

```powershell
target\release\v2raydar.exe
```

The terminal output shows the app folder and config path. Edit that generated `config.yaml` and add your subscription sources:

```yaml
bind: 127.0.0.1:14127
top_n: 10
refresh_seconds: 300
encoded_subscription: true
fetch_timeout_ms: 15000
fetch_concurrency: 4

sharing:
  enabled: false
  require_token: false
  token: AUTO_GENERATED_ON_FIRST_RUN

probe:
  mode: active
  sing_box_path: sing-box
  connect_timeout_ms: 1500
  active_timeout_ms: 10000
  startup_timeout_ms: 2000
  concurrency: 16
  test_url: https://www.gstatic.com/generate_204
  accepted_statuses: [204, 200]
  download_url:
  download_bytes_limit: 1048576

subscriptions:
  - name: primary
    url: https://your-subscription-url
    priority: 1
```

Run once without starting the HTTP server:

```bash
cargo run -- --once
```

Run continuously and serve the endpoint:

```bash
cargo run
```

V2RayDAR watches the active config file while it runs. Most fields take effect live, including `top_n`, `refresh_seconds`, `encoded_subscription`, sharing settings, fetch settings, probe settings, and subscriptions. Changing `bind` requires restarting V2RayDAR because the HTTP listener is already open on the old address.

For portable mode, keep the app data beside the executable:

```bash
v2raydar --portable
```

For development or tests with an explicit config path:

```bash
v2raydar --config configs.example.yaml --once
```

For a release build:

```bash
cargo build --release
target/release/v2raydar
```

On Windows, the release binary is:

```powershell
target\release\v2raydar.exe
```

## Config Fields

Top-level keys:

| Key | Type | Default | Possible values | Hot reload | Description |
| --- | --- | --- | --- | --- | --- |
| `bind` | Socket address | `127.0.0.1:14127` | `IP:PORT`, for example `127.0.0.1:14127`, `0.0.0.0:14127`, `192.168.1.23:14127` | Restart required | HTTP bind address for `/subscription`, `/subscription.txt`, `/results`, and `/health`. Use `127.0.0.1` for same-machine clients. Use `0.0.0.0` or a LAN IP for Android/LAN clients. |
| `top_n` | Positive integer | `10` | `1` or higher | Yes | Maximum number of validated working configs exposed by subscription endpoints. |
| `refresh_seconds` | Integer seconds | `300` | `0` or higher | Yes | Time between automatic subscription refreshes. `0` disables timer refresh, but saved config changes still trigger a reload. |
| `encoded_subscription` | Boolean | `true` | `true`, `false` | Yes | When `true`, `/subscription` returns a base64-encoded newline list. Keep `true` for v2rayNG/v2rayN unless you know your client wants raw links. |
| `fetch_timeout_ms` | Integer milliseconds | `15000` | `1` or higher | Yes | Timeout for fetching each subscription source. |
| `fetch_concurrency` | Positive integer | `4` | `1` or higher | Yes | Number of same-priority subscription sources fetched concurrently. |
| `sharing` | Object | See sharing table | Sharing object | Yes | Controls LAN exposure and optional URL token protection. |
| `probe` | Object | See probe table | Probe object | Yes | Controls validation strategy, sing-box path, timeouts, concurrency, and optional speed measurement. |
| `subscriptions` | Array | `[]` | Zero or more subscription objects | Yes | Subscription sources to fetch and test. Lower `priority` values are processed first. A fresh install starts empty. |

Sharing keys:

| Key | Type | Default | Possible values | Hot reload | Description |
| --- | --- | --- | --- | --- | --- |
| `sharing.enabled` | Boolean | `false` | `true`, `false` | Yes | Enables LAN clients to read `/subscription`, `/subscription.txt`, and `/results`. Same-machine clients can still use `127.0.0.1`. |
| `sharing.require_token` | Boolean | `false` | `true`, `false` | Yes | When enabled for LAN clients, subscription URLs must include `?token=...`. |
| `sharing.token` | String | Generated on first run | URL-safe token string | Yes | Token used when `sharing.require_token` is true. Regenerate if the URL was shared too widely. |

Probe keys:

| Key | Type | Default | Possible values | Hot reload | Description |
| --- | --- | --- | --- | --- | --- |
| `probe.mode` | String enum | `active` | `active`, `tcp` | Yes | `active` starts `sing-box` and loads a real HTTP URL through the candidate config. `tcp` only checks TCP connect and can produce false positives. Use `active` for normal operation. |
| `probe.sing_box_path` | String path | `sing-box` | Executable name on `PATH` or absolute path, for example `/usr/local/bin/sing-box`, `C:\Tools\sing-box\sing-box.exe` | Yes | sing-box executable used by active probes. Required when `probe.mode` is `active`. |
| `probe.connect_timeout_ms` | Integer milliseconds | `1500` | `1` or higher | Yes | TCP connection timeout used only by `probe.mode: tcp`. |
| `probe.active_timeout_ms` | Integer milliseconds | `10000` | `1` or higher | Yes | Timeout for the HTTP request sent through the candidate config in active mode. |
| `probe.startup_timeout_ms` | Integer milliseconds | `2000` | `1` or higher | Yes | Timeout while waiting for the temporary local sing-box mixed proxy to become ready. |
| `probe.concurrency` | Positive integer | `16` | `1` or higher | Yes | Number of configs tested at once. Higher values are faster but spawn more sing-box processes and use more CPU/RAM/network. |
| `probe.test_url` | URL string | `https://www.gstatic.com/generate_204` | Any `http://` or `https://` URL reachable from a working proxy | Yes | Connectivity URL loaded through every candidate config. Choose a small, stable URL that works from your network. |
| `probe.accepted_statuses` | Array of HTTP status codes | `[204, 200]` | HTTP status integers, for example `[204]`, `[200]`, `[200, 204, 301, 302]` | Yes | HTTP statuses treated as active-probe success for `probe.test_url`. |
| `probe.download_url` | Optional URL string | Empty/null | Empty, `null`, or any `http://`/`https://` URL | Yes | Optional download URL used after the active connectivity probe succeeds. Leave empty to rank by active HTTP latency only. |
| `probe.download_bytes_limit` | Positive integer bytes | `1048576` | `1` or higher | Yes | Maximum bytes counted for optional download Mbps measurement. The probe sends an HTTP `Range` request, but the server may ignore it. |

Subscription keys:

| Key | Type | Default | Possible values | Hot reload | Description |
| --- | --- | --- | --- | --- | --- |
| `subscriptions[].name` | String | Required | Any non-empty label, for example `primary`, `backup`, `work-isp` | Yes | Local source name shown in `/results` and terminal output. |
| `subscriptions[].url` | String | Required | `https://...`, `http://...`, `file://...`, local file path, or `data:` URL | Yes | Subscription source. Content may be base64 newline links, raw newline links, JSON/YAML containers, or supported DataURL content. |
| `subscriptions[].priority` | Integer | `100` | `0` or higher | Yes | Lower number means higher priority. Priority `1` is ranked before priority `2` when validation quality is otherwise equal. |

Supported config file extensions:

| Extension | Parser | Notes |
| --- | --- | --- |
| `.yaml` | YAML | Recommended in examples because it is explicit. |
| `.yml` | YAML | Fully supported shorthand extension. |
| `.json` | JSON | Supported for users who prefer JSON config files. |

Supported subscription source URL formats:

| Format | Example | Notes |
| --- | --- | --- |
| HTTPS URL | `https://example.com/subscription` | Recommended for remote subscriptions. |
| HTTP URL | `http://example.com/subscription` | Supported, but HTTPS is safer when available. |
| File URL | `file:///home/user/sub.txt` | Reads a local subscription file. |
| Local path | `/home/user/sub.txt`, `C:\Users\me\sub.txt` | Reads a local subscription file directly. |
| Data URL | `data:,vless://uuid@example.com:443%23demo` | Useful for tests or tiny inline subscriptions. |

## Local Endpoints

When the app is running with the default config:

- `http://127.0.0.1:14127/subscription` returns the top N actively validated configs, base64 encoded by default.
- `http://127.0.0.1:14127/subscription.txt` returns the same validated configs as raw newline-separated share links.
- `http://127.0.0.1:14127/results` returns JSON diagnostics with all ranked configs.
- `http://127.0.0.1:14127/health` returns `ok`.

LAN clients are blocked by default. To allow LAN access, set:

```yaml
bind: 0.0.0.0:14127
sharing:
  enabled: true
  require_token: false
```

Open LAN subscription URL:

```text
http://192.168.1.23:14127/subscription
```

For tokenized LAN access:

```yaml
bind: 0.0.0.0:14127
sharing:
  enabled: true
  require_token: true
  token: your-generated-token
```

Tokenized LAN subscription URL:

```text
http://192.168.1.23:14127/subscription?token=your-generated-token
```

## Using With v2rayN on Windows

If v2rayN runs on the same Windows machine as V2RayDAR, keep:

```yaml
bind: 127.0.0.1:14127
```

Start V2RayDAR:

```powershell
target\release\v2raydar.exe
```

In v2rayN:

1. Open the subscription settings or subscription group settings.
2. Add a new subscription.
3. Use a clear alias such as `V2RayDAR Top 10`.
4. Set the URL to:

```text
http://127.0.0.1:14127/subscription
```

5. Save it.
6. Run the client action that updates subscriptions.
7. Select one of the imported configs and connect.

If v2rayN is on another Windows machine in the same LAN, bind V2RayDAR to a LAN-reachable address, enable LAN sharing, and use the server PC's LAN IP:

```yaml
bind: 0.0.0.0:14127
sharing:
  enabled: true
  require_token: false
```

Subscription URL from the other Windows machine:

```text
http://PC_LAN_IP:14127/subscription
```

Example:

```text
http://192.168.1.23:14127/subscription
```

## Using With v2rayNG on Android

Android cannot reach `127.0.0.1` on your PC. On Android, `127.0.0.1` means the Android device itself.

To use v2rayNG from Android:

1. Make sure the Android device and the PC running V2RayDAR are on the same Wi-Fi or LAN.
2. In `config.yaml`, bind V2RayDAR to a LAN-reachable address and enable LAN sharing:

```yaml
bind: 0.0.0.0:14127
sharing:
  enabled: true
  require_token: false
```

3. Start V2RayDAR on the PC:

```bash
cargo run
```

4. Find the PC LAN IP.

Linux:

```bash
ip addr
```

Windows PowerShell:

```powershell
ipconfig
```

5. On the Android device, test the endpoint in a browser:

```text
http://PC_LAN_IP:14127/health
```

If it shows `ok`, the phone can reach V2RayDAR.

6. In v2rayNG, open subscription group settings.
7. Add a new subscription group.
8. Use a clear remarks/name value such as `V2RayDAR Top 10`.
9. Set the URL to:

```text
http://PC_LAN_IP:14127/subscription
```

Example:

```text
http://192.168.1.23:14127/subscription
```

10. Save it.
11. Run the app action that updates subscriptions.
12. Select one of the imported configs and connect.

If Android cannot open `/health`, check:

- The PC firewall allows inbound TCP traffic to the V2RayDAR port.
- The PC and Android device are on the same network.
- The endpoint uses the PC LAN IP, not `127.0.0.1`.
- V2RayDAR is still running.

## Security Notes

The default `bind` is `127.0.0.1:14127`, which is only reachable from the same machine. The default `sharing.enabled` is `false`, so LAN clients cannot read subscription data even if you later bind to a LAN address until sharing is enabled.

If you use `0.0.0.0:14127` and `sharing.enabled: true`, anyone on the reachable network may be able to fetch your top configs unless `sharing.require_token` is enabled. Use a trusted LAN, firewall rules, tokenized URLs, or a more specific bind address when possible.

Do not expose the endpoint to the public internet.

## Troubleshooting

No configs are imported by the client:

- Open `/subscription.txt` in a browser and confirm it contains share links.
- Keep `encoded_subscription: true` for `/subscription`.
- Try `/subscription.txt` only if the client explicitly supports raw URL-line subscriptions.

Android cannot reach V2RayDAR:

- Do not use `127.0.0.1` from Android.
- Use `bind: 0.0.0.0:14127`.
- Set `sharing.enabled: true`.
- Use `http://PC_LAN_IP:14127/subscription`.
- If `sharing.require_token: true`, include `?token=...` in the URL.
- Check firewall rules.

All configs are unreachable:

- Confirm `sing-box version` works from the same terminal.
- Increase `probe.active_timeout_ms`.
- Confirm the original subscriptions still contain valid links.
- Confirm the PC running V2RayDAR can reach the proxy server addresses.
- Check `/results` for active probe errors, such as unsupported share-link fields or HTTP probe failures.
- Try another `probe.test_url` if your network blocks the default connectivity endpoint.

The endpoint is empty:

- V2RayDAR only exposes configs that pass the active probe.
- Check `/results` to see failed candidates and errors.
- Check the terminal summary after each refresh.

## References

- v2rayNG official repository: https://github.com/2dust/v2rayNG
- v2rayN official repository: https://github.com/2dust/v2rayN
- V2Fly subscription service documentation: https://www.v2fly.org/en_US/v5/config/service/subscription.html
- sing-box configuration documentation: https://sing-box.sagernet.org/configuration/
- Xray outbound configuration documentation: https://xtls.github.io/en/config/outbounds/
