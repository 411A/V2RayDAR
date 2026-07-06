# V2RayDAR Installer for Windows
# Usage:
#   irm https://raw.githubusercontent.com/411A/V2RayDAR/main/install.ps1 | iex
#   .\install.ps1 -Version 0.4.0 -Portable
#   .\install.ps1 -Version 0.4.0 -User

param(
    [string]$Version = "",
    [string]$Dir = "",
    [switch]$Portable,
    [switch]$User,
    [switch]$Yes,
    [switch]$Help
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Repo = "411A/V2RayDAR"
$AppName = "v2raydar"
$GitHubApi = "https://api.github.com/repos/$Repo/releases/latest"
$GitHubDownload = "https://github.com/$Repo/releases/download"

# Data files preserved across updates (portable mode)
$DataFiles = @("configs.yaml", "data.db")
$DataDirs = @("v2raydar_data")

# ─── Helpers ───────────────────────────────────────────────────────────────────

function Write-Info    { param([string]$Msg) Write-Host "> $Msg" -ForegroundColor Cyan }
function Write-Warn    { param([string]$Msg) Write-Host "! $Msg" -ForegroundColor Yellow }
function Write-Err     { param([string]$Msg) Write-Host "X $Msg" -ForegroundColor Red; exit 1 }

function Confirm {
    param([string]$Prompt, [bool]$Default = $true)
    if ($Yes) { return $true }
    $suffix = if ($Default) { " [Y/n] " } else { " [y/N] " }
    $answer = Read-Host "$Prompt$suffix"
    if ([string]::IsNullOrWhiteSpace($answer)) { return $Default }
    return $answer -match '^[Yy]'
}

# ─── Platform Detection ────────────────────────────────────────────────────────

function Get-Arch {
    if ([System.Environment]::Is64BitOperatingSystem) { return "x86_64" }
    return "i686"
}

# ─── Asset Selection ───────────────────────────────────────────────────────────

function Select-Asset {
    param([string]$Arch)
    return "v2raydar-windows-${Arch}_with_singbox.zip"
}

# ─── Download ──────────────────────────────────────────────────────────────────

function Get-LatestVersion {
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        $release = Invoke-RestMethod -Uri $GitHubApi -UseBasicParsing
        return $release.tag_name -replace '^v', ''
    }
    catch {
        Write-Err "failed to query latest version from GitHub: $_"
    }
}

function Download-File {
    param([string]$Url, [string]$Dest)
    try {
        Invoke-WebRequest -Uri $Url -OutFile $Dest -UseBasicParsing
    }
    catch {
        Write-Err "failed to download $Url : $_"
    }
}

function Verify-Checksum {
    param([string]$FilePath)

    try {
        $checksumsUrl = "$GitHubDownload/v$Version/checksums.txt"
        $checksums = (Invoke-WebRequest -Uri $checksumsUrl -UseBasicParsing).Content
        $fileName = Split-Path $FilePath -Leaf
        $expected = ($checksums -split "`n" | Where-Object { $_ -match $fileName } | Select-Object -First 1) -split '\s+' | Select-Object -First 1

        if ([string]::IsNullOrWhiteSpace($expected)) {
            Write-Warn "no checksum found for $fileName, skipping verification"
            return
        }

        $hash = (Get-FileHash -Path $FilePath -Algorithm SHA256).Hash.ToLower()
        if ($hash -eq $expected.ToLower()) {
            Write-Info "checksum verified"
        }
        else {
            Write-Err "checksum mismatch: expected $expected, got $hash"
        }
    }
    catch {
        Write-Warn "could not verify checksum: $_"
    }
}

# ─── Backup/Restore Data ──────────────────────────────────────────────────────

function Backup-Data {
    param([string]$Dir)

    $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null

    foreach ($file in $DataFiles) {
        $src = Join-Path $Dir $file
        if (Test-Path $src) { Copy-Item -Path $src -Destination $tmpDir -Force }
    }
    foreach ($dirName in $DataDirs) {
        $src = Join-Path $Dir $dirName
        if (Test-Path $src) { Copy-Item -Path $src -Destination $tmpDir -Recurse -Force }
    }

    return $tmpDir
}

function Restore-Data {
    param([string]$Dir, [string]$TmpDir)

    foreach ($file in $DataFiles) {
        $src = Join-Path $TmpDir $file
        if (Test-Path $src) { Copy-Item -Path $src -Destination $Dir -Force }
    }
    foreach ($dirName in $DataDirs) {
        $src = Join-Path $TmpDir $dirName
        if (Test-Path $src) { Copy-Item -Path $src -Destination $Dir -Recurse -Force }
    }

    Remove-Item -Path $TmpDir -Recurse -Force -ErrorAction SilentlyContinue
}

