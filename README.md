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


## Copy-paste setup (latest V2RayDAR + recommended sing-box)

Copy and paste the script for your OS into the terminal. These scripts fetch the latest V2RayDAR release, read its embedded `SING_BOX_VERSION` from `src/constants.rs`, install or download that recommended `sing-box` release by default, and write `probe.sing_box_path` into `configs.yaml` so the printed `--no-tui` command can start without the interactive setup prompt. Set `V2RAYDAR_SING_BOX_VERSION` before running a script only if you intentionally want a different default `sing-box` version, or set `V2RAYDAR_SING_BOX_PATH` to reuse a specific working `sing-box` binary.

<details>
<summary>Windows PowerShell</summary>

```powershell
$ErrorActionPreference = 'Stop'
$Headers = @{ 'User-Agent' = 'Mozilla/5.0' }
$Base = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { Join-Path $HOME 'AppData\Local' }
$Root = Join-Path $Base 'V2RayDAR'
New-Item -ItemType Directory -Force $Root | Out-Null

function Get-EmbeddedSingBoxVersion([string]$V2RayDARTag) {
    if ($env:V2RAYDAR_SING_BOX_VERSION) {
        return $env:V2RAYDAR_SING_BOX_VERSION.Trim()
    }
    $constantsUrl = "https://raw.githubusercontent.com/411A/V2RayDAR/$V2RayDARTag/src/constants.rs"
    $constants = (Invoke-WebRequest -Headers $Headers -Uri $constantsUrl).Content
    $match = [regex]::Match($constants, 'pub\s+const\s+SING_BOX_VERSION\s*:\s*&str\s*=\s*"([^"]+)"')
    if (!$match.Success) {
        throw "Unable to read SING_BOX_VERSION from $constantsUrl"
    }
    return $match.Groups[1].Value
}

function Test-NameToken([string]$Name, [string]$Token) {
    return $Name -match ('(^|[-_.])' + [regex]::Escape($Token) + '($|[-_.])')
}

function Select-ReleaseAsset($Release, [string]$Platform, [string[]]$ArchTokens, [string[]]$Suffixes) {
    foreach ($arch in $ArchTokens) {
        foreach ($suffix in $Suffixes) {
            foreach ($asset in $Release.assets) {
                $name = $asset.name.ToLowerInvariant()
                if ($name -match '\.(sha256|sig|asc|txt)$') { continue }
                if ($name -notmatch $Platform) { continue }
                if (-not (Test-NameToken $name $arch)) { continue }
                if ($suffix -and -not $name.EndsWith($suffix)) { continue }
                return $asset
            }
        }
    }
    throw "No $Platform asset found for architecture token(s): $($ArchTokens -join ', ')"
}

function Save-ReleaseAsset($Asset, [string]$Directory) {
    New-Item -ItemType Directory -Force $Directory | Out-Null
    $file = Join-Path $Directory $Asset.name
    if (!(Test-Path $file) -or (Get-Item $file).Length -lt 1024) {
        Invoke-WebRequest -Headers $Headers -Uri $Asset.browser_download_url -OutFile $file
    }
    return $file
}

function Test-SingBoxExecutable([string]$SingBoxPath) {
    if ([string]::IsNullOrWhiteSpace($SingBoxPath)) {
        return $false
    }
    try {
        & $SingBoxPath version *> $null
        return $LASTEXITCODE -eq 0
    } catch {
        return $false
    }
}

function Get-ConfiguredSingBoxPath {
    if (!$env:V2RAYDAR_SING_BOX_PATH) {
        return $null
    }
    $configured = $env:V2RAYDAR_SING_BOX_PATH.Trim()
    if (!(Test-SingBoxExecutable $configured)) {
        throw "V2RAYDAR_SING_BOX_PATH does not run as a sing-box executable: $configured"
    }
    return $configured
}

function Set-SingBoxPathInConfig([string]$ConfigPath, [string]$SingBoxPath, [string]$V2RayDARTag) {
    New-Item -ItemType Directory -Force (Split-Path -Parent $ConfigPath) | Out-Null
    if (!(Test-Path $ConfigPath)) {
        $template = "https://raw.githubusercontent.com/411A/V2RayDAR/$V2RayDARTag/configs.example.yaml"
        Invoke-WebRequest -Headers $Headers -Uri $template -OutFile $ConfigPath
    }
    $yamlValue = "'" + ($SingBoxPath -replace "'", "''") + "'"
    $text = Get-Content -Raw $ConfigPath
    if ($text -match '(?m)^\s*sing_box_path\s*:') {
        $text = [regex]::Replace($text, '(?m)^(\s*sing_box_path\s*:\s*).*$', ('$1' + $yamlValue), 1)
    } elseif ($text -match '(?m)^probe\s*:\s*$') {
        $text = [regex]::Replace($text, '(?m)^(probe\s*:\s*)$', ('$1' + "`r`n  sing_box_path: $yamlValue"), 1)
    } else {
        $text += "`r`nprobe:`r`n  mode: active`r`n  sing_box_path: $yamlValue`r`n"
    }
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($ConfigPath, $text, $utf8NoBom)
}

$osArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
$singBoxArch = if ($osArch -match 'arm64') { 'arm64' } elseif ($osArch -match 'x64|amd64') { 'amd64' } else { '386' }
$v2raydarArch = if ($osArch -match 'arm64') { @('arm64', 'x86_64') } elseif ($osArch -match 'x64|amd64') { @('x86_64', 'amd64') } else { @('386', 'x86') }

$vrRel = Invoke-RestMethod -Headers $Headers 'https://api.github.com/repos/411A/V2RayDAR/releases/latest'
$SingBoxVersion = Get-EmbeddedSingBoxVersion $vrRel.tag_name
$sbExe = Get-ConfiguredSingBoxPath
if (!$sbExe) {
    $sbRel = Invoke-RestMethod -Headers $Headers "https://api.github.com/repos/SagerNet/sing-box/releases/tags/v$SingBoxVersion"
    $sbAsset = Select-ReleaseAsset $sbRel 'windows' @($singBoxArch) @('.zip', '.exe')
    $sbRoot = Join-Path $Root "sing-box\$SingBoxVersion"
    $sbFile = Save-ReleaseAsset $sbAsset $sbRoot
    if ($sbFile.ToLowerInvariant().EndsWith('.zip') -and !(Get-ChildItem $sbRoot -Recurse -Filter 'sing-box.exe' -ErrorAction SilentlyContinue)) {
        Expand-Archive $sbFile -DestinationPath $sbRoot -Force
    }
    $sbExe = if ($sbFile.ToLowerInvariant().EndsWith('.exe')) { $sbFile } else { (Get-ChildItem $sbRoot -Recurse -Filter 'sing-box.exe' | Select-Object -First 1).FullName }
    if (!$sbExe) { throw 'sing-box.exe was not found after download/extract.' }
}

$vrAsset = Select-ReleaseAsset $vrRel 'windows' $v2raydarArch @('.exe', '.zip')
$vrRoot = Join-Path $Root "v2raydar\$($vrRel.tag_name.TrimStart('v'))"
$vrFile = Save-ReleaseAsset $vrAsset $vrRoot
if ($vrFile.ToLowerInvariant().EndsWith('.zip') -and !(Get-ChildItem $vrRoot -Recurse -Filter 'v2raydar*.exe' -ErrorAction SilentlyContinue)) {
    Expand-Archive $vrFile -DestinationPath $vrRoot -Force
}
$vrRun = if ($vrFile.ToLowerInvariant().EndsWith('.exe')) { $vrFile } else { (Get-ChildItem $vrRoot -Recurse -Filter 'v2raydar*.exe' | Select-Object -First 1).FullName }
if (!$vrRun) { throw 'V2RayDAR executable was not found after download/extract.' }

$config = Join-Path $Root 'v2raydar_data\configs.yaml'
Set-SingBoxPathInConfig $config $sbExe $vrRel.tag_name
Write-Host "sing-box=$sbExe"
Write-Host "v2raydar=$vrRun"
Write-Host "config=$config"
Write-Host "run: & `"$vrRun`" --no-tui"
```

</details>

<details>
<summary>Linux</summary>

```bash
python3 << 'EOF'
import json, os, platform, re, shlex, stat, subprocess, tarfile, urllib.request, zipfile

