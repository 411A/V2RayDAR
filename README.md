# V2RayDAR

V2RayDAR is a small Rust CLI that fetches V2Ray-style subscriptions, finds the configs whose server endpoints are reachable on the current network, ranks them, and exposes the best configs through a local subscription endpoint.

Project name: V2Ray Detection And Reconnaissance, pronounced like `v2ray` + `radar`.

## Current Phase

Phase 1 is a fast scanner and local subscription server:

- Loads `configs.yaml`, `configs.yml`, or `configs.json`.
- Fetches subscription sources by priority.
- Parses common share links such as `vmess://`, `vless://`, `trojan://`, `ss://`, `ssr://`, `hysteria2://`, `hy2://`, and `tuic://`.
- Runs concurrent TCP reachability and latency checks.
- Ranks reachable configs.
- Exposes the top N configs from a local HTTP endpoint.

Important: Phase 1 checks TCP reachability and latency. It does not yet perform a full proxy handshake or real download-speed test through the config. That requires running a maintained core such as Xray, V2Ray, or sing-box, which is planned separately in `PLAN.md`.

## Requirements

- Rust toolchain with Cargo.
- Internet access from the machine running V2RayDAR so it can fetch subscription URLs.
- Android and the PC must be on the same network if Android v2rayNG will read the endpoint from the PC.

## Quick Start

Create your runtime config from the example. Both `.yaml` and `.yml` are supported; this README uses `.yaml` because it is the less abbreviated YAML extension.

Linux/macOS:

```bash
cp configs.example.yaml configs.yaml
```

Windows PowerShell:

```powershell
Copy-Item configs.example.yaml configs.yaml
```

Edit `configs.yaml` and replace the example subscription URL:

```yaml
bind: 127.0.0.1:14127
top_n: 10
refresh_seconds: 300
encoded_subscription: true
fetch_timeout_ms: 15000
fetch_concurrency: 4

probe:
  connect_timeout_ms: 1500
  concurrency: 256

subscriptions:
  - name: primary
    url: https://example.com/subscription
    priority: 1
```

Run once without starting the HTTP server:

```bash
cargo run -- --config configs.yaml --once
```

Run continuously and serve the endpoint:

```bash
cargo run -- --config configs.yaml
```

V2RayDAR watches the config file while it runs. When you edit and save `configs.yaml`, it reloads the new settings and refreshes the ranked configs automatically.

Most fields take effect live, including `top_n`, `refresh_seconds`, `encoded_subscription`, fetch settings, probe settings, and subscriptions. Changing `bind` requires restarting V2RayDAR because the HTTP listener is already open on the old address.

For a release build:

```bash
cargo build --release
target/release/v2raydar --config configs.yaml
```

On Windows, the release binary is:

```powershell
target\release\v2raydar.exe --config configs.yaml
```

## Config Fields

`bind`: HTTP bind address for the local endpoint.

`top_n`: how many best reachable configs are exposed to clients.

`refresh_seconds`: how often subscriptions are fetched and tested again. Use `0` to disable automatic refresh.

`encoded_subscription`: when `true`, `/subscription` returns a base64-encoded newline list, which is the safest default for V2Ray clients.

`fetch_timeout_ms`: timeout for each subscription fetch.

`fetch_concurrency`: how many subscriptions in the same priority group can be fetched at once.

`probe.connect_timeout_ms`: TCP connection timeout per config.

`probe.concurrency`: how many configs can be tested at once.

`subscriptions[].priority`: lower number means higher priority. Priority `1` is tested before priority `2`.

Config file extensions:

- `.yaml`
- `.yml`
- `.json`

Supported subscription source URLs:

- `https://...`
- `http://...`
- `file://...`
- local file paths
- `data:` URLs

## Local Endpoints

When the app is running with the default config:

- `http://127.0.0.1:14127/subscription` returns the top N reachable configs, base64 encoded by default.
- `http://127.0.0.1:14127/subscription.txt` returns the same configs as raw newline-separated share links.
- `http://127.0.0.1:14127/results` returns JSON diagnostics with all ranked configs.
- `http://127.0.0.1:14127/health` returns `ok`.

## Using With v2rayN on Windows

If v2rayN runs on the same Windows machine as V2RayDAR, keep:

```yaml
bind: 127.0.0.1:14127
```

Start V2RayDAR:

```powershell
target\release\v2raydar.exe --config configs.yaml
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

If v2rayN is on another Windows machine in the same LAN, bind V2RayDAR to a LAN-reachable address and use the server PC's LAN IP:

```yaml
bind: 0.0.0.0:14127
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
2. In `configs.yaml`, bind V2RayDAR to a LAN-reachable address:

```yaml
bind: 0.0.0.0:14127
```

3. Start V2RayDAR on the PC:

```bash
cargo run -- --config configs.yaml
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

The default `bind` is `127.0.0.1:14127`, which is only reachable from the same machine.

If you use `0.0.0.0:14127`, anyone on the reachable network may be able to fetch your top configs. Use a trusted LAN, firewall rules, or a more specific bind address when possible.

Do not expose the endpoint to the public internet.

## Troubleshooting

No configs are imported by the client:

- Open `/subscription.txt` in a browser and confirm it contains share links.
- Keep `encoded_subscription: true` for `/subscription`.
- Try `/subscription.txt` only if the client explicitly supports raw URL-line subscriptions.

Android cannot reach V2RayDAR:

- Do not use `127.0.0.1` from Android.
- Use `bind: 0.0.0.0:14127`.
- Use `http://PC_LAN_IP:14127/subscription`.
- Check firewall rules.

All configs are unreachable:

- Increase `probe.connect_timeout_ms`.
- Confirm the original subscriptions still contain valid links.
- Confirm the PC running V2RayDAR can reach the proxy server addresses.
- Remember that Phase 1 checks server TCP reachability, not full proxy login/protocol validity.

The endpoint is empty:

- V2RayDAR only exposes reachable configs.
- Check `/results` to see failed candidates and errors.
- Check the terminal summary after each refresh.

## References

- v2rayNG official repository: https://github.com/2dust/v2rayNG
- v2rayN official repository: https://github.com/2dust/v2rayN
- V2Fly subscription service documentation: https://www.v2fly.org/en_US/v5/config/service/subscription.html
- Xray outbound configuration documentation: https://xtls.github.io/en/config/outbounds/