# ─── Extract ───────────────────────────────────────────────────────────────────

function Extract-Archive {
    param([string]$FilePath, [string]$Dest)

    $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null

    Expand-Archive -LiteralPath $FilePath -DestinationPath $tmpDir -Force

    # Copy all contents from extracted dir to Dest
    Copy-Item -Path "$tmpDir\*" -Destination $Dest -Recurse -Force

    Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
}

# ─── Install Modes ─────────────────────────────────────────────────────────────

function Do-PortableInstall {
    param([string]$Target)

    $exePath = Join-Path $Target "$AppName.exe"
    $existing = Test-Path $exePath

    if ($existing) {
        Write-Info "existing V2RayDAR installation found at $Target"
        if (Confirm -Prompt "update to latest version?") {
            $backupDir = Backup-Data -Dir $Target

            $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
            New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null
            $archive = Join-Path $tmpDir $Asset

            Write-Info "downloading ${Asset}..."
            $downloadUrl = "$GitHubDownload/v$Version/$Asset"
            Download-File -Url $downloadUrl -Dest $archive
            Verify-Checksum -FilePath $archive

            Write-Info "updating..."
            Extract-Archive -FilePath $archive -Dest $Target
            Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue

            Restore-Data -Dir $Target -TmpDir $backupDir

            Write-Info "updated to v$Version"
        }
        else {
            Write-Info "keeping current version"
            return
        }
    }
    else {
        Write-Info "fresh install to $Target"
        if (-not (Test-Path $Target)) {
            New-Item -ItemType Directory -Path $Target -Force | Out-Null
        }

        $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
        New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null
        $archive = Join-Path $tmpDir $Asset

        Write-Info "downloading ${Asset}..."
        $downloadUrl = "$GitHubDownload/v$Version/$Asset"
        Download-File -Url $downloadUrl -Dest $archive
        Verify-Checksum -FilePath $archive

        Write-Info "installing..."
        Extract-Archive -FilePath $archive -Dest $Target
        Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue

        Write-Info "installed V2RayDAR"
    }

    Write-Host ""
    Write-Info "installed to: $exePath"
    Write-Info "run:  cd $Target; .\$AppName.exe --portable"
}

function Do-UserInstall {
    param([string]$BinDir)

    $exePath = Join-Path $BinDir "$AppName.exe"
    $existing = Test-Path $exePath

    if ($existing) {
        Write-Info "existing V2RayDAR binary found at $exePath"
        if (Confirm -Prompt "update to latest version?") {
            $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
            New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null
            $archive = Join-Path $tmpDir $Asset

            Write-Info "downloading ${Asset}..."
            $downloadUrl = "$GitHubDownload/v$Version/$Asset"
            Download-File -Url $downloadUrl -Dest $archive
            Verify-Checksum -FilePath $archive

            $extractDir = Join-Path $tmpDir "extract"
            New-Item -ItemType Directory -Path $extractDir -Force | Out-Null
            Extract-Archive -FilePath $archive -Dest $extractDir

            $extractedExe = Join-Path $extractDir "$AppName.exe"
            if (Test-Path $extractedExe) {
                Copy-Item -Path $extractedExe -Destination $exePath -Force
            }
            else {
                $found = Get-ChildItem -Path $extractDir -Filter "$AppName.exe" -Recurse | Select-Object -First 1
                if ($found) { Copy-Item -Path $found.FullName -Destination $exePath -Force }
                else { Write-Err "could not find $AppName.exe in archive" }
            }

            Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
            Write-Info "updated to v$Version"
        }
        else {
            Write-Info "keeping current version"
            return
        }
    }
    else {
        Write-Info "fresh install to $exePath"
        if (-not (Test-Path $BinDir)) {
            New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
        }

        $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
        New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null
        $archive = Join-Path $tmpDir $Asset

        Write-Info "downloading ${Asset}..."
        $downloadUrl = "$GitHubDownload/v$Version/$Asset"
        Download-File -Url $downloadUrl -Dest $archive
        Verify-Checksum -FilePath $archive

        $extractDir = Join-Path $tmpDir "extract"
        New-Item -ItemType Directory -Path $extractDir -Force | Out-Null
        Extract-Archive -FilePath $archive -Dest $extractDir

        $extractedExe = Join-Path $extractDir "$AppName.exe"
        if (Test-Path $extractedExe) {
            Copy-Item -Path $extractedExe -Destination $exePath -Force
        }
        else {
            $found = Get-ChildItem -Path $extractDir -Filter "$AppName.exe" -Recurse | Select-Object -First 1
            if ($found) { Copy-Item -Path $found.FullName -Destination $exePath -Force }
            else { Write-Err "could not find $AppName.exe in archive" }
        }

        Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
        Write-Info "installed binary"
    }

    Write-Host ""
    Write-Info "installed to: $exePath"
    Write-Info "run:  $AppName"
}