home = os.path.expanduser('~')
root = os.path.join(os.environ.get('XDG_DATA_HOME', os.path.join(home, '.local', 'share')), 'V2RayDAR')
os.makedirs(root, exist_ok=True)

def j(url):
    return json.load(urllib.request.urlopen(urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})))

def dl(url, path):
    if os.path.exists(path) and os.path.getsize(path) > 1024:
        return path
    urllib.request.urlretrieve(url, path)
    return path

def token(name, value):
    return re.search(r'(^|[-_.])' + re.escape(value) + r'($|[-_.])', name) is not None

def embedded_sing_box_version(tag):
    override = os.environ.get('V2RAYDAR_SING_BOX_VERSION', '').strip()
    if override:
        return override
    constants_url = 'https://raw.githubusercontent.com/411A/V2RayDAR/' + tag + '/src/constants.rs'
    data = urllib.request.urlopen(urllib.request.Request(constants_url, headers={'User-Agent': 'Mozilla/5.0'})).read().decode('utf-8')
    match = re.search(r'pub\s+const\s+SING_BOX_VERSION\s*:\s*&str\s*=\s*"([^"]+)"', data)
    if not match:
        raise SystemExit('Unable to read SING_BOX_VERSION from ' + constants_url)
    return match.group(1)

def asset_for(release, platforms, arch_tokens, suffixes):
    for arch in arch_tokens:
        for suffix in suffixes:
            for asset in release.get('assets', []):
                name = asset['name'].lower()
                if name.endswith(('.sha256', '.sig', '.asc', '.txt')):
                    continue
                if not any(platform in name for platform in platforms):
                    continue
                if arch and not token(name, arch):
                    continue
                if suffix and not name.endswith(suffix):
                    continue
                return asset
    return None

