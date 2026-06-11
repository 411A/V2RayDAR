# V2RayDAR Install And Uninstall

## Download

Download the file for your operating system from the GitHub release page:

- Windows: `v2raydar-windows-x86_64.exe`
- Linux: `v2raydar-linux-x86_64`
- macOS: `v2raydar-macos-universal.app.zip`

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
shasum -a 256 ./v2raydar-macos-universal.app.zip
```

## First Run

V2RayDAR is shipped as an icon-bearing executable on Windows, a compatibility-first raw binary on Linux, and one universal icon-bearing `.app` bundle on macOS. It creates `V2RayDAR/v2raydar_data/configs.yaml` under the user's platform app-data folder on first run.

Active probing requires `sing-box`, which is downloaded separately. On first run, V2RayDAR asks for the local `sing-box` executable path, verifies it with `sing-box version`, saves it in the generated `configs.yaml`, and then starts scanning.

Download the sing-box archive for your OS: `sing-box.exe` from the Windows archive, `sing-box` from the Linux archive, `sing-box` from the Android archive for Termux, or `sing-box` from the Darwin archive for macOS. Windows users who already have v2rayN can also check the v2rayN installation folder for `sing-box.exe`.

```text
https://github.com/SagerNet/sing-box/releases/latest
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

This removes V2RayDAR's generated `V2RayDAR/v2raydar_data` folder:

- Windows: `%LOCALAPPDATA%\V2RayDAR\v2raydar_data`
- macOS: `~/Library/Application Support/V2RayDAR/v2raydar_data`
- Linux: `$XDG_DATA_HOME/V2RayDAR/v2raydar_data` or `~/.local/share/V2RayDAR/v2raydar_data`
- Portable: `v2raydar_data` beside the executable

The command asks for confirmation. For unattended scripts, add `--yes`:

```bash
v2raydar --uninstall --yes
```

The command does not remove the downloaded V2RayDAR release artifact, `sing-box`, or config files supplied through `--config` outside `v2raydar_data`.