# ─── Interactive Prompts ───────────────────────────────────────────────────────

function Select-InstallMode {
    $arch = Get-Arch
    Write-Host ""
    Write-Host "  ========================================"
    Write-Host "       V2RayDAR Installer v$Version"
    Write-Host "  ========================================"
    Write-Host ""
    Write-Info "Detected: Windows $arch"
    Write-Info "Latest version: $Version"
    Write-Host ""

    $desktop = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
    if ([string]::IsNullOrWhiteSpace($desktop)) { $desktop = Join-Path $env:USERPROFILE "Desktop" }
    $defaultDir = if (Test-Path $desktop) { Join-Path $desktop "V2RayDAR" } else { Join-Path $env:USERPROFILE "V2RayDAR" }

    Write-Host "  Installation mode:"
    Write-Host "    1) Portable  — everything in one folder (recommended)"
    Write-Host "    2) User      — binary to AppData"
    Write-Host ""

    if ($Yes) { $choice = "1" }
    else {
        $choice = Read-Host "? Choose mode [1-2, default: 1]"
        if ([string]::IsNullOrWhiteSpace($choice)) { $choice = "1" }
    }

    switch ($choice) {
        "1" {
            $Script:InstallMode = "portable"
            if ($Yes -or [string]::IsNullOrWhiteSpace($Dir)) {
                $Script:InstallDir = $defaultDir
            }
            else {
                $Script:InstallDir = $Dir
            }
        }
        "2" {
            $Script:InstallMode = "user"
            $localAppData = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { "$env:USERPROFILE\AppData\Local" }
            $Script:InstallDir = Join-Path $localAppData "V2RayDAR"
        }
        default { Write-Err "invalid choice: $choice" }
    }
}

# ─── Help ──────────────────────────────────────────────────────────────────────

function Show-Help {
    Write-Host @"
V2RayDAR Installer for Windows

Usage:
    irm https://raw.githubusercontent.com/411A/V2RayDAR/main/install.ps1 | iex
    .\install.ps1 -Version 0.4.0 -Portable -Dir C:\V2RayDAR
    .\install.ps1 -Version 0.4.0 -User

Options:
    -Version VERSION    Install a specific version (default: latest)
    -Dir DIR            Install to a specific directory (portable mode)
    -Portable           Install in portable mode (everything in one folder)
    -User               Install in user mode (binary to AppData)
    -Yes                Skip all confirmation prompts
    -Help               Show this help message
"@
}

# ─── Main ──────────────────────────────────────────────────────────────────────

function Main {
    if ($Help) { Show-Help; return }

    # Get version
    if ([string]::IsNullOrWhiteSpace($Version)) {
        $Version = Get-LatestVersion
    }
    Write-Info "version: $Version"

    # Detect arch
    $arch = Get-Arch
    Write-Info "arch: $arch"

    $Asset = Select-Asset -Arch $arch
    Write-Info "asset: $Asset"

    # Determine install mode
    if ($Portable) {
        $Script:InstallMode = "portable"
        $desktop = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
        if ([string]::IsNullOrWhiteSpace($desktop)) { $desktop = Join-Path $env:USERPROFILE "Desktop" }
        $defaultDir = if (Test-Path $desktop) { Join-Path $desktop "V2RayDAR" } else { Join-Path $env:USERPROFILE "V2RayDAR" }
        $Script:InstallDir = if (-not [string]::IsNullOrWhiteSpace($Dir)) { $Dir } else { $defaultDir }
    }
    elseif ($User) {
        $Script:InstallMode = "user"
        $localAppData = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { "$env:USERPROFILE\AppData\Local" }
        $Script:InstallDir = Join-Path $localAppData "V2RayDAR"
    }
    else {
        Select-InstallMode
    }

    Write-Host ""
    Write-Info "will install to: $InstallDir"

    if (-not (Confirm -Prompt "Proceed with installation?" -Default $true)) {
        Write-Host "installation cancelled" -ForegroundColor Yellow
        return
    }

    switch ($InstallMode) {
        "portable" { Do-PortableInstall -Target $InstallDir }
        "user"     { Do-UserInstall -BinDir $InstallDir }
    }

    Write-Host ""
    Write-Info "done!"
    Write-Host ""
}

Main