def extract(path, dest):
    with open(path, 'rb') as fp:
        magic = fp.read(2)
    if magic == b'PK':
        with zipfile.ZipFile(path) as z:
            z.extractall(dest)
            for info in z.infolist():
                mode = (info.external_attr >> 16) & 0o777
                target = os.path.join(dest, info.filename)
                if mode and os.path.exists(target):
                    os.chmod(target, mode)
    else:
        with tarfile.open(path, 'r:*') as t:
            try:
                t.extractall(dest, filter='data')
            except TypeError:
                t.extractall(dest)

def chmod_x(path):
    os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)

def find_file(base, needle):
    for dp, _, files in os.walk(base):
        for filename in files:
            lower = filename.lower()
            if lower.endswith(('.sha256', '.zip', '.tar.gz', '.tgz')):
                continue
            if needle in lower:
                return os.path.join(dp, filename)
    return None

def sing_box_version(path):
    try:
        return subprocess.check_output([path, 'version'], text=True, stderr=subprocess.STDOUT, timeout=10)
    except Exception:
        return ''

def configured_sing_box_path():
    path = os.environ.get('V2RAYDAR_SING_BOX_PATH', '').strip()
    if not path:
        return None
    if not sing_box_version(path):
        raise SystemExit('V2RAYDAR_SING_BOX_PATH does not run as a sing-box executable: ' + path)
    return path

def warn_if_not_recommended(path, recommended):
    version = sing_box_version(path)
    if version and recommended not in version:
        print('Using sing-box from ' + path + '; recommended version is ' + recommended)

def yaml_sq(value):
    return "'" + value.replace("'", "''") + "'"

