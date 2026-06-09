# V2RayDAR Install And Uninstall

## Download

Download the file for your operating system from the GitHub release page:

- Windows: `v2raydar-windows-x86_64.exe`
- Linux: `v2raydar-linux-x86_64`
- macOS Intel: `v2raydar-macos-x86_64`
- macOS Apple Silicon: `v2raydar-macos-aarch64`

The release also includes `checksums.txt`.

## Verify The Download

Compare your downloaded file's SHA-256 hash with `checksums.txt`.

Windows PowerShell:

```powershell
Get-FileHash .\v2raydar-windows-x86_64.exe -Algorithm SHA256
```

Linux:

```bash
sha256sum ./v2raydar-linux-x86_64
```

macOS:

```bash
shasum -a 256 ./v2raydar-macos-aarch64
```

## First Run

V2RayDAR is a single executable. It creates `v2raydar_data/configs.yaml` on first run.

Active probing requires `sing-box`, which is downloaded separately. On first run, V2RayDAR asks for the local `sing-box` executable path, verifies it with `sing-box version`, saves it in `v2raydar_data/configs.yaml`, and then starts scanning.

Windows users who already have v2rayN should check the v2rayN installation folder for `sing-box.exe`. Otherwise, download sing-box from:

```text
https://github.com/SagerNet/sing-box/releases
```

## Trust Warnings

Checksums verify file integrity, but they do not remove Windows SmartScreen or macOS Gatekeeper warnings. Until signed builds are available, your operating system may ask for confirmation before running V2RayDAR.

## Uninstall

For installed mode:

```bash
v2raydar --uninstall
```

For portable mode:

```bash
v2raydar --portable --uninstall
```

This removes V2RayDAR's generated `v2raydar_data` folder:

- Windows: `%LOCALAPPDATA%\v2raydar_data`
- macOS: `~/Library/Application Support/v2raydar_data`
- Linux: `$XDG_DATA_HOME/v2raydar_data` or `~/.local/share/v2raydar_data`
- Portable: `v2raydar_data` beside the executable

The command asks for confirmation. For unattended scripts, add `--yes`:

```bash
v2raydar --uninstall --yes
```

The command does not remove the downloaded V2RayDAR executable, `sing-box`, or config files supplied through `--config` outside `v2raydar_data`.
