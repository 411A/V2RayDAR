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

# ─── Cleanup on Ctrl+C / forced exit ─────────────────────────────────────────
$Script:TempPaths = @()
function Remove-TempItems {
    foreach ($p in $Script:TempPaths) {
        if (Test-Path $p) {
            try { Remove-Item -Path $p -Recurse -Force -ErrorAction SilentlyContinue } catch {}
        }
    }
}
Register-EngineEvent PowerShell.Exiting -Action { Remove-TempItems } | Out-Null

$Repo = "411A/V2RayDAR"
$AppName = "v2raydar"
$GitHubApi = "https://api.github.com/repos/$Repo/releases/latest"
$GitHubDownload = "https://github.com/$Repo/releases/download"

# ─── Helpers ───────────────────────────────────────────────────────────────────

function Write-Info    { param([string]$Msg) Write-Host "> $Msg" -ForegroundColor Cyan }
function Write-Warn    { param([string]$Msg) Write-Host "! $Msg" -ForegroundColor Yellow }
function Write-Err     { param([string]$Msg) Write-Host "X $Msg" -ForegroundColor Red; exit 1 }

function Confirm {
    param([string]$Prompt, [bool]$Default = $true)
    if ($Yes) { return $true }
    $suffix = if ($Default) { " [Y/n] " } else { " [y/N] " }
    try {
        $answer = Read-Host "$Prompt$suffix"
    }
    catch {
        Remove-TempItems
        Write-Host ""
        Write-Info "cancelled"
        exit 130
    }
    if ([string]::IsNullOrWhiteSpace($answer)) { return $Default }
    return $answer -match '^[Yy]'
}

# ─── Version Comparison ────────────────────────────────────────────────────────
# Compare two semver strings (e.g. "0.4.0" vs "0.5.3").
# Returns: 0 if equal, 1 if $Left > $Right, -1 if $Left < $Right
# Uses .NET [version] for idiomatic, efficient comparison with fallback.
function Compare-Version {
    param([string]$Left, [string]$Right)

    $l = $Left.TrimStart('v')
    $r = $Right.TrimStart('v')

    # Use .NET [version] — idiomatic, handles Major.Minor[.Build[.Revision]]
    try {
        return [version]$l.CompareTo([version]$r)
    }
    catch {
        # Fallback for non-standard version strings
    }

    # Manual fallback
    $lParts = $l.Split('.')
    $rParts = $r.Split('.')
    $max = [math]::Max($lParts.Length, $rParts.Length)
    for ($i = 0; $i -lt $max; $i++) {
        $lNum = if ($i -lt $lParts.Length) { [int]$lParts[$i] } else { 0 }
        $rNum = if ($i -lt $rParts.Length) { [int]$rParts[$i] } else { 0 }
        if ($lNum -gt $rNum) { return 1 }
        if ($lNum -lt $rNum) { return -1 }
    }
    return 0
}

# ─── Installation Detection ───────────────────────────────────────────────────
# Search common locations for an existing v2raydar binary and get its version.
# Sets $Script:FoundPath and $Script:FoundVersion. Returns $true if found.
function Find-Installed {
    $Script:FoundPath = $null
    $Script:FoundVersion = $null

    $desktop = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
    if ([string]::IsNullOrWhiteSpace($desktop)) { $desktop = Join-Path $env:USERPROFILE "Desktop" }

    $localAppData = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { "$env:USERPROFILE\AppData\Local" }

    $candidatePaths = @()
    if (Test-Path $desktop) {
        $candidatePaths += Join-Path $desktop "V2RayDAR"
    }
    $candidatePaths += Join-Path $env:USERPROFILE "V2RayDAR"
    $candidatePaths += Join-Path $localAppData "V2RayDAR"

    foreach ($dir in $candidatePaths) {
        $exePath = Join-Path $dir "$AppName.exe"
        if (Test-Path $exePath) {
            $Script:FoundPath = $dir
            Get-VersionFromBinary -Path $exePath | Out-Null
            return $true
        }
    }

    # Check PATH
    $inPath = Get-Command $AppName -ErrorAction SilentlyContinue
    if ($inPath -and (Test-Path $inPath.Source)) {
        $Script:FoundPath = Split-Path $inPath.Source -Parent
        Get-VersionFromBinary -Path $inPath.Source | Out-Null
        return $true
    }

    return $false
}