def configure_config(tag, sing_box_path):
    config_dir = os.path.join(root, 'v2raydar_data')
    os.makedirs(config_dir, exist_ok=True)
    config = os.path.join(config_dir, 'configs.yaml')
    if not os.path.exists(config):
        template = 'https://raw.githubusercontent.com/411A/V2RayDAR/' + tag + '/configs.example.yaml'
        dl(template, config)
    with open(config, 'r', encoding='utf-8') as fp:
        text = fp.read()
    value = yaml_sq(sing_box_path)
    if re.search(r'(?m)^\s*sing_box_path\s*:', text):
        text = re.sub(r'(?m)^(\s*sing_box_path\s*:\s*).*$', lambda m: m.group(1) + value, text, count=1)
    elif re.search(r'(?m)^probe\s*:\s*$', text):
        text = re.sub(r'(?m)^(probe\s*:\s*)$', lambda m: m.group(1) + '\n  sing_box_path: ' + value, text, count=1)
    else:
        text += '\nprobe:\n  mode: active\n  sing_box_path: ' + value + '\n'
    with open(config, 'w', encoding='utf-8') as fp:
        fp.write(text)
    return config

def source_dir(base):
    return next((os.path.join(base, name) for name in os.listdir(base) if os.path.isdir(os.path.join(base, name)) and 'v2raydar' in name.lower()), base)

m = platform.machine().lower()
sb_arch = 'amd64' if m in ('x86_64', 'amd64') else 'arm64' if m in ('aarch64', 'arm64') else 'armv7' if m.startswith(('armv7', 'armv8l', 'armv8')) else '386' if m in ('i386', 'i686') else m
vr_arch = ['x86_64', 'amd64'] if sb_arch == 'amd64' else [sb_arch]

vr = j('https://api.github.com/repos/411A/V2RayDAR/releases/latest')
SB_VERSION = embedded_sing_box_version(vr['tag_name'])
sb_bin = configured_sing_box_path()
if not sb_bin:
    sb = j('https://api.github.com/repos/SagerNet/sing-box/releases/tags/v' + SB_VERSION)
    sb_a = asset_for(sb, ['linux'], [sb_arch], ['.tar.gz', '.tgz', '.zip'])
    if not sb_a:
        raise SystemExit('No sing-box v' + SB_VERSION + ' Linux asset found for ' + sb_arch)
    sb_dir = os.path.join(root, 'sing-box', SB_VERSION)
    os.makedirs(sb_dir, exist_ok=True)
    sb_file = dl(sb_a['browser_download_url'], os.path.join(sb_dir, sb_a['name']))
    if not find_file(sb_dir, 'sing-box'):
        extract(sb_file, sb_dir)
    sb_bin = find_file(sb_dir, 'sing-box')
    if not sb_bin:
        raise SystemExit('sing-box was not found after extraction')
    chmod_x(sb_bin)
warn_if_not_recommended(sb_bin, SB_VERSION)

vr_dir = os.path.join(root, 'v2raydar', vr['tag_name'].lstrip('v'))
os.makedirs(vr_dir, exist_ok=True)
vr_a = asset_for(vr, ['linux'], vr_arch, ['', '.tar.gz', '.tgz', '.zip'])
config = configure_config(vr['tag_name'], sb_bin)

if vr_a:
    vr_file = dl(vr_a['browser_download_url'], os.path.join(vr_dir, vr_a['name']))
    if vr_file.endswith(('.zip', '.tar.gz', '.tgz')):
        extract(vr_file, vr_dir)
        vr_bin = find_file(vr_dir, 'v2raydar')
    else:
        vr_bin = vr_file
    if not vr_bin:
        raise SystemExit('V2RayDAR binary was not found after download/extract')
    chmod_x(vr_bin)
    print('sing-box=' + sb_bin)
    print('v2raydar=' + vr_bin)
    print('config=' + config)
    print('run: ' + shlex.quote(vr_bin) + ' --no-tui')
else:
    src_tgz = dl(vr['tarball_url'], os.path.join(vr_dir, 'src.tar.gz'))
    extract(src_tgz, vr_dir)
    src = source_dir(vr_dir)
    print('sing-box=' + sb_bin)
    print('v2raydar_source=' + src)
    print('config=' + config)
    print('run: cd ' + shlex.quote(src) + ' && cargo run --release -- --no-tui')
EOF
```

</details>

<details>
<summary>macOS</summary>

```bash
python3 << 'EOF'
import json, os, platform, re, shlex, stat, subprocess, tarfile, urllib.request, zipfile

home = os.path.expanduser('~')
root = os.path.join(home, 'Library', 'Application Support', 'V2RayDAR')
os.makedirs(root, exist_ok=True)

