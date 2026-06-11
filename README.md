<p align="center">
  <img src="assets/V2RayDAR_logo_v1.png" alt="V2RayDAR logo" width="100" height="100">
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


## One-line setup (idempotent, OS-aware, latest V2RayDAR + sing-box v1.13.13)

Copy and paste the script for your OS into the terminal to download everything needed and run it.

<details>
<summary>Windows PowerShell</summary>

```powershell
$ErrorActionPreference='Stop'; $hdr=@{'User-Agent'='Mozilla/5.0'}; $root=Join-Path $env:LOCALAPPDATA 'V2RayDAR'; New-Item -ItemType Directory -Force $root|Out-Null; $arch=([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLower()); $arch=if($arch -match 'arm64'){'arm64'}elseif($arch -match 'x64|amd64'){'amd64'}else{'386'}; $sbRel=Invoke-RestMethod -Headers $hdr 'https://api.github.com/repos/SagerNet/sing-box/releases/tags/v1.13.13'; $sbAsset=$sbRel.assets|Where-Object{$_.name -match '(?i)windows' -and $_.name -match $arch -and $_.name -match '(\.zip|\.exe)$'}|Select-Object -First 1; $sbRoot=Join-Path $root ('sing-box\' + $sbRel.tag_name.TrimStart('v')); $sbFile=Join-Path $sbRoot $sbAsset.name; if(!(Test-Path $sbFile)){New-Item -ItemType Directory -Force $sbRoot|Out-Null; Invoke-WebRequest $sbAsset.browser_download_url -OutFile $sbFile}; if($sbFile -match '\.zip$' -and !(Get-ChildItem $sbRoot -Recurse -Filter 'sing-box.exe' -ErrorAction SilentlyContinue)){Expand-Archive $sbFile -DestinationPath $sbRoot -Force}; $sbExe=(Get-ChildItem $sbRoot -Recurse -Filter 'sing-box.exe' | Select-Object -First 1).FullName; $vrRel=Invoke-RestMethod -Headers $hdr 'https://api.github.com/repos/411A/V2RayDAR/releases/latest'; $vrAsset=$vrRel.assets|Where-Object{$_.name -match '(?i)windows' -and $_.name -match $arch -and $_.name -match '(\.zip|\.exe)$'}|Select-Object -First 1; $vrRoot=Join-Path $root ('v2raydar\' + $vrRel.tag_name.TrimStart('v')); $vrFile=Join-Path $vrRoot $vrAsset.name; if(!(Test-Path $vrFile)){New-Item -ItemType Directory -Force $vrRoot|Out-Null; Invoke-WebRequest $vrAsset.browser_download_url -OutFile $vrFile}; if($vrFile -match '\.zip$' -and !(Get-ChildItem $vrRoot -Recurse -Filter '*.exe' -ErrorAction SilentlyContinue)){Expand-Archive $vrFile -DestinationPath $vrRoot -Force}; $vrRun=(Get-ChildItem $vrRoot -Recurse -Filter '*.exe' | Select-Object -First 1).FullName; Write-Host "sing-box=$sbExe"; Write-Host "v2raydar=$vrRun"; Write-Host "run: & `"$vrRun`" --no-tui"
```

</details>

<details>
<summary>Linux</summary>

```bash
python3 -c "import json,os,platform,stat,tarfile,urllib.request,zipfile; home=os.path.expanduser('~'); root=os.path.join(os.environ.get('XDG_DATA_HOME',os.path.join(home,'.local','share')),'V2RayDAR'); os.makedirs(root,exist_ok=True); m=platform.machine().lower(); arch='amd64' if m in ('x86_64','amd64') else 'arm64' if m in ('aarch64','arm64') else 'armv7' if m.startswith(('armv7','armv8l','armv8')) else '386' if m in ('i386','i686') else m; j=lambda u: json.load(urllib.request.urlopen(urllib.request.Request(u,headers={'User-Agent':'Mozilla/5.0'}))); dl=lambda u,p: (os.path.exists(p) or urllib.request.urlretrieve(u,p)); sb=j('https://api.github.com/repos/SagerNet/sing-box/releases/tags/v1.13.13'); sb_a=next(x for x in sb['assets'] if 'linux' in x['name'].lower() and arch in x['name'].lower()); sb_dir=os.path.join(root,'sing-box',sb['tag_name'].lstrip('v')); os.makedirs(sb_dir,exist_ok=True); sb_file=os.path.join(sb_dir,sb_a['name']); dl(sb_a['browser_download_url'],sb_file); (os.path.exists(os.path.join(sb_dir,'sing-box')) or (tarfile.open(sb_file,'r:gz').extractall(sb_dir) if sb_file.endswith(('.tar.gz','.tgz')) else zipfile.ZipFile(sb_file).extractall(sb_dir))); sb_bin=next(os.path.join(dp,f) for dp,_,fs in os.walk(sb_dir) for f in fs if f=='sing-box'); vr=j('https://api.github.com/repos/411A/V2RayDAR/releases/latest'); vr_a=next(x for x in vr['assets'] if 'linux' in x['name'].lower() and arch in x['name'].lower()); vr_dir=os.path.join(root,'v2raydar',vr['tag_name'].lstrip('v')); os.makedirs(vr_dir,exist_ok=True); vr_file=os.path.join(vr_dir,vr_a['name']); dl(vr_a['browser_download_url'],vr_file); vr_bin=vr_file; (vr_file.endswith(('.zip','.tar.gz','.tgz')) and (tarfile.open(vr_file,'r:gz').extractall(vr_dir) if vr_file.endswith(('.tar.gz','.tgz')) else zipfile.ZipFile(vr_file).extractall(vr_dir))); vr_bin=next((os.path.join(dp,f) for dp,_,fs in os.walk(vr_dir) for f in fs if 'v2raydar' in f.lower() and os.access(os.path.join(dp,f),os.X_OK)),vr_file); os.chmod(vr_bin,os.stat(vr_bin).st_mode|stat.S_IEXEC); print('sing-box='+sb_bin); print('v2raydar='+vr_bin); print('run='+vr_bin+' --no-tui')"
```

</details>

<details>
<summary>macOS</summary>

```bash
python3 -c "import json,os,platform,stat,tarfile,urllib.request,zipfile; home=os.path.expanduser('~'); root=os.path.join(home,'Library','Application Support','V2RayDAR'); os.makedirs(root,exist_ok=True); m=platform.machine().lower(); arch='arm64' if m in ('aarch64','arm64') else 'amd64' if m in ('x86_64','amd64') else '386' if m in ('i386','i686') else m; j=lambda u: json.load(urllib.request.urlopen(urllib.request.Request(u,headers={'User-Agent':'Mozilla/5.0'}))); dl=lambda u,p: (os.path.exists(p) or urllib.request.urlretrieve(u,p)); sb=j('https://api.github.com/repos/SagerNet/sing-box/releases/tags/v1.13.13'); sb_a=next(x for x in sb['assets'] if ('darwin' in x['name'].lower() or 'macos' in x['name'].lower()) and (arch in x['name'].lower() or 'universal' in x['name'].lower())); sb_dir=os.path.join(root,'sing-box',sb['tag_name'].lstrip('v')); os.makedirs(sb_dir,exist_ok=True); sb_file=os.path.join(sb_dir,sb_a['name']); dl(sb_a['browser_download_url'],sb_file); (os.path.exists(os.path.join(sb_dir,'sing-box')) or (tarfile.open(sb_file,'r:gz').extractall(sb_dir) if sb_file.endswith(('.tar.gz','.tgz')) else zipfile.ZipFile(sb_file).extractall(sb_dir))); sb_bin=next(os.path.join(dp,f) for dp,_,fs in os.walk(sb_dir) for f in fs if f=='sing-box'); vr=j('https://api.github.com/repos/411A/V2RayDAR/releases/latest'); vr_a=next(x for x in vr['assets'] if ('macos' in x['name'].lower() or 'darwin' in x['name'].lower()) and (arch in x['name'].lower() or 'universal' in x['name'].lower())); vr_dir=os.path.join(root,'v2raydar',vr['tag_name'].lstrip('v')); os.makedirs(vr_dir,exist_ok=True); vr_file=os.path.join(vr_dir,vr_a['name']); dl(vr_a['browser_download_url'],vr_file); vr_bin=vr_file; (vr_file.endswith(('.zip','.tar.gz','.tgz')) and (tarfile.open(vr_file,'r:gz').extractall(vr_dir) if vr_file.endswith(('.tar.gz','.tgz')) else zipfile.ZipFile(vr_file).extractall(vr_dir))); app=next((os.path.join(dp,d) for dp,ds,_ in os.walk(vr_dir) for d in ds if d.endswith('.app')), None); vr_bin=app or next((os.path.join(dp,f) for dp,_,fs in os.walk(vr_dir) for f in fs if 'v2raydar' in f.lower() and os.access(os.path.join(dp,f),os.X_OK)),vr_file); os.chmod(vr_bin,os.stat(vr_bin).st_mode|stat.S_IEXEC) if os.path.isfile(vr_bin) else None; print('sing-box='+sb_bin); print('v2raydar='+vr_bin); print('run='+('open "'+vr_bin+'"' if vr_bin.endswith('.app') else vr_bin+' --no-tui'))"
```

</details>

<details>
<summary>Android / Termux</summary>

```bash
python3 -c "import json,os,platform,stat,tarfile,urllib.request,zipfile; home=os.path.expanduser('~'); root=os.path.join(home,'.local','share','V2RayDAR'); os.makedirs(root,exist_ok=True); m=platform.machine().lower(); arch='arm64-v8a' if m in ('aarch64','arm64') else 'armeabi-v7a' if m.startswith('armv7') else 'x86_64' if m in ('x86_64','amd64') else 'x86' if m in ('i386','i686') else m; j=lambda u: json.load(urllib.request.urlopen(urllib.request.Request(u,headers={'User-Agent':'Mozilla/5.0'}))); dl=lambda u,p: (os.path.exists(p) or urllib.request.urlretrieve(u,p)); sb=j('https://api.github.com/repos/SagerNet/sing-box/releases/tags/v1.13.13'); sb_a=next((x for x in sb['assets'] if ('android' in x['name'].lower() or 'termux' in x['name'].lower()) and arch in x['name'].lower() and not x['name'].lower().endswith('.apk')), None); sb_dir=os.path.join(root,'sing-box',sb['tag_name'].lstrip('v')); os.makedirs(sb_dir,exist_ok=True); sb_bin=''; (sb_a and (sb_file:=os.path.join(sb_dir,sb_a['name'])) and dl(sb_a['browser_download_url'],sb_file) and (not sb_file.endswith(('.apk','.zip','.tar.gz','.tgz')) or (tarfile.open(sb_file,'r:gz').extractall(sb_dir) if sb_file.endswith(('.tar.gz','.tgz')) else zipfile.ZipFile(sb_file).extractall(sb_dir))) and (sb_bin:=next((os.path.join(dp,f) for dp,_,fs in os.walk(sb_dir) for f in fs if f=='sing-box'), sb_file))); vr=j('https://api.github.com/repos/411A/V2RayDAR/releases/latest'); vr_a=next((x for x in vr['assets'] if ('android' in x['name'].lower() or 'termux' in x['name'].lower()) and arch in x['name'].lower()), None); vr_dir=os.path.join(root,'v2raydar',vr['tag_name'].lstrip('v')); os.makedirs(vr_dir,exist_ok=True); vr_src=''; (vr_a and (vr_file:=os.path.join(vr_dir,vr_a['name'])) and dl(vr_a['browser_download_url'],vr_file) and (not vr_file.endswith(('.apk','.zip','.tar.gz','.tgz')) or (tarfile.open(vr_file,'r:gz').extractall(vr_dir) if vr_file.endswith(('.tar.gz','.tgz')) else zipfile.ZipFile(vr_file).extractall(vr_dir))) and (vr_src:=next((os.path.join(dp,f) for dp,_,fs in os.walk(vr_dir) for f in fs if 'v2raydar' in f.lower() and os.access(os.path.join(dp,f),os.X_OK)), vr_file))); src_tar=os.path.join(vr_dir,vr['tag_name'].lstrip('v')+'.tar.gz'); (not vr_a) and dl('https://github.com/411A/V2RayDAR/archive/refs/tags/'+vr['tag_name'].lstrip('v')+'.tar.gz',src_tar) and tarfile.open(src_tar,'r:gz').extractall(vr_dir) or None; src=next((os.path.join(vr_dir,n) for n in os.listdir(vr_dir) if n.startswith('V2RayDAR-')),vr_dir); print('sing-box='+sb_bin if sb_bin else 'sing-box=not-found'); print('v2raydar='+ (vr_src or src)); print('run: cd "'+(vr_src or src)+'" && cargo run --release -- --no-tui' if not vr_a else 'run: '+(vr_src or src)+' --no-tui')"
```

</details>

Each block reuses the already-downloaded file or extracted folder when it is already present, then prints the resolved sing-box and V2RayDAR paths and a run command.

---

## Why V2RayDAR

- Pulls subscriptions in parallel from any number of sources you list.
- Parses raw, base64, JSON, and YAML feeds — and `vmess`, `vless`, `trojan`, `ss`, `ssr`, `hysteria2`, `hy2`, `tuic` share-links.
- Validates each candidate through your current network with `sing-box` (it actually loads a test URL through the proxy).
- Re-exposes the top working configs at a local URL so any compatible client just sees one always-fresh subscription.
- Survives restricted networks via a snapshot cache, an in-network bridge config, or an `emergency_config`.
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

🖥️ Windows TUI Preview:

<p align="center">
  <img src="assets/Windows_TUI_v0.2.3.png" alt="Windows TUI" width="100%">
</p>

## Quick start

1. **Get sing-box**. Active probing needs the [sing-box](https://github.com/SagerNet/sing-box/releases/latest) binary for your OS. Use `sing-box.exe` on Windows and `sing-box` on Linux, Termux, and macOS.
2. **Run V2RayDAR**. Use the release binary for your OS, or build from source with `cargo run --release`.
3. **First launch** creates `configs.yaml` in the platform's app-data folder and (if `probe.mode: active`) asks for the full path to `sing-box`.
4. **Point your client** at one of the local URLs below.

### Local URLs (default `127.0.0.1:27141`)

| Endpoint | Use for |
| --- | --- |
| `http://127.0.0.1:27141/subscription` | base64 subscription feed — what v2rayN / v2rayNG expect |
| `http://127.0.0.1:27141/subscription.txt` | the same, but plain newline-separated share-links |
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
| `scan_all_configs` | `false` | When `true`, validates every loaded config instead of stopping after enough have been confirmed. |
| `fetch_timeout_ms` | `30000` | Per-source fetch timeout. |
| `fetch_concurrency` | `8` | Subscription sources fetched in parallel. |
| `max_subscription_bytes` | `33554432` | Size cap per fetched subscription source (32 MiB). |
| `use_cache_only` | `false` | When `true`, skip fresh fetches and test only cached HTTP snapshots — useful on heavily restricted networks. |
| `emergency_config` | `null` | Optional working share-link used through `sing-box` as a bridge when HTTP subscription fetches fail. |
| `sharing.enabled` | `false` | Lets LAN clients read the endpoints. |
| `sharing.require_token` | `false` | Requires `?token=...` for LAN requests. |
| `sharing.token` | `null` | Leave empty, set `true` to auto-generate, or supply a string. |
| `probe.mode` | `active` | `active` uses `sing-box`; `tcp` is diagnostic only. |
| `probe.sing_box_path` | `null` | Full path to the `sing-box` executable. |
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
| `subscriptions` | _(two demo entries)_ | List of `{ name, url, enabled, priority }` sources. |

</details>

## Notes for restricted networks

- If you are on a very restricted network, it is recommended to never delete the cache so the app can test cached HTTP subscription snapshots.
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
