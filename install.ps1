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
    [switch]$Pre,
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

function Get-DevVersion {
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/tags/dev-build" -UseBasicParsing
        if ($release.tag_name -ne "dev-build") {
            Write-Err "unexpected dev release tag: $($release.tag_name)"
        }
        return "dev-build"
    }
    catch {
        Write-Err "no dev-build pre-release found (run the Release workflow on dev with prerelease first): $_"
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
        $checksumsUrl = "$GitHubDownload/$Tag/checksums.txt"
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

# ─── Country IP Database (GeoIP) ─────────────────────────────────────────────
# Keyless ipdeny zone files, refreshed independently of app releases into
# <data-root>/geoip (v4 zones plus an ipv6/ subdir). The app loads them at
# startup (see src/geoip.rs) and runs fine without them. Every installer run
# refreshes unconditionally; failures never fail the install — functions
# return $false and callers warn.

$GeoipV4Url = "https://www.ipdeny.com/ipblocks/data/countries/all-zones.tar.gz"
$GeoipV4Md5Url = "https://www.ipdeny.com/ipblocks/data/countries/MD5SUM"
$GeoipV6Url = "https://www.ipdeny.com/ipv6/ipaddresses/blocks/ipv6-all-zones.tar.gz"
$GeoipV6Md5Url = "https://www.ipdeny.com/ipv6/ipaddresses/blocks/MD5SUM"

# Data dir for an existing install. On Windows both portable and user-mode
# installs keep the data root next to the install dir (<dir>/v2raydar_data).
function Get-GeoipDirForFound {
    return Join-Path $Script:FoundPath "v2raydar_data/geoip"
}

# Remove databases from the retired GeoLite2 era (replaced by zone files).
function Remove-LegacyMmdb {
    param([string[]]$Roots)
    foreach ($root in $Roots) {
        if ([string]::IsNullOrWhiteSpace($root)) { continue }
        foreach ($candidate in @(
            (Join-Path $root "GeoLite2-Country.mmdb"),
            (Join-Path $root "v2raydar_data/GeoLite2-Country.mmdb")
        )) {
            if (Test-Path $candidate) {
                Remove-Item -Path $candidate -Force -ErrorAction SilentlyContinue
                Write-Info "removed legacy GeoLite2 database: $candidate"
            }
        }
    }
}

# Verify every .zone file in a directory against an MD5SUM listing.
function Test-ZoneTree {
    param([string]$Dir, [string]$Md5Data)

    $checked = 0
    foreach ($line in ($Md5Data -split "`n")) {
        $parts = ($line.Trim() -split '\s+')
        if ($parts.Length -lt 2) { continue }
        $hash, $file = $parts[0], $parts[1]
        if (-not $file.EndsWith(".zone")) { continue }
        $path = Join-Path $Dir $file
        if (-not (Test-Path $path)) {
            Write-Warn "GeoIP archive is missing $file"
            return $false
        }
        try {
            $actual = (Get-FileHash -Path $path -Algorithm MD5).Hash
        }
        catch {
            Write-Warn "could not hash $file : $_"
            return $false
        }
        if ($actual.ToLower() -ne $hash.ToLower()) {
            Write-Warn "GeoIP checksum mismatch for $file"
            return $false
        }
        $checked++
    }

    $files = @(Get-ChildItem -Path $Dir -Filter "*.zone" -File | Where-Object { $_.DirectoryName -eq $Dir }).Count
    if ($checked -eq 0 -or $checked -ne $files) {
        Write-Warn "GeoIP file count mismatch (verified $checked of $files)"
        return $false
    }
    return $true
}

# Download, verify, and atomically install fresh zone files into a geoip dir.
function Update-GeoipData {
    param([string]$GeoipDir)

    Write-Info "updating country IP database..."
    if (-not (Get-Command tar -ErrorAction SilentlyContinue)) {
        Write-Warn "tar not found, keeping existing GeoIP data"
        return $false
    }
    $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null
    $Script:TempPaths += $tmpDir
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        try {
            $v4md5 = (Invoke-WebRequest -Uri $GeoipV4Md5Url -UseBasicParsing).Content
            $v6md5 = (Invoke-WebRequest -Uri $GeoipV6Md5Url -UseBasicParsing).Content
            # file:// and charset-less responses arrive as bytes, not text.
            if ($v4md5 -is [byte[]]) { $v4md5 = [System.Text.Encoding]::UTF8.GetString($v4md5) }
            if ($v6md5 -is [byte[]]) { $v6md5 = [System.Text.Encoding]::UTF8.GetString($v6md5) }
        }
        catch {
            Write-Warn "could not fetch GeoIP checksums, keeping existing data"
            return $false
        }
        if ([string]::IsNullOrWhiteSpace($v4md5) -or [string]::IsNullOrWhiteSpace($v6md5)) {
            Write-Warn "could not fetch GeoIP checksums, keeping existing data"
            return $false
        }

        Download-File -Url $GeoipV4Url -Dest (Join-Path $tmpDir "v4.tar.gz")
        Download-File -Url $GeoipV6Url -Dest (Join-Path $tmpDir "v6.tar.gz")

        $stage = Join-Path $tmpDir "stage"
        $stageV6 = Join-Path $stage "ipv6"
        New-Item -ItemType Directory -Path $stageV6 -Force | Out-Null
        & tar xzf (Join-Path $tmpDir "v4.tar.gz") -C $stage
        if ($LASTEXITCODE -ne 0) {
            Write-Warn "could not extract GeoIP v4 archive, keeping existing data"
            return $false
        }
        & tar xzf (Join-Path $tmpDir "v6.tar.gz") -C $stageV6
        if ($LASTEXITCODE -ne 0) {
            Write-Warn "could not extract GeoIP v6 archive, keeping existing data"
            return $false
        }
        if (-not (Test-ZoneTree -Dir $stage -Md5Data $v4md5)) {
            Write-Warn "GeoIP v4 verification failed, keeping existing data"
            return $false
        }
        if (-not (Test-ZoneTree -Dir $stageV6 -Md5Data $v6md5)) {
            Write-Warn "GeoIP v6 verification failed, keeping existing data"
            return $false
        }

        if (-not (Test-Path $GeoipDir)) {
            New-Item -ItemType Directory -Path $GeoipDir -Force | Out-Null
        }
        $newDir = Join-Path $tmpDir "new"
        Move-Item -Path $stage -Destination $newDir -Force
        Remove-Item -Path $GeoipDir -Recurse -Force
        Move-Item -Path $newDir -Destination $GeoipDir -Force
        Write-Info "country IP database updated"
        return $true
    }
    finally {
        Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
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
            $downloadUrl = "$GitHubDownload/$Tag/$Asset"
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
        $downloadUrl = "$GitHubDownload/$Tag/$Asset"
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
            $downloadUrl = "$GitHubDownload/$Tag/$Asset"
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
        $downloadUrl = "$GitHubDownload/$Tag/$Asset"
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
    -Pre                Install the dev-build pre-release (developer testing)
    -Help               Show this help message
"@
}

# ─── Main ──────────────────────────────────────────────────────────────────────

function Main {
    try {
        if ($Help) { Show-Help; return }

        # Get version (dev-build pre-release for developer testing via -Pre
        # or -Version dev|dev-build|pre; same replace-binaries flow as latest)
        $DevBuild = $false
        if ($Pre -or ($Version -match '^(?i)(dev|dev-build|pre)$')) { $DevBuild = $true }
        $Version = $Version.TrimStart('v')
        if ($DevBuild) {
            $Version = Get-DevVersion
        }
        elseif ([string]::IsNullOrWhiteSpace($Version)) {
            $Version = Get-LatestVersion
        }
        $Version = $Version.TrimStart('v')
        if ($DevBuild) { $Tag = "dev-build" } else { $Tag = "v$Version" }
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

        # One-question auto mode: a single Enter (default Y) installs/updates
        # with defaults and asks nothing else; N keeps the step-by-step prompts.
        if (-not $Yes -and [Environment]::UserInteractive -and (-not [Console]::IsInputRedirected)) {
            Write-Host ""
            if (Confirm -Prompt "automatic install/update to v$Version (no more questions)?" -Default $true) {
                $Yes = $true
            }
        }

        $found = Find-Installed

        if ($found -and $DevBuild) {
            # Developer pre-release: skip semver comparison and offer a
            # straight binary replacement.
            Write-Host ""
            if ($Script:FoundVersion) {
                Write-Warn "V2RayDAR v$($Script:FoundVersion) is installed at $($Script:FoundPath)\$AppName.exe."
            }
            else {
                Write-Warn "V2RayDAR is installed at $($Script:FoundPath)\$AppName.exe (version unknown)."
            }
            Write-Info "dev-build requested: binaries will be replaced, user data preserved."
            Write-Host ""
            if (-not (Confirm -Prompt "install dev-build ($Tag)?")) {
                Write-Info "cancelled"
                return
            }
        }
        elseif ($found) {
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
                    # No new app version: still refresh the country IP database.
                    Remove-LegacyMmdb -Roots @($Script:FoundPath)
                    if (-not (Update-GeoipData -GeoipDir (Get-GeoipDirForFound))) {
                        Write-Warn "country IP database update failed, keeping existing data"
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
                    if (-not (Confirm -Prompt "update from v$($Script:FoundVersion) to v$($Version)?")) {
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
                    Remove-LegacyMmdb -Roots @($Script:FoundPath)
                    if (-not (Update-GeoipData -GeoipDir (Get-GeoipDirForFound))) {
                        Write-Warn "country IP database update failed, keeping existing data"
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

        # Auto mode with an existing install: update in place instead of
        # dropping a second copy at the portable default (explicit -Portable/
        # -User/-Dir flags still win).
        if ($Yes -and [string]::IsNullOrWhiteSpace($Dir) -and (-not $Portable) -and (-not $User) -and [string]::IsNullOrWhiteSpace($Script:InstallMode) -and $found -and $Script:FoundPath -and (Test-Path $Script:FoundPath)) {
            $localAppData = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { "$env:USERPROFILE\AppData\Local" }
            $defaultUserDir = Join-Path $localAppData "V2RayDAR"
            if ($Script:FoundPath -eq $defaultUserDir) {
                $Script:InstallMode = "user"
            }
            else {
                $Script:InstallMode = "portable"
            }
            $Script:InstallDir = $Script:FoundPath
            Write-Info "auto mode: updating in place at $($Script:InstallDir)"
        }

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
        elseif ([string]::IsNullOrWhiteSpace($Script:InstallMode)) {
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

        # Fresh country IP database next to the new install.
        $geoipDir = Join-Path $InstallDir "v2raydar_data/geoip"
        Remove-LegacyMmdb -Roots @($Script:FoundPath, $InstallDir)
        if (-not (Update-GeoipData -GeoipDir $geoipDir)) {
            Write-Warn "country IP database update failed, keeping existing data"
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