def j(url):
    return json.load(urllib.request.urlopen(urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})))

def dl(url, path):
    if os.path.exists(path) and os.path.getsize(path) > 1024:
        return path
    urllib.request.urlretrieve(url, path)
    return path

def token(name, value):
    return re.search(r'(^|[-_.])' + re.escape(value) + r'($|[-_.])', name) is not None

def embedded_sing_box_version(tag):
    override = os.environ.get('V2RAYDAR_SING_BOX_VERSION', '').strip()
    if override:
        return override
    constants_url = 'https://raw.githubusercontent.com/411A/V2RayDAR/' + tag + '/src/constants.rs'
    data = urllib.request.urlopen(urllib.request.Request(constants_url, headers={'User-Agent': 'Mozilla/5.0'})).read().decode('utf-8')
    match = re.search(r'pub\s+const\s+SING_BOX_VERSION\s*:\s*&str\s*=\s*"([^"]+)"', data)
    if not match:
        raise SystemExit('Unable to read SING_BOX_VERSION from ' + constants_url)
    return match.group(1)

def asset_for(release, platforms, arch_tokens, suffixes):
    for arch in arch_tokens:
        for suffix in suffixes:
            for asset in release.get('assets', []):
                name = asset['name'].lower()
                if name.endswith(('.sha256', '.sig', '.asc', '.txt')):
                    continue
                if not any(platform in name for platform in platforms):
                    continue
                if arch and not token(name, arch):
                    continue
                if suffix and not name.endswith(suffix):
                    continue
                return asset
    return None

def extract(path, dest):
    with open(path, 'rb') as fp:
        magic = fp.read(2)
    if magic == b'PK':
        with zipfile.ZipFile(path) as z:
            z.extractall(dest)
            for info in z.infolist():
                mode = (info.external_attr >> 16) & 0o777
                target = os.path.join(dest, info.filename)
                if mode and os.path.exists(target):
                    os.chmod(target, mode)
    else:
        with tarfile.open(path, 'r:*') as t:
            try:
                t.extractall(dest, filter='data')
            except TypeError:
                t.extractall(dest)

def chmod_x(path):
    os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)

def find_file(base, needle):
    for dp, _, files in os.walk(base):
        for filename in files:
            lower = filename.lower()
            if lower.endswith(('.sha256', '.zip', '.tar.gz', '.tgz')):
                continue
            if needle in lower:
                return os.path.join(dp, filename)
    return None

def sing_box_version(path):
    try:
        return subprocess.check_output([path, 'version'], text=True, stderr=subprocess.STDOUT, timeout=10)
    except Exception:
        return ''

def configured_sing_box_path():
    path = os.environ.get('V2RAYDAR_SING_BOX_PATH', '').strip()
    if not path:
        return None
    if not sing_box_version(path):
        raise SystemExit('V2RAYDAR_SING_BOX_PATH does not run as a sing-box executable: ' + path)
    return path

def warn_if_not_recommended(path, recommended):
    version = sing_box_version(path)
    if version and recommended not in version:
        print('Using sing-box from ' + path + '; recommended version is ' + recommended)

def find_app(base):
    for dp, dirs, _ in os.walk(base):
        for dirname in dirs:
            if dirname.endswith('.app'):
                return os.path.join(dp, dirname)
    return None

def yaml_sq(value):
    return "'" + value.replace("'", "''") + "'"

def configure_config(tag, sing_box_path):
    config_dir = os.path.join(root, 'v2raydar_data')
    os.makedirs(config_dir, exist_ok=True)
    config = os.path.join(config_dir, 'configs.yaml')
    if not os.path.exists(config):
        template = 'https://raw.githubusercontent.com/411A/V2RayDAR/' + tag + '/configs.example.yaml'
        dl(template, config)
    with open(config, 'r', encoding='utf-8') as fp:
        text = fp.read()
    value = yaml_sq(sing_box_path)
    if re.search(r'(?m)^\s*sing_box_path\s*:', text):
        text = re.sub(r'(?m)^(\s*sing_box_path\s*:\s*).*$', lambda m: m.group(1) + value, text, count=1)
    elif re.search(r'(?m)^probe\s*:\s*$', text):
        text = re.sub(r'(?m)^(probe\s*:\s*)$', lambda m: m.group(1) + '\n  sing_box_path: ' + value, text, count=1)
    else:
        text += '\nprobe:\n  mode: active\n  sing_box_path: ' + value + '\n'
    with open(config, 'w', encoding='utf-8') as fp:
        fp.write(text)
    return config

