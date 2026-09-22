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

# --- Cleanup on Ctrl+C / forced exit -----------------------------------------
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

# --- Helpers -------------------------------------------------------------------

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

# --- Version Comparison --------------------------------------------------------
# Compare two semver strings (e.g. "0.4.0" vs "0.5.3").
# Returns: 0 if equal, 1 if $Left > $Right, -1 if $Left < $Right
# Uses .NET [version] for idiomatic, efficient comparison with fallback.
function Compare-Version {
    param([string]$Left, [string]$Right)

    $l = $Left.TrimStart('v')
    $r = $Right.TrimStart('v')

    # Use .NET [version] - idiomatic, handles Major.Minor[.Build[.Revision]]
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

# --- Installation Detection ---------------------------------------------------
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

# One status report, printed before any prompt: installed version (if any)
# against the requested version, so every later question is informed.
function Show-InstallStatus {
    param([string]$Version, [string]$DisplayVersion, [bool]$DevBuild, [string]$Tag)

    if ($Script:FoundPath) {
        $where = "$($Script:FoundPath)\$AppName.exe"
        if ($DevBuild) {
            if ($Script:FoundVersion) {
                Write-Warn "Installed: V2RayDAR v$($Script:FoundVersion) at $where."
            }
            else {
                Write-Warn "Installed: V2RayDAR at $where (version unknown)."
            }
            Write-Info "Requested: dev-build ($Tag) -- binaries will be replaced, user data preserved."
        }
        elseif ($Script:FoundVersion) {
            $cmp = Compare-Version -Left $Script:FoundVersion -Right $Version
            if ($cmp -eq 0) {
                Write-Host "> V2RayDAR v$($Script:FoundVersion) is already installed at $where." -ForegroundColor Green
            }
            elseif ($cmp -lt 0) {
                Write-Host "! V2RayDAR v$($Script:FoundVersion) is installed at $where, v$Version is available." -ForegroundColor Yellow
            }
            else {
                Write-Host "> V2RayDAR v$($Script:FoundVersion) is installed at $where (newer than requested v$Version)." -ForegroundColor Green
            }
        }
        else {
            Write-Warn "Installed: V2RayDAR at $where (version unknown). Requested: v$Version."
        }
    }
    else {
        Write-Info "No existing installation found. Fresh install of $DisplayVersion."
    }
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

# --- Platform Detection --------------------------------------------------------

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

# --- Asset Selection -----------------------------------------------------------

function Select-Asset {
    param([string]$Arch)
    return "v2raydar-windows-${Arch}_with_singbox.zip"
}

# --- Download ------------------------------------------------------------------

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
    # With -AllowFail the final failure throws (catchable) instead of
    # exiting, for optional downloads like GeoIP data. The main asset keeps
    # the default exit behavior.
    param([string]$Url, [string]$Dest, [switch]$AllowFail)

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
                if ($AllowFail) { throw "failed to download $Url after $maxRetries attempts : $_" }
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
        # Release assets serve as octet-stream, which arrives as bytes.
        if ($checksums -is [byte[]]) { $checksums = [System.Text.Encoding]::UTF8.GetString($checksums) }
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

# --- Country IP Database (GeoIP) ---------------------------------------------
# Keyless ipdeny zone blocks, refreshed independently of app releases into
# <data-root>/geoip/zones.txt (one "<cc> <cidr>" per line, v4 and v6 mixed;
# the app prefers it over loose files). The MaxMind mmdb beside it stays the
# primary tier and is never touched by the zone refresh. The app loads both
# at startup (see src/geoip.rs) and runs fine without them. Every installer
# run refreshes unconditionally; failures never fail the install - functions
# return $false and callers warn.

$GeoipV4Url = "https://www.ipdeny.com/ipblocks/data/countries/all-zones.tar.gz"
$GeoipV4Md5Url = "https://www.ipdeny.com/ipblocks/data/countries/MD5SUM"
$GeoipV6Url = "https://www.ipdeny.com/ipv6/ipaddresses/blocks/ipv6-all-zones.tar.gz"
$GeoipV6Md5Url = "https://www.ipdeny.com/ipv6/ipaddresses/blocks/MD5SUM"
$GeoipMmdbUrl = "https://github.com/P3TERX/GeoLite.mmdb/raw/download/GeoLite2-Country.mmdb"
$GeoipMmdbFile = "GeoLite2-Country.mmdb"
# Sanity floor for the mmdb (real file is ~8MB; the publisher ships no
# checksum, so TLS + this size check + structural validation on open in the
# app are the integrity layers).
$GeoipMmdbMinBytes = 1000000

# Data dir for an existing install: portable layouts keep it next to the
# binary (<dir>/v2raydar_data), user-mode installs under LocalAppData.
function Get-GeoipDirForFound {
    # User-mode data lives under LocalAppData; only portable layouts keep it
    # beside the binary. Pointing a bare-exe install here would create a
    # v2raydar_data dir and flip portable auto-detect on the next start.
    if (Test-PortableLayout -Path $Script:FoundPath) {
        return Join-Path $Script:FoundPath "v2raydar_data/geoip"
    }
    $localAppData = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { "$env:USERPROFILE\AppData\Local" }
    return Join-Path $localAppData "V2RayDAR/v2raydar_data/geoip"
}

# True when a folder already runs as portable: a data dir beside the binary,
# or the bundled sing-box beside it. A bare-exe folder ran in installed mode
# (data under LocalAppData) -- updates must preserve that layout, or the
# binary flips databases on the next start.
function Test-PortableLayout {
    param([string]$Path)
    return ((Test-Path (Join-Path $Path "v2raydar_data")) -or (Test-Path (Join-Path $Path "sing-box.exe")))
}

function Heal-FlippedDatabase {
    param([string]$Target)
    # Repairs damage from the pre-fix updater, which dropped sing-box.exe
    # beside bare-exe installs: that flipped portable auto-detect and exposed
    # a stale portable database while the real one sat under LocalAppData. If
    # that looks like what happened here (an installed-root database exists
    # and is bigger than the portable one), back the portable dir up and
    # promote the real database. Never deletes user data.
    $localAppData = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { "$env:USERPROFILE\AppData\Local" }
    $installedDb = Join-Path $localAppData "V2RayDAR/v2raydar_data/data.db"
    if (-not (Test-Path $installedDb)) { return }
    $portableDb = Join-Path $Target "v2raydar_data/data.db"
    $installedSize = (Get-Item $installedDb).Length
    $portableSize = 0
    if (Test-Path $portableDb) { $portableSize = (Get-Item $portableDb).Length }
    if ($installedSize -le $portableSize) { return }
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $portableDir = Join-Path $Target "v2raydar_data"
    if (Test-Path $portableDir) {
        $backup = Join-Path $Target "v2raydar_data.backup-$stamp"
        Move-Item -Path $portableDir -Destination $backup -Force
        Write-Info "backed up stale portable data to $backup"
    }
    New-Item -ItemType Directory -Path $portableDir -Force | Out-Null
    try {
        Copy-Item -Path $installedDb -Destination $portableDb -Force
        Write-Info "restored your real database from $installedDb"
    }
    catch {
        Write-Warn "could not restore database (stop the server and re-run the installer): $($_.Exception.Message)"
    }
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

# Verify an extracted zone tree: every country file we would install must be
# listed in MD5SUM with a matching hash, and unexpected files fail the run.
# Files the listing mentions but the archive does not ship (publisher
# placeholders such as ap.zone) are skipped with a note -- the app runs fine
# on the remaining countries.
function Test-ZoneTree {
    param([string]$Dir, [string]$Md5Data)

    $map = @{}
    foreach ($line in ($Md5Data -split "`n")) {
        $parts = ($line.Trim() -split '\s+')
        if ($parts.Length -lt 2) { continue }
        $hash, $file = $parts[0], $parts[1]
        if (-not $file.EndsWith(".zone")) { continue }
        $map[$file] = $hash
    }

    $checked = 0
    foreach ($f in @(Get-ChildItem -Path $Dir -Filter "*.zone" -File)) {
        if ($f.BaseName.Length -ne 2 -or $f.BaseName -notmatch '^[A-Za-z]{2}$') {
            continue
        }
        if (-not $map.ContainsKey($f.Name)) {
            Write-Warn "GeoIP file $($f.Name) is not in the checksum list"
            return $false
        }
        try {
            $actual = (Get-FileHash -Path $f.FullName -Algorithm MD5).Hash
        }
        catch {
            Write-Warn "could not hash $($f.Name) : $_"
            return $false
        }
        if ($actual.ToLower() -ne $map[$f.Name].ToLower()) {
            Write-Warn "GeoIP checksum mismatch for $($f.Name)"
            return $false
        }
        $checked++
    }

    foreach ($name in $map.Keys) {
        if (-not (Test-Path (Join-Path $Dir $name))) {
            Write-Verbose "GeoIP archive does not ship $name, skipping"
        }
    }

    if ($checked -eq 0) {
        Write-Warn "no GeoIP zone files verified"
        return $false
    }
    return $true
}

# Download and atomically install the MaxMind country database (primary
# tier). No published checksum exists, so failures and undersized files keep
# the previous database.
function Update-GeoipMmdb {
    param([string]$GeoipDir)

    Write-Info "updating GeoIP country database..."
    $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null
    $Script:TempPaths += $tmpDir
    try {
        $dest = Join-Path $tmpDir $GeoipMmdbFile
        try {
            Download-File -Url $GeoipMmdbUrl -Dest $dest -AllowFail
        }
        catch {
            Write-Warn "GeoIP database download failed, keeping existing data"
            return $false
        }
        $bytes = (Get-Item $dest).Length
        if ($bytes -lt $GeoipMmdbMinBytes) {
            Write-Warn "GeoIP database download looks truncated ($bytes bytes), keeping existing data"
            return $false
        }
        if (-not (Test-Path $GeoipDir)) {
            New-Item -ItemType Directory -Path $GeoipDir -Force | Out-Null
        }
        Move-Item -Path $dest -Destination (Join-Path $GeoipDir $GeoipMmdbFile) -Force
        Write-Info "GeoIP country database updated"
        return $true
    }
    finally {
        Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# Consolidate verified zone files into one "<cc> <cidr>" file (v4 + v6).
# Blank lines and publisher comments never reach the output. UTF-8 without
# BOM so the app parses every line (a BOM would corrupt the first entry).
function New-ConsolidatedZones {
    param([string]$Staged, [string]$Dest)

    $writer = New-Object System.IO.StreamWriter($Dest, $false, (New-Object System.Text.UTF8Encoding $false))
    try {
        $writer.WriteLine("# V2RayDAR consolidated country zones (<cc> <cidr>), generated $((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'))")
        $prefixes = 0
        foreach ($dir in @($Staged, (Join-Path $Staged "ipv6"))) {
            foreach ($f in @(Get-ChildItem -Path $dir -Filter "*.zone" -File -ErrorAction SilentlyContinue)) {
                if ($f.BaseName.Length -ne 2 -or $f.BaseName -notmatch '^[A-Za-z]{2}$') { continue }
                foreach ($line in [System.IO.File]::ReadLines($f.FullName)) {
                    $trimmed = $line.Trim()
                    if ([string]::IsNullOrEmpty($trimmed) -or $trimmed.StartsWith('#')) { continue }
                    $writer.WriteLine("$($f.BaseName.ToLowerInvariant()) $trimmed")
                    $prefixes++
                }
            }
        }
    }
    finally {
        $writer.Close()
    }
    return $prefixes
}

# Download, verify, and install fresh zones as one zones.txt into a geoip
# dir. Only zones.txt and stale loose files are touched: the MaxMind mmdb
# beside them (primary tier) is preserved, and legacy <cc>.zone files plus
# the ipv6/ subdir are removed once the single file lands.
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

        try {
            Download-File -Url $GeoipV4Url -Dest (Join-Path $tmpDir "v4.tar.gz") -AllowFail
            Download-File -Url $GeoipV6Url -Dest (Join-Path $tmpDir "v6.tar.gz") -AllowFail
        }
        catch {
            Write-Warn "GeoIP zone download failed, keeping existing data"
            return $false
        }

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

        $zonesTmp = Join-Path $tmpDir "zones.txt"
        $prefixes = New-ConsolidatedZones -Staged $stage -Dest $zonesTmp
        if ($prefixes -le 0) {
            Write-Warn "GeoIP consolidation produced no prefixes, keeping existing data"
            return $false
        }
        if (-not (Test-Path $GeoipDir)) {
            New-Item -ItemType Directory -Path $GeoipDir -Force | Out-Null
        }
        Move-Item -Path $zonesTmp -Destination (Join-Path $GeoipDir "zones.txt") -Force
        Remove-Item -Path (Join-Path $GeoipDir "*.zone") -Force -ErrorAction SilentlyContinue
        Remove-Item -Path (Join-Path $GeoipDir "ipv6") -Recurse -Force -ErrorAction SilentlyContinue
        Write-Info "country IP database updated ($prefixes prefixes in zones.txt)"
        return $true
    }
    finally {
        Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# --- Extract -------------------------------------------------------------------

function Extract-Archive {
    param([string]$FilePath, [string]$Dest)

    $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null

    Expand-Archive -LiteralPath $FilePath -DestinationPath $tmpDir -Force

    # Copy all contents from extracted dir to Dest
    Copy-Item -Path "$tmpDir\*" -Destination $Dest -Recurse -Force

    Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Install Modes -------------------------------------------------------------

function Do-PortableInstall {
    param([string]$Target)

    $exePath = Join-Path $Target "$AppName.exe"
    $existing = Test-Path $exePath

    if ($existing) {
        Write-Info "existing V2RayDAR installation found at $Target"
        if (Confirm -Prompt "update to latest version?") {
            Heal-FlippedDatabase -Target $Target
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

            # Replace only binaries - user data stays untouched
            Copy-Item -Path "$tmpDir\$AppName.exe" -Destination $exePath -Force
            $singBox = Join-Path $tmpDir "sing-box.exe"
            if (Test-Path $singBox) {
                Copy-Item -Path $singBox -Destination (Join-Path $Target "sing-box.exe") -Force
            }
            Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue

            Write-Info "updated to $DisplayVersion"
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
    # Location and run hints print once in Main's result block below.
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
            Write-Info "updated to $DisplayVersion"
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
    # Location and run hints print once in Main's result block below.
}

# --- Interactive Prompts -------------------------------------------------------

function Select-InstallMode {
    $desktop = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
    if ([string]::IsNullOrWhiteSpace($desktop)) { $desktop = Join-Path $env:USERPROFILE "Desktop" }
    $defaultDir = if (Test-Path $desktop) { Join-Path $desktop "V2RayDAR" } else { Join-Path $env:USERPROFILE "V2RayDAR" }

    Write-Host "  Installation mode:"
    Write-Host "    1) Portable  - everything in one folder (recommended)"
    Write-Host "    2) User      - binary to AppData"
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

# --- Help ----------------------------------------------------------------------

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

# --- Main ----------------------------------------------------------------------

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
        $DisplayVersion = if ($DevBuild) { $Tag } else { "v$Version" }
        Write-Info "version: $DisplayVersion"

        # Detect arch
        $arch = Get-Arch
        Write-Info "arch: $arch"

        $Asset = Select-Asset -Arch $arch
        Write-Info "asset: $Asset"

        # Detect first: every question below is asked with the full
        # picture (installed version vs requested version) already shown.
        $found = Find-Installed

        # --- Check for existing installation ------------------------------------
        Write-Host ""
        Write-Host "  ========================================"
        Write-Host "       V2RayDAR Installer $DisplayVersion"
        Write-Host "  ========================================"
        Write-Host ""
        Write-Info "Detected: Windows $arch"
        Write-Host ""
        Show-InstallStatus -Version $Version -DisplayVersion $DisplayVersion -DevBuild $DevBuild -Tag $Tag

        # One-question auto mode: a single Enter (default Y) installs/updates
        # with defaults and asks nothing else; N keeps the step-by-step prompts.
        if (-not $Yes -and [Environment]::UserInteractive -and (-not [Console]::IsInputRedirected)) {
            Write-Host ""
            if (Confirm -Prompt "automatic install/update to v$Version (no more questions)?" -Default $true) {
                $Yes = $true
            }
        }

        if ($found -and $DevBuild) {
            # Developer pre-release: skip semver comparison and offer a
            # straight binary replacement (status reported above).
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
                    # Already up to date (reported above): repair a flipped
                    # database and refresh country data, then stop.
                    Write-Host ""
                    Heal-FlippedDatabase -Target $Script:FoundPath
                    Remove-LegacyMmdb -Roots @($Script:FoundPath)
                    if (-not (Update-GeoipMmdb -GeoipDir (Get-GeoipDirForFound))) {
                        Write-Warn "GeoIP database update failed, keeping existing data"
                    }
                    if (-not (Update-GeoipData -GeoipDir (Get-GeoipDirForFound))) {
                        Write-Warn "country IP database update failed, keeping existing data"
                    }
                    Write-Host ""
                    return
                }
                elseif ($cmp -lt 0) {
                    # Installed version is older (reported above) - outdated
                    Write-Host ""
                    if (-not (Confirm -Prompt "update from v$($Script:FoundVersion) to v$($Version)?")) {
                        Write-Info "cancelled"
                        return
                    }
                }
                else {
                    # Installed version is newer than requested (reported
                    # above): refresh country data, then stop.
                    Write-Host ""
                    Remove-LegacyMmdb -Roots @($Script:FoundPath)
                    Heal-FlippedDatabase -Target $Script:FoundPath
                    if (-not (Update-GeoipMmdb -GeoipDir (Get-GeoipDirForFound))) {
                        Write-Warn "GeoIP database update failed, keeping existing data"
                    }
                    if (-not (Update-GeoipData -GeoipDir (Get-GeoipDirForFound))) {
                        Write-Warn "country IP database update failed, keeping existing data"
                    }
                    Write-Host ""
                    return
                }
            }
            else {
                # Found binary but couldn't determine version (reported above)
                Write-Host ""
                if (-not (Confirm -Prompt "update to latest version?")) {
                    Write-Info "cancelled"
                    return
                }
            }
        }
        # Not installed was reported above: fall through to a fresh install.

        # --- Proceed with installation ------------------------------------------
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
            elseif (Test-PortableLayout -Path $Script:FoundPath) {
                $Script:InstallMode = "portable"
            }
            else {
                # Bare-exe folder: it ran in installed mode (data under
                # LocalAppData), so update the binary in place without
                # dropping sing-box.exe beside it -- that would flip portable
                # auto-detect and orphan the user's database.
                $Script:InstallMode = "user"
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

        # Fresh country data where the running binary reads it: user-mode
        # data lives under LocalAppData, portable data next to the install
        # dir. (Creating a v2raydar_data dir beside a bare-exe install would
        # flip portable auto-detect and orphan the user's database.)
        if ($Script:InstallMode -eq "user") {
            $localAppData = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { "$env:USERPROFILE\AppData\Local" }
            $geoipDir = Join-Path $localAppData "V2RayDAR/v2raydar_data/geoip"
        }
        else {
            $geoipDir = Join-Path $InstallDir "v2raydar_data/geoip"
        }
        Remove-LegacyMmdb -Roots @($Script:FoundPath, $InstallDir)
        if (-not (Update-GeoipMmdb -GeoipDir $geoipDir)) {
            Write-Warn "GeoIP database update failed, keeping existing data"
        }
        # Remove a stale release archive from older installers (binaries only).
        $staleArchive = Join-Path $InstallDir $Asset
        if (Test-Path $staleArchive) {
            Remove-Item -Path $staleArchive -Force -ErrorAction SilentlyContinue
            Write-Info "removed stale archive: $Asset"
        }
        if (-not (Update-GeoipData -GeoipDir $geoipDir)) {
            Write-Warn "country IP database update failed, keeping existing data"
        }

        # --- Result: one human-friendly summary of what changed ---------------
        $localAppData = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { "$env:USERPROFILE\AppData\Local" }
        if ($Script:InstallMode -eq "user") {
            $dataDir = Join-Path $localAppData "V2RayDAR/v2raydar_data"
            $runHint = "$InstallDir\$AppName.exe"
        }
        else {
            $dataDir = Join-Path $Script:InstallDir "v2raydar_data"
            $runHint = "cd $($Script:InstallDir); .\$AppName.exe"
        }
        Write-Host ""
        Write-Host "  ========================================" -ForegroundColor Green
        Write-Host "  Result: V2RayDAR $DisplayVersion ready" -ForegroundColor Green
        Write-Host "  Location: $($Script:InstallDir)\$AppName.exe"
        Write-Host "  Data:       $dataDir"
        Write-Host "  Run:        $runHint"
        Write-Host "  ========================================" -ForegroundColor Green
        Write-Host ""
    }
    finally {
        Remove-TempItems
    }
}

Main