# Extract version from a binary by running --version.
function Get-VersionFromBinary {
    param([string]$Path)

    try {
        $output = & $Path --version 2>&1 | Out-String
        if ($LASTEXITCODE -eq 0 -and $output) {
            # Output format: "v2raydar 0.5.3" or "v2raydar v0.5.3"
            if ($output -match 'v?(\d+\.\d+\.\d+)') {
                $Script:FoundVersion = $Matches[1]
                return $true
            }
        }
    }
    catch {}
    return $false
}

# ─── Platform Detection ────────────────────────────────────────────────────────

function Get-Arch {
    $cpu = $env:PROCESSOR_ARCHITECTURE
    switch -Regex ($cpu) {
        'ARM64|aarch64' { return "aarch64" }
        'ARM|armv7'     { return "armv7" }
        'AMD64|x86_64'  { return "x86_64" }
        default {
            if ([System.Environment]::Is64BitOperatingSystem) { return "x86_64" }
            return "i686"
        }
    }
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

    $maxRetries = 5
    $retryDelay = 3

    for ($attempt = 1; $attempt -le $maxRetries; $attempt++) {
        $fileStream = $null; $stream = $null; $response = $null
        try {
            [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

            $existingBytes = 0
            if (Test-Path $Dest) {
                $existingBytes = (Get-Item $Dest).Length
            }

            $request = [Net.HttpWebRequest]::Create($Url)
            $request.AllowAutoRedirect = $true
            $request.Timeout = 300000

            if ($existingBytes -gt 0) {
                $request.AddRange($existingBytes)
                Write-Info "resuming from $("{0:N2}" -f ($existingBytes / 1MB)) MB..."
            }

            $response = $request.GetResponse()

            if ($response.StatusCode -ne [Net.HttpStatusCode]::PartialContent) {
                $existingBytes = 0
            }

            $totalBytes = if ($response.ContentLength -gt 0) { $response.ContentLength + $existingBytes } else { 0 }
            $stream = $response.GetResponseStream()

            if ($existingBytes -gt 0 -and (Test-Path $Dest)) {
                $fileStream = [IO.File]::Open($Dest, [IO.FileMode]::Append, [IO.FileAccess]::Write, [IO.FileShare]::None)
            }
            else {
                $fileStream = [IO.File]::Create($Dest)
            }

            $buffer = New-Object byte[] 65536
            $totalRead = $existingBytes
            $sw = [Diagnostics.Stopwatch]::StartNew()
            $lastBarLen = 0

            while ($true) {
                $read = $stream.Read($buffer, 0, $buffer.Length)
                if ($read -eq 0) { break }
                $fileStream.Write($buffer, 0, $read)
                $totalRead += $read

                $elapsed = $sw.Elapsed.TotalSeconds
                if ($elapsed -gt 0) {
                    $speed = $totalRead / $elapsed
                    if ($speed -ge 1MB)     { $speedStr = "{0:N1} MB/s" -f ($speed / 1MB) }
                    elseif ($speed -ge 1KB) { $speedStr = "{0:N1} KB/s" -f ($speed / 1KB) }
                    else                    { $speedStr = "{0:N0} B/s"  -f $speed }

                    if ($totalBytes -gt 0) {
                        $pct = [math]::Floor(($totalRead / $totalBytes) * 100)
                        $dlMB = "{0:N2}" -f ($totalRead / 1MB)
                        $totalMB = "{0:N2}" -f ($totalBytes / 1MB)
                        $bar = "$pct%  $dlMB/$totalMB MB  $speedStr"
                    } else {
                        $dlMB = "{0:N2}" -f ($totalRead / 1MB)
                        $bar = "$dlMB MB  $speedStr"
                    }

                    $pad = " " * [math]::Max(0, $lastBarLen - $bar.Length)
                    Write-Host "`r$bar$pad" -NoNewline
                    $lastBarLen = $bar.Length
                }
            }

            $fileStream.Close(); $stream.Close(); $response.Close()
            if ($lastBarLen -gt 0) { Write-Host "" }
            return
        }
        catch {
            try { if ($fileStream)  { $fileStream.Close() } } catch {}
            try { if ($stream)      { $stream.Close() }     } catch {}
            try { if ($response)    { $response.Close() }   } catch {}

            if ($_.Exception -is [System.OperationCanceledException] -or
                $_.Exception -is [System.Management.Automation.PipelineStoppedException]) {
                Remove-TempItems
                Write-Host ""
                Write-Info "cancelled"
                exit 130
            }

            if ($attempt -lt $maxRetries) {
                $delay = $retryDelay * $attempt
                Write-Warn "download failed (attempt $attempt/$maxRetries), retrying in ${delay}s..."
                Start-Sleep -Seconds $delay
            }
            else {
                Write-Host ""
                Write-Err "failed to download $Url after $maxRetries attempts : $_"
            }
        }
    }
}

function Verify-Checksum {
    param([string]$FilePath)

    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
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
            $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
            New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null
            $Script:TempPaths += $tmpDir
            $archive = Join-Path $tmpDir $Asset

            Write-Info "downloading ${Asset}..."
            $downloadUrl = "$GitHubDownload/v$Version/$Asset"
            Download-File -Url $downloadUrl -Dest $archive
            Verify-Checksum -FilePath $archive

            Write-Info "updating..."
            Extract-Archive -FilePath $archive -Dest $tmpDir

            # Replace only binaries — user data stays untouched
            Copy-Item -Path "$tmpDir\$AppName.exe" -Destination $exePath -Force
            $singBox = Join-Path $tmpDir "sing-box.exe"
            if (Test-Path $singBox) {
                Copy-Item -Path $singBox -Destination (Join-Path $Target "sing-box.exe") -Force
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
        Write-Info "fresh install to $Target"
        if (-not (Test-Path $Target)) {
            New-Item -ItemType Directory -Path $Target -Force | Out-Null
        }

        $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
        New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null
        $Script:TempPaths += $tmpDir
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
            $Script:TempPaths += $tmpDir
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
        $Script:TempPaths += $tmpDir
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
    try {
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

        # ─── Check for existing installation ────────────────────────────────────
        Write-Host ""
        Write-Host "  ========================================"
        Write-Host "       V2RayDAR Installer v$Version"
        Write-Host "  ========================================"
        Write-Host ""
        Write-Info "Detected: Windows $arch"

        $found = Find-Installed

        if ($found) {
            if ($Script:FoundVersion) {
                $cmp = Compare-Version -Left $Script:FoundVersion -Right $Version

                if ($cmp -eq 0) {
                    # Same version — already up to date
                    Write-Host ""
                    Write-Host "> V2RayDAR v$($Script:FoundVersion) (latest version) is already installed." -ForegroundColor Green
                    if ($Script:FoundPath) {
                        Write-Info "location: $($Script:FoundPath)\$AppName.exe"
                    }
                    Write-Host ""
                    return
                }
                elseif ($cmp -lt 0) {
                    # Installed version is older — outdated
                    Write-Host ""
                    Write-Host "! V2RayDAR v$($Script:FoundVersion) is installed, but v$Version is available." -ForegroundColor Yellow
                    if ($Script:FoundPath) {
                        Write-Info "location: $($Script:FoundPath)\$AppName.exe"
                    }
                    Write-Host ""
                    if (-not (Confirm -Prompt "update from v$($Script:FoundVersion) to v$Version?")) {
                        Write-Info "cancelled"
                        return
                    }
                }
                else {
                    # Installed version is newer than latest release (unusual)
                    Write-Host ""
                    Write-Host "> V2RayDAR v$($Script:FoundVersion) is already installed (newer than latest release v$Version)." -ForegroundColor Green
                    if ($Script:FoundPath) {
                        Write-Info "location: $($Script:FoundPath)\$AppName.exe"
                    }
                    Write-Host ""
                    return
                }
            }
            else {
                # Found binary but couldn't determine version
                Write-Host ""
                Write-Warn "V2RayDAR is installed at $($Script:FoundPath)\$AppName.exe, but could not determine its version."
                Write-Host ""
                if (-not (Confirm -Prompt "update to latest version?")) {
                    Write-Info "cancelled"
                    return
                }
            }
        }
        else {
            # Not installed
            Write-Host ""
            Write-Info "V2RayDAR is not installed."
        }

        # ─── Proceed with installation ──────────────────────────────────────────
        Write-Host ""

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
    finally {
        Remove-TempItems
    }
}

Main