def source_dir(base):
    return next((os.path.join(base, name) for name in os.listdir(base) if os.path.isdir(os.path.join(base, name)) and 'v2raydar' in name.lower()), base)

m = platform.machine().lower()
arch = 'arm64' if m in ('aarch64', 'arm64') else 'amd64' if m in ('x86_64', 'amd64') else '386'
vr_arch = ['universal', arch, 'x86_64' if arch == 'amd64' else arch]

vr = j('https://api.github.com/repos/411A/V2RayDAR/releases/latest')
SB_VERSION = embedded_sing_box_version(vr['tag_name'])
sb_bin = configured_sing_box_path()
if not sb_bin:
    sb = j('https://api.github.com/repos/SagerNet/sing-box/releases/tags/v' + SB_VERSION)
    sb_a = asset_for(sb, ['darwin', 'macos'], [arch], ['.tar.gz', '.tgz', '.zip'])
    if not sb_a:
        raise SystemExit('No sing-box v' + SB_VERSION + ' macOS asset found for ' + arch)
    sb_dir = os.path.join(root, 'sing-box', SB_VERSION)
    os.makedirs(sb_dir, exist_ok=True)
    sb_file = dl(sb_a['browser_download_url'], os.path.join(sb_dir, sb_a['name']))
    if not find_file(sb_dir, 'sing-box'):
        extract(sb_file, sb_dir)
    sb_bin = find_file(sb_dir, 'sing-box')
    if not sb_bin:
        raise SystemExit('sing-box was not found after extraction')
    chmod_x(sb_bin)
warn_if_not_recommended(sb_bin, SB_VERSION)

vr_dir = os.path.join(root, 'v2raydar', vr['tag_name'].lstrip('v'))
os.makedirs(vr_dir, exist_ok=True)
vr_a = asset_for(vr, ['macos', 'darwin'], vr_arch, ['.zip', '.tar.gz', '.tgz', ''])
config = configure_config(vr['tag_name'], sb_bin)

if vr_a:
    vr_file = dl(vr_a['browser_download_url'], os.path.join(vr_dir, vr_a['name']))
    if vr_file.endswith(('.zip', '.tar.gz', '.tgz')):
        extract(vr_file, vr_dir)
    app = find_app(vr_dir)
    if app:
        run = os.path.join(app, 'Contents', 'MacOS', 'V2RayDAR')
        if not os.path.exists(run):
            run = find_file(app, 'v2raydar')
        if not run:
            raise SystemExit('V2RayDAR app launcher was not found after extraction')
        chmod_x(run)
        print('sing-box=' + sb_bin)
        print('v2raydar=' + app)
        print('config=' + config)
        print('run: ' + shlex.quote(run) + ' --no-tui')
    else:
        run = find_file(vr_dir, 'v2raydar') or vr_file
        chmod_x(run)
        print('sing-box=' + sb_bin)
        print('v2raydar=' + run)
        print('config=' + config)
        print('run: ' + shlex.quote(run) + ' --no-tui')
else:
    src_tgz = dl(vr['tarball_url'], os.path.join(vr_dir, 'src.tar.gz'))
    extract(src_tgz, vr_dir)
    src = source_dir(vr_dir)
    print('sing-box=' + sb_bin)
    print('v2raydar_source=' + src)
    print('config=' + config)
    print('run: cd ' + shlex.quote(src) + ' && cargo run --release -- --no-tui')
EOF
```

</details>

<details>
<summary>Android / Termux</summary>

Termux currently has no Android V2RayDAR release asset, so install the build toolchain and build the latest source locally. In Termux, the `rust` package provides Cargo; confirm `cargo --version` works before building.

```bash
pkg update
pkg upgrade -y
pkg install -y python rust git clang make cmake pkg-config
cargo --version

python3 << 'EOF'
import json, os, platform, re, shlex, stat, subprocess, tarfile, urllib.request, zipfile

home = os.path.expanduser('~')
root = os.path.join(home, '.local', 'share', 'V2RayDAR')
os.makedirs(root, exist_ok=True)

def j(url):
    return json.load(urllib.request.urlopen(urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})))

def dl(url, path):
    if os.path.exists(path) and os.path.getsize(path) > 1024:
        return path
    urllib.request.urlretrieve(url, path)
    return path

def token(name, value):
    return re.search(r'(^|[-_.])' + re.escape(value) + r'($|[-_.])', name) is not None

def embedded_sing_box_version(tag):
    override = os.environ.get('V2RAYDAR_SING_BOX_VERSION', '').strip()
    if override:
        return override
    constants_url = 'https://raw.githubusercontent.com/411A/V2RayDAR/' + tag + '/src/constants.rs'
    data = urllib.request.urlopen(urllib.request.Request(constants_url, headers={'User-Agent': 'Mozilla/5.0'})).read().decode('utf-8')
    match = re.search(r'pub\s+const\s+SING_BOX_VERSION\s*:\s*&str\s*=\s*"([^"]+)"', data)
    if not match:
        raise SystemExit('Unable to read SING_BOX_VERSION from ' + constants_url)
    return match.group(1)

def asset_for(release, platforms, arch_tokens, suffixes):
    for arch in arch_tokens:
        for suffix in suffixes:
            for asset in release.get('assets', []):
                name = asset['name'].lower()
                if name.endswith(('.apk', '.sha256', '.sig', '.asc', '.txt')):
                    continue
                if not any(platform in name for platform in platforms):
                    continue
                if arch and not token(name, arch):
                    continue
                if suffix and not name.endswith(suffix):
                    continue
                return asset
    return None

def extract(path, dest):
    with open(path, 'rb') as fp:
        magic = fp.read(2)
    if magic == b'PK':
        with zipfile.ZipFile(path) as z:
            z.extractall(dest)
            for info in z.infolist():
                mode = (info.external_attr >> 16) & 0o777
                target = os.path.join(dest, info.filename)
                if mode and os.path.exists(target):
                    os.chmod(target, mode)
    else:
        with tarfile.open(path, 'r:*') as t:
            try:
                t.extractall(dest, filter='data')
            except TypeError:
                t.extractall(dest)

def chmod_x(path):
    os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)

def find_file(base, needle):
    for dp, _, files in os.walk(base):
        for filename in files:
            lower = filename.lower()
            if lower.endswith(('.sha256', '.zip', '.tar.gz', '.tgz')):
                continue
            if needle in lower:
                return os.path.join(dp, filename)
    return None

def sing_box_version(path):
    try:
        out = subprocess.check_output([path, 'version'], text=True, stderr=subprocess.STDOUT, timeout=10)
    except Exception:
        return ''
    return out

def configured_sing_box_path():
    path = os.environ.get('V2RAYDAR_SING_BOX_PATH', '').strip()
    if not path:
        return None
    if not sing_box_version(path):
        raise SystemExit('V2RAYDAR_SING_BOX_PATH does not run as a sing-box executable: ' + path)
    return path

def warn_if_not_recommended(path, recommended):
    version = sing_box_version(path)
    if version and recommended not in version:
        print('Using sing-box from ' + path + '; recommended version is ' + recommended)

def yaml_sq(value):
    return "'" + value.replace("'", "''") + "'"

def configure_config(tag, sing_box_path):
    config_dir = os.path.join(root, 'v2raydar_data')
    os.makedirs(config_dir, exist_ok=True)
    config = os.path.join(config_dir, 'configs.yaml')
    if not os.path.exists(config):
        template = 'https://raw.githubusercontent.com/411A/V2RayDAR/' + tag + '/configs.example.yaml'
        dl(template, config)
    with open(config, 'r', encoding='utf-8') as fp:
        text = fp.read()
    value = yaml_sq(sing_box_path)
    if re.search(r'(?m)^\s*sing_box_path\s*:', text):
        text = re.sub(r'(?m)^(\s*sing_box_path\s*:\s*).*$', lambda m: m.group(1) + value, text, count=1)
    elif re.search(r'(?m)^probe\s*:\s*$', text):
        text = re.sub(r'(?m)^(probe\s*:\s*)$', lambda m: m.group(1) + '\n  sing_box_path: ' + value, text, count=1)
    else:
        text += '\nprobe:\n  mode: active\n  sing_box_path: ' + value + '\n'
    with open(config, 'w', encoding='utf-8') as fp:
        fp.write(text)
    return config

def source_dir(base):
    return next((os.path.join(base, name) for name in os.listdir(base) if os.path.isdir(os.path.join(base, name)) and 'v2raydar' in name.lower()), base)

vr = j('https://api.github.com/repos/411A/V2RayDAR/releases/latest')
SB_VERSION = embedded_sing_box_version(vr['tag_name'])
pkg_path = '/data/data/com.termux/files/usr/bin/sing-box'
sb_bin = configured_sing_box_path()
if not sb_bin and os.path.isfile(pkg_path) and sing_box_version(pkg_path):
    sb_bin = pkg_path
if not sb_bin:
    print('Installing recommended sing-box ' + SB_VERSION + ' via pkg...')
    subprocess.run(['pkg', 'install', '-y', 'sing-box=' + SB_VERSION], check=False)
    if os.path.isfile(pkg_path) and sing_box_version(pkg_path):
        sb_bin = pkg_path

if not sb_bin:
    print('pkg install did not provide sing-box ' + SB_VERSION + '; trying the recommended GitHub release...')
    m = platform.machine().lower()
    arch = 'arm64' if m in ('aarch64', 'arm64') else 'armv7' if m.startswith('armv7') else 'amd64' if m in ('x86_64', 'amd64') else '386' if m in ('i386', 'i686') else m
    sb = j('https://api.github.com/repos/SagerNet/sing-box/releases/tags/v' + SB_VERSION)
    sb_a = asset_for(sb, ['android', 'termux'], [arch], ['.tar.gz', '.tgz', '.zip'])
    if sb_a:
        sb_dir = os.path.join(root, 'sing-box', SB_VERSION)
        os.makedirs(sb_dir, exist_ok=True)
        sb_file = dl(sb_a['browser_download_url'], os.path.join(sb_dir, sb_a['name']))
        extract(sb_file, sb_dir)
        sb_bin = find_file(sb_dir, 'sing-box')
        if sb_bin:
            chmod_x(sb_bin)

if not sb_bin or not sing_box_version(sb_bin):
    raise SystemExit('sing-box executable was not found. Install the recommended version with: pkg install sing-box=' + SB_VERSION + ' or set V2RAYDAR_SING_BOX_PATH')
warn_if_not_recommended(sb_bin, SB_VERSION)

vr_dir = os.path.join(root, 'v2raydar', vr['tag_name'].lstrip('v'))
os.makedirs(vr_dir, exist_ok=True)
src_tgz = dl(vr['tarball_url'], os.path.join(vr_dir, 'src.tar.gz'))
extract(src_tgz, vr_dir)
src = source_dir(vr_dir)
config = configure_config(vr['tag_name'], sb_bin)

print('sing-box=' + sb_bin)
print('v2raydar_source=' + src)
print('config=' + config)
print('run: cd ' + shlex.quote(src) + ' && cargo run --release -- --no-tui')
EOF
```

</details>

Each block reuses already-downloaded files when possible, prints the resolved `sing-box`, V2RayDAR, and config paths, then prints the command to start V2RayDAR.

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

1. **Get sing-box**. Active probing needs a working `sing-box` executable. V2RayDAR recommends the version declared by `SING_BOX_VERSION` in `src/constants.rs`, but a user-provided working version is accepted. Use `sing-box.exe` on Windows, `sing-box` on Linux and macOS, and `/data/data/com.termux/files/usr/bin/sing-box` from Termux `pkg install`.
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
