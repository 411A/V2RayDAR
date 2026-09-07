#!/bin/sh
# shellcheck shell=dash
# V2RayDAR Installer — https://github.com/411A/V2RayDAR
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh -s -- --help
#   curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh -s -- --portable
#   curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh -s -- --user

set -eu

# ─── Cleanup on interrupt ─────────────────────────────────────────────────────
_CLEANUP_DIRS=""
cleanup() {
    if [ -n "$_CLEANUP_DIRS" ]; then
        for _d in $_CLEANUP_DIRS; do
            [ -d "$_d" ] && rm -rf "$_d"
        done
    fi
    printf '\n\033[1;33m!\033[0m cancelled\n' >&2
    exit 130
}
trap cleanup INT TERM HUP

# Temp dir creator that registers for auto-cleanup on interrupt
mktemp_d() {
    _td="$(mktemp -d)"
    _CLEANUP_DIRS="$_CLEANUP_DIRS $_td"
    echo "$_td"
}

REPO="411A/V2RayDAR"
APP_NAME="v2raydar"
GITHUB_API="https://api.github.com/repos/${REPO}/releases/latest"
GITHUB_DOWNLOAD="https://github.com/${REPO}/releases/download"

# ─── Helpers ───────────────────────────────────────────────────────────────────

info()  { printf '\033[1;34m>\033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33m!\033[0m %s\n' "$*"; }
err()   { printf '\033[1;31m✗\033[0m %s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || err "required command not found: $1"
}

# Ensure a download tool exists. When the script arrives without curl (local
# copy, wget pipe), bootstrap it automatically via the system package manager
# instead of failing: update indexes, then install curl (and tar, needed to
# extract the release archive).
ensure_curl() {
    command -v curl >/dev/null 2>&1 && return 0
    if command -v pkg >/dev/null 2>&1; then
        # Termux: pkg wraps apt; a full upgrade here would be slow and is not
        # needed to install two small packages, so update + install only.
        info "curl not found, installing it via pkg..."
        pkg update -y && pkg install -y curl tar \
            || err "failed to install curl via pkg (run: pkg install -y curl tar)"
    elif command -v apt-get >/dev/null 2>&1; then
        # Debian/Ubuntu (and Termux fallback): script-safe apt frontend.
        _apt="apt-get"
        if [ "$(id -u)" -ne 0 ]; then
            command -v sudo >/dev/null 2>&1 \
                || err "curl not found and no root/sudo available; install curl and tar first"
            _apt="sudo apt-get"
        fi
        info "curl not found, installing it via ${_apt}..."
        # shellcheck disable=SC2086
        $_apt update && $_apt full-upgrade -y && $_apt install -y curl tar \
            || err "failed to install curl via ${_apt}"
    else
        err "required command not found: curl"
    fi
    command -v curl >/dev/null 2>&1 \
        || err "curl installation failed; install curl and tar manually, then rerun"
}

confirm() {
    if [ "${NON_INTERACTIVE:-0}" = "1" ]; then
        return 0
    fi
    _default="${2:-y}"
    if [ "$_default" = "y" ]; then
        printf '\033[1;36m?\033[0m %s [Y/n] ' "$1"
    else
        printf '\033[1;36m?\033[0m %s [y/N] ' "$1"
    fi
    read -r _answer </dev/tty || _answer=""
    case "$_answer" in
        [Yy]*) return 0 ;;
        [Nn]*) return 1 ;;
        "")    if [ "$_default" = "y" ]; then return 0; else return 1; fi ;;
        *)     return 0 ;;
    esac
}

# ─── Version Comparison ────────────────────────────────────────────────────────
# Compare two semver strings (e.g. "0.4.0" vs "0.5.3").
# Returns: 0 if equal, 1 if $1 > $2, 2 if $1 < $2
# Uses awk for single-process, portable, efficient comparison.
version_compare() {
    _v1="${1#v}"
    _v2="${2#v}"

    [ "$_v1" = "$_v2" ] && return 0

    awk -v v1="$_v1" -v v2="$_v2" '
    BEGIN {
        n1 = split(v1, a1, ".")
        n2 = split(v2, a2, ".")
        n = (n1 > n2) ? n1 : n2
        for (i = 1; i <= n; i++) {
            x = (i <= n1) ? a1[i] + 0 : 0
            y = (i <= n2) ? a2[i] + 0 : 0
            if (x < y) exit 2
            if (x > y) exit 1
        }
        exit 0
    }'
}

# ─── Installation Detection ───────────────────────────────────────────────────
# Search common locations for an existing v2raydar binary and get its version.
# Sets FOUND_PATH and FOUND_VERSION. Returns 0 if found, 1 if not.
find_installed() {
    FOUND_PATH=""
    FOUND_VERSION=""

    _candidate_paths=""
    [ -d "$HOME/Desktop" ] && _candidate_paths="$_candidate_paths $HOME/Desktop/V2RayDAR"
    _candidate_paths="$_candidate_paths $HOME/V2RayDAR $HOME/.local/bin"
    [ "$IS_TERMUX" = "1" ] && _candidate_paths="$_candidate_paths ${PREFIX:-/usr/local}/bin"

    for _dir in $_candidate_paths; do
        _bin="$_dir/$APP_NAME"
        [ -f "$_bin" ] || continue
        [ -x "$_bin" ] || continue
        FOUND_PATH="$_dir"
        _get_version "$_bin" && return 0
        # Binary found but couldn't get version — still counts as installed
        return 0
    done

    # Check PATH
    set +e
    _which_bin="$(command -v "$APP_NAME" 2>/dev/null)"
    set -e
    if [ -n "$_which_bin" ] && [ -f "$_which_bin" ]; then
        FOUND_PATH="$(dirname "$_which_bin")"
        _get_version "$_which_bin" || true
        return 0
    fi

    return 1
}

# Extract version from a binary by running --version.
# Sets FOUND_VERSION if successful. Returns 0/1.
_get_version() {
    _bin="$1"
    set +e
    _ver_output="$("$_bin" --version 2>/dev/null)"
    _ver_exit=$?
    set -e
    if [ "$_ver_exit" -eq 0 ] && [ -n "$_ver_output" ]; then
        # Output format: "v2raydar 0.5.3" or "v2raydar v0.5.3"
        FOUND_VERSION="$(echo "$_ver_output" | sed -n 's/^[^ ]* *v\{0,1\}\([0-9][0-9.]*\).*/\1/p' | head -1)"
        [ -n "$FOUND_VERSION" ] && return 0
    fi
    return 1
}

# ─── Platform Detection ────────────────────────────────────────────────────────

detect_os() {
    _os="$(uname -s)"
    case "$_os" in
        Linux*)   OS="linux" ;;
        Darwin*)  OS="macos" ;;
        CYGWIN*|MSYS*|MINGW*)
                err "Windows detected — use: irm https://raw.githubusercontent.com/${REPO}/main/install.ps1 | iex" ;;
        *)      err "unsupported OS: $_os" ;;
    esac
}

detect_arch() {
    _arch="$(uname -m)"
    case "$_arch" in
        x86_64|amd64)         ARCH="x86_64" ;;
        aarch64|arm64)        ARCH="aarch64" ;;
        armv7*|armhf)         ARCH="armv7" ;;
        i686|i386)            ARCH="i686" ;;
        *)                    err "unsupported architecture: $_arch" ;;
    esac
}

detect_termux() {
    IS_TERMUX=0
    case "${PREFIX:-}" in
        *com.termux*) IS_TERMUX=1 ;;
    esac
    if [ -d "/data/data/com.termux/files/usr" ]; then
        IS_TERMUX=1
    fi
}

# ─── Asset Selection ───────────────────────────────────────────────────────────

select_asset() {
    # Termux has its own release archives separate from Linux desktop builds.
    if [ "$IS_TERMUX" = "1" ]; then
        case "$ARCH" in
            aarch64)  ASSET="v2raydar-termux-aarch64.tar.gz" ;;
            x86_64)   ASSET="v2raydar-termux-x86_64.tar.gz" ;;
            *)        err "Termux only supports aarch64 and x86_64" ;;
        esac
        ARCHIVE_TYPE="tar.gz"
        return
    fi

    case "$OS" in
        linux)
            ASSET="v2raydar-linux-${ARCH}_with_singbox.tar.gz"
            ARCHIVE_TYPE="tar.gz"
            ;;
        macos)
            ASSET="v2raydar-macos-universal_with_singbox.zip"
            ARCHIVE_TYPE="zip"
            ;;
    esac
}

# ─── Download ──────────────────────────────────────────────────────────────────

get_latest_version() {
    need curl
    set +e
    _response="$(curl -fsSL "$GITHUB_API" 2>/dev/null)"
    _curl_exit=$?
    set -e
    if [ "$_curl_exit" -ne 0 ] || [ -z "$_response" ]; then
        err "failed to query GitHub API (check network/proxy/firewall)"
    fi
    _version="$(echo "$_response" | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p' | head -1)"
    [ -n "$_version" ] || err "failed to parse version from GitHub response"
    echo "$_version"
}

get_dev_version() {
    need curl
    set +e
    _response="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/tags/dev-build" 2>/dev/null)"
    _curl_exit=$?
    set -e
    if [ "$_curl_exit" -ne 0 ] || [ -z "$_response" ]; then
        err "no dev-build pre-release found (trigger the Release workflow on dev with prerelease first)"
    fi
    _version="$(echo "$_response" | sed -n 's/.*"tag_name": *"\(dev-build\)".*/\1/p' | head -1)"
    [ -n "$_version" ] || err "no dev-build pre-release found (trigger the Release workflow on dev with prerelease first)"
    echo "$_version"
}

download_file() {
    _url="$1"
    _dest="$2"
    _max_retries=5
    _retry_delay=3

    for _attempt in $(seq 1 "$_max_retries"); do
        if command -v curl >/dev/null 2>&1; then
            set +e
            if [ -f "$_dest" ] && [ -s "$_dest" ]; then
                info "resuming download..."
                curl -fSL -C - --progress-bar "$_url" -o "$_dest"
            else
                curl -fSL --progress-bar "$_url" -o "$_dest"
            fi
            _dl_exit=$?
            set -e
            [ "$_dl_exit" -eq 0 ] && return 0
        elif command -v wget >/dev/null 2>&1; then
            set +e
            wget -c -q --show-progress "$_url" -O "$_dest"
            _dl_exit=$?
            set -e
            [ "$_dl_exit" -eq 0 ] && return 0
        else
            err "neither curl nor wget found"
        fi

        if [ "$_attempt" -lt "$_max_retries" ]; then
            _delay=$((_retry_delay * _attempt))
            warn "download failed (attempt $_attempt/$_max_retries), retrying in ${_delay}s..."
            sleep "$_delay"
        fi
    done
    err "download failed after $_max_retries attempts"
}

verify_checksum() {
    _file="$1"
    _checksums_url="${GITHUB_DOWNLOAD}/${TAG}/checksums.txt"
    set +e
    _checksums="$(curl -fsSL "$_checksums_url" 2>/dev/null)"
    _curl_exit=$?
    set -e
    if [ "$_curl_exit" -ne 0 ] || [ -z "$_checksums" ]; then
        warn "could not fetch checksums, skipping verification"
        return 0
    fi

    _expected="$(echo "$_checksums" | grep "$(basename "$_file")" | awk '{print $1}')"
    [ -n "$_expected" ] || { warn "no checksum found for $(basename "$_file"), skipping verification"; return 0; }

    if command -v sha256sum >/dev/null 2>&1; then
        _actual="$(sha256sum "$_file" | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
        _actual="$(shasum -a 256 "$_file" | awk '{print $1}')"
    else
        warn "no sha256sum or shasum found, skipping checksum verification"
        return 0
    fi

    if [ "$_actual" = "$_expected" ]; then
        info "checksum verified"
    else
        err "checksum mismatch: expected $_expected, got $_actual"
    fi
}

# ─── Country IP Database (GeoIP) ─────────────────────────────────────────────
# Keyless ipdeny zone files, refreshed independently of app releases into
# <data-root>/geoip (v4 zones plus an ipv6/ subdir). The app loads them at
# startup (see src/geoip.rs) and runs fine without them. Refreshes are
# stamp-gated (7 days) to respect the provider's usage limits, and failures
# never fail the install — callers run this in a subshell and warn on error.

GEOIP_V4_URL="https://www.ipdeny.com/ipblocks/data/countries/all-zones.tar.gz"
GEOIP_V4_MD5_URL="https://www.ipdeny.com/ipblocks/data/countries/MD5SUM"
GEOIP_V6_URL="https://www.ipdeny.com/ipv6/ipaddresses/blocks/ipv6-all-zones.tar.gz"
GEOIP_V6_MD5_URL="https://www.ipdeny.com/ipv6/ipaddresses/blocks/MD5SUM"
GEOIP_REFRESH_SECONDS=604800
GEOIP_STAMP_FILE=".geoip_stamp"

# Data dir for an existing install: user-mode binaries live in a bin dir, so
# their data follows the XDG-style app root; portable installs keep data
# next to the binary.
geoip_data_dir_for_found() {
    case "$FOUND_PATH" in
        "$HOME/.local/bin"|"${PREFIX:-/usr/local}/bin")
            printf '%s' "${XDG_DATA_HOME:-$HOME/.local/share}/V2RayDAR/v2raydar_data/geoip" ;;
        *)
            printf '%s' "$FOUND_PATH/v2raydar_data/geoip" ;;
    esac
}

# Remove databases from the retired GeoLite2 era (replaced by zone files).
cleanup_legacy_mmdb() {
    for _mmdb in \
        "$FOUND_PATH/GeoLite2-Country.mmdb" \
        "$FOUND_PATH/v2raydar_data/GeoLite2-Country.mmdb" \
        "${XDG_DATA_HOME:-$HOME/.local/share}/V2RayDAR/v2raydar_data/GeoLite2-Country.mmdb" \
        "$HOME/V2RayDAR/v2raydar_data/GeoLite2-Country.mmdb" \
    ; do
        if [ -n "$_mmdb" ] && [ -f "$_mmdb" ]; then
            rm -f "$_mmdb" && info "removed legacy GeoLite2 database: $_mmdb"
        fi
    done
}

# Verify every .zone file in a directory against an MD5SUM listing.
verify_zone_tree() {
    _vdir="$1"
    _md5data="$2"
    if command -v md5sum >/dev/null 2>&1; then
        _hasher="md5sum"
    elif command -v md5 >/dev/null 2>&1; then
        _hasher="md5 -r"
    else
        warn "no md5 tool found, skipping GeoIP verification"
        return 0
    fi

    _checked=0
    while read -r _hash _file; do
        case "$_file" in *.zone) ;; *) continue ;; esac
        if [ ! -f "$_vdir/$_file" ]; then
            warn "GeoIP archive is missing $_file"
            return 1
        fi
        # shellcheck disable=SC2086
        _actual="$($_hasher "$_vdir/$_file" | awk '{print $1}')"
        if [ "$_actual" != "$_hash" ]; then
            warn "GeoIP checksum mismatch for $_file"
            return 1
        fi
        _checked=$((_checked + 1))
    done <<CHECKSUMS
$_md5data
CHECKSUMS

    _files="$(find "$_vdir" -maxdepth 1 -name '*.zone' | wc -l | tr -d ' ')"
    if [ "$_checked" -eq 0 ] || [ "$_checked" -ne "$_files" ]; then
        warn "GeoIP file count mismatch (verified $_checked of $_files)"
        return 1
    fi
    return 0
}

# Download, verify, and atomically install fresh zone files into a geoip dir.
refresh_geoip_data() {
    _geoip_dir="$1"
    _stamp="$_geoip_dir/$GEOIP_STAMP_FILE"
    _now="$(date +%s)"
    if [ -f "$_stamp" ]; then
        _stamped="$(cat "$_stamp" 2>/dev/null || echo 0)"
        case "$_stamped" in ''|*[!0-9]*) _stamped=0 ;; esac
        if [ $((_now - _stamped)) -lt $GEOIP_REFRESH_SECONDS ]; then
            info "country IP database is fresh, skipping update"
            return 0
        fi
    fi

    info "updating country IP database..."
    _tmpdir="$(mktemp_d)"
    set +e
    _v4md5="$(curl -fsSL "$GEOIP_V4_MD5_URL" 2>/dev/null)"
    _v6md5="$(curl -fsSL "$GEOIP_V6_MD5_URL" 2>/dev/null)"
    set -e
    if [ -z "${_v4md5:-}" ] || [ -z "${_v6md5:-}" ]; then
        warn "could not fetch GeoIP checksums, keeping existing data"
        rm -rf "$_tmpdir"
        return 1
    fi
    download_file "$GEOIP_V4_URL" "$_tmpdir/v4.tar.gz"
    download_file "$GEOIP_V6_URL" "$_tmpdir/v6.tar.gz"

    mkdir -p "$_tmpdir/stage" "$_tmpdir/stage/ipv6"
    if ! tar xzf "$_tmpdir/v4.tar.gz" -C "$_tmpdir/stage"; then
        warn "could not extract GeoIP v4 archive, keeping existing data"
        rm -rf "$_tmpdir"
        return 1
    fi
    if ! tar xzf "$_tmpdir/v6.tar.gz" -C "$_tmpdir/stage/ipv6"; then
        warn "could not extract GeoIP v6 archive, keeping existing data"
        rm -rf "$_tmpdir"
        return 1
    fi
    if ! verify_zone_tree "$_tmpdir/stage" "$_v4md5"; then
        warn "GeoIP v4 verification failed, keeping existing data"
        rm -rf "$_tmpdir"
        return 1
    fi
    if ! verify_zone_tree "$_tmpdir/stage/ipv6" "$_v6md5"; then
        warn "GeoIP v6 verification failed, keeping existing data"
        rm -rf "$_tmpdir"
        return 1
    fi

    mkdir -p "$_geoip_dir"
    mv "$_tmpdir/stage" "$_tmpdir/new" || { rm -rf "$_tmpdir"; return 1; }
    rm -rf "$_geoip_dir" || { rm -rf "$_tmpdir"; return 1; }
    mv "$_tmpdir/new" "$_geoip_dir" || { rm -rf "$_tmpdir"; return 1; }
    printf '%s' "$_now" > "$_geoip_dir/$GEOIP_STAMP_FILE"
    rm -rf "$_tmpdir"
    info "country IP database updated"
}

# ─── Extract ───────────────────────────────────────────────────────────────────

extract_archive() {
    _file="$1"
    _dest="$2"

    case "$ARCHIVE_TYPE" in
        tar.gz)
            if [ "$IS_TERMUX" = "1" ]; then
                # Termux archives contain a top-level directory — strip it
                tar xzf "$_file" -C "$_dest" --strip-components=1 --no-same-owner 2>/dev/null \
                    || tar xzf "$_file" -C "$_dest" --strip-components=1
            else
                tar xzf "$_file" -C "$_dest" --no-same-owner 2>/dev/null || tar xzf "$_file" -C "$_dest"
            fi
            ;;
        zip)
            _tmpdir="$(mktemp_d)"
            unzip -qo "$_file" -d "$_tmpdir"
            # Find the v2raydar binary inside the .app bundle
            _app_binary="$(find "$_tmpdir" -name "$APP_NAME" -type f \( -perm /111 -o -perm +111 \) 2>/dev/null | head -1)"
            [ -n "$_app_binary" ] || err "could not find $APP_NAME binary inside archive"
            cp "$_app_binary" "$_dest/$APP_NAME"
            # Copy sing-box if present
            _sing_box="$(find "$_tmpdir" -name "sing-box" -type f 2>/dev/null | head -1)"
            [ -n "$_sing_box" ] && cp "$_sing_box" "$_dest/sing-box" 2>/dev/null || true
            rm -rf "$_tmpdir"
            ;;
    esac
}

# ─── Install ───────────────────────────────────────────────────────────────────

do_portable_install() {
    _target="$1"

    # Check for existing installation
    if [ -f "$_target/$APP_NAME" ] || [ -f "$_target/${APP_NAME}.exe" ]; then
        info "existing V2RayDAR installation found at $_target"
        if confirm "update to latest version?"; then
            _tmpdir="$(mktemp_d)"
            _archive="$_tmpdir/$ASSET"

            info "downloading ${ASSET}..."
            download_file "${GITHUB_DOWNLOAD}/${TAG}/${ASSET}" "$_archive"
            verify_checksum "$_archive"

            info "updating..."
            extract_archive "$_archive" "$_tmpdir"

            # Replace only binaries — user data (configs, db, v2raydar_data) stays untouched
            cp "$_tmpdir/$APP_NAME" "$_target/$APP_NAME"
            chmod +x "$_target/$APP_NAME" 2>/dev/null || true
            if [ -f "$_tmpdir/sing-box" ]; then
                cp "$_tmpdir/sing-box" "$_target/sing-box"
                chmod +x "$_target/sing-box" 2>/dev/null || true
            fi
            rm -rf "$_tmpdir"

            info "updated to v${VERSION}"
        else
            info "keeping current version"
            return
        fi
    else
        info "fresh install to $_target"
        mkdir -p "$_target"

        _tmpdir="$(mktemp_d)"
        _archive="$_tmpdir/$ASSET"

        info "downloading ${ASSET}..."
        download_file "${GITHUB_DOWNLOAD}/${TAG}/${ASSET}" "$_archive"
        verify_checksum "$_archive"

        info "installing..."
        extract_archive "$_archive" "$_target"
        rm -rf "$_tmpdir"

        info "installed V2RayDAR"
    fi

    chmod +x "$_target/$APP_NAME" 2>/dev/null || true
    chmod +x "$_target/sing-box" 2>/dev/null || true

    echo ""
    info "installed to: $_target/$APP_NAME"
    if [ "$IS_TERMUX" = "1" ]; then
        info "run:  cd $_target && ./$APP_NAME --no-tui"
    else
        info "run:  cd $_target && ./$APP_NAME --portable"
    fi
}

do_user_install() {
    _bin_dir="$1"

    if [ -f "$_bin_dir/$APP_NAME" ]; then
        info "existing V2RayDAR binary found at $_bin_dir/$APP_NAME"
        if confirm "update to latest version?"; then
            _tmpdir="$(mktemp_d)"
            _archive="$_tmpdir/$ASSET"

            info "downloading ${ASSET}..."
            download_file "${GITHUB_DOWNLOAD}/${TAG}/${ASSET}" "$_archive"
            verify_checksum "$_archive"

            _extract_dir="$_tmpdir/extract"
            mkdir -p "$_extract_dir"
            extract_archive "$_archive" "$_extract_dir"

            info "updating binary..."
            cp "$_extract_dir/$APP_NAME" "$_bin_dir/$APP_NAME"
            rm -rf "$_tmpdir"

            info "updated to v${VERSION}"
        else
            info "keeping current version"
            return
        fi
    else
        info "fresh install to $_bin_dir/$APP_NAME"
        mkdir -p "$_bin_dir"

        _tmpdir="$(mktemp_d)"
        _archive="$_tmpdir/$ASSET"

        info "downloading ${ASSET}..."
        download_file "${GITHUB_DOWNLOAD}/${TAG}/${ASSET}" "$_archive"
        verify_checksum "$_archive"

        _extract_dir="$_tmpdir/extract"
        mkdir -p "$_extract_dir"
        extract_archive "$_archive" "$_extract_dir"

        cp "$_extract_dir/$APP_NAME" "$_bin_dir/$APP_NAME"
        rm -rf "$_tmpdir"

        info "installed binary"
    fi

    chmod +x "$_bin_dir/$APP_NAME"

    echo ""
    info "installed to: $_bin_dir/$APP_NAME"
    if [ "$IS_TERMUX" = "1" ]; then
        info "run:  $APP_NAME --no-tui"
    else
        info "run:  $APP_NAME"
    fi
}

# ─── PATH Management ──────────────────────────────────────────────────────────

add_to_path() {
    _dir="$1"
    _shell_rc=""

    case "${SHELL:-}" in
        */bash) if [ -f "$HOME/.bashrc" ]; then _shell_rc="$HOME/.bashrc"; fi
                if [ -z "$_shell_rc" ]; then _shell_rc="$HOME/.profile"; fi ;;
        */zsh)  _shell_rc="$HOME/.zshrc" ;;
        */fish) _shell_rc="$HOME/.config/fish/config.fish" ;;
        *)      _shell_rc="$HOME/.profile" ;;
    esac

    if [ -z "$_shell_rc" ]; then return 1; fi

    # Check if already in PATH
    case ":$PATH:" in
        *":$_dir:"*) info "$_dir is already in PATH"; return 0 ;;
    esac

    # Check if already in rc file
    if grep -qF "$_dir" "$_shell_rc" 2>/dev/null; then
        info "$_dir already configured in $_shell_rc"
        return 0
    fi

    if [ "$(basename "$_shell_rc")" = "config.fish" ]; then
        echo "set -gx PATH \"$_dir \$PATH\"" >> "$_shell_rc"
    else
        { echo ""; echo "# V2RayDAR"; echo "export PATH=\"$_dir:\$PATH\""; } >> "$_shell_rc"
    fi
    info "added $_dir to PATH in $_shell_rc"
    warn "restart your shell or run: source $_shell_rc"
}

# ─── Interactive Prompts ───────────────────────────────────────────────────────

interactive_install() {
    # Determine default portable directory: Desktop/V2RayDAR if Desktop exists, else ~/V2RayDAR
    _portable_default="$HOME/V2RayDAR"
    if [ -d "$HOME/Desktop" ]; then
        _portable_default="$HOME/Desktop/V2RayDAR"
    fi

    echo "  Installation mode:"
    echo "    1) Portable  — everything in one folder (recommended)"
    echo "    2) User      — binary to ~/.local/bin"
    echo ""

    if [ "${NON_INTERACTIVE:-0}" = "1" ]; then
        CHOICE="${INSTALL_MODE_NUM:-1}"
    elif [ -t 0 ] || [ -t 2 ]; then
        printf '\033[1;36m?\033[0m Choose mode [1-2, default: 1]: '
        read -r CHOICE </dev/tty || CHOICE=""
        CHOICE="${CHOICE:-1}"
    else
        info "non-interactive mode detected, defaulting to portable install"
        CHOICE=1
    fi

    case "$CHOICE" in
        1)
            INSTALL_MODE="portable"
            if [ "${NON_INTERACTIVE:-0}" = "1" ]; then
                INSTALL_DIR="${INSTALL_DIR:-$_portable_default}"
            elif [ -t 0 ] || [ -t 2 ]; then
                printf '\033[1;36m?\033[0m Install directory [%s]: ' "$_portable_default"
                read -r _input_dir </dev/tty || _input_dir=""
                INSTALL_DIR="${_input_dir:-$_portable_default}"
            else
                INSTALL_DIR="$_portable_default"
            fi
            ;;
        2)
            INSTALL_MODE="user"
            INSTALL_DIR="$HOME/.local/bin"
            ;;
        *)
            err "invalid choice: $CHOICE"
            ;;
    esac
}

# ─── Help ──────────────────────────────────────────────────────────────────────

usage() {
    cat <<EOF
V2RayDAR Installer

Usage:
    curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh
    curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh -s -- --help

Options:
    -v, --version VERSION    Install a specific version (default: latest)
        --pre                Install the dev-build pre-release (developer testing)
    -d, --dir DIR            Install to a specific directory (portable mode)
    -p, --portable           Install in portable mode (everything in one directory)
    -u, --user               Install in user mode (binary to ~/.local/bin)
    -y, --yes                Skip all confirmation prompts
    -h, --help               Show this help message
EOF
}

# ─── Main ──────────────────────────────────────────────────────────────────────

main() {
    VERSION=""
    INSTALL_DIR=""
    INSTALL_MODE=""
    NON_INTERACTIVE=0
    DEV_BUILD=0

    while [ $# -gt 0 ]; do
        case "$1" in
            -v|--version)  VERSION="$2"; shift 2 ;;
            -d|--dir)      INSTALL_DIR="$2"; INSTALL_MODE="portable"; shift 2 ;;
            -p|--portable) INSTALL_MODE="portable"; shift ;;
            -u|--user)     INSTALL_MODE="user"; shift ;;
            -y|--yes)      NON_INTERACTIVE=1; shift ;;
            --pre)         DEV_BUILD=1; shift ;;
            -h|--help)     usage; exit 0 ;;
            *)             err "unknown option: $1 (use --help)" ;;
        esac
    done

    ensure_curl
    need uname
    need mktemp

    detect_os
    detect_arch
    detect_termux

    VERSION="${VERSION#v}"
    case "$(printf '%s' "${VERSION:-}" | tr '[:upper:]' '[:lower:]')" in
        dev|dev-build|pre) DEV_BUILD=1 ;;
    esac
    if [ "$DEV_BUILD" = "1" ]; then
        VERSION="$(get_dev_version)"
    elif [ -z "$VERSION" ]; then
        VERSION="$(get_latest_version)"
    fi
    if [ "$DEV_BUILD" = "1" ]; then TAG="dev-build"; else TAG="v$VERSION"; fi
    info "version: $VERSION"

    select_asset
    info "asset: $ASSET"

    # ─── Check for existing installation ────────────────────────────────────────
    _detected_os="$OS"
    if [ "$IS_TERMUX" = "1" ]; then _detected_os="termux"; fi

    echo ""
    echo "  ========================================"
    echo "       V2RayDAR Installer v${VERSION}"
    echo "  ========================================"
    echo ""
    info "Detected: ${_detected_os} ${ARCH}"

    # ─── One-question auto mode ───────────────────────────────────────────────
    # A single Enter (default Y) installs/updates with defaults and asks
    # nothing else; N keeps the step-by-step prompts below.
    if [ "${NON_INTERACTIVE:-0}" = "0" ]; then
        if [ -t 0 ] || [ -t 2 ]; then
            echo ""
            if confirm "automatic install/update to v${VERSION} (no more questions)?"; then
                NON_INTERACTIVE=1
            fi
        fi
    fi

    if find_installed; then
        # Found an existing installation
        if [ "$DEV_BUILD" = "1" ]; then
            # Developer pre-release: skip semver comparison (dev-build is not
            # a version number) and offer a straight binary replacement.
            echo ""
            if [ -n "$FOUND_VERSION" ]; then
                warn "V2RayDAR v${FOUND_VERSION} is installed at $FOUND_PATH/$APP_NAME."
            else
                warn "V2RayDAR is installed at $FOUND_PATH/$APP_NAME (version unknown)."
            fi
            info "dev-build requested: binaries will be replaced, user data preserved."
            echo ""
            if [ "${NON_INTERACTIVE:-0}" = "1" ]; then
                info "non-interactive mode: proceeding with dev-build install"
            elif [ -t 0 ] || [ -t 2 ]; then
                confirm "install dev-build (${TAG})?" || { info "cancelled"; return; }
            else
                info "non-interactive mode detected, proceeding with dev-build install"
            fi
        elif [ -n "$FOUND_VERSION" ]; then
            # Compare versions
            if version_compare "$FOUND_VERSION" "$VERSION"; then
                # Same version
                echo ""
                printf '\033[1;32m✓\033[0m V2RayDAR v%s (latest version) is already installed.\n' "$FOUND_VERSION"
                if [ -n "$FOUND_PATH" ]; then
                    info "location: $FOUND_PATH/$APP_NAME"
                fi
                echo ""
                # No new app version: still refresh the country IP database.
                cleanup_legacy_mmdb
                ( refresh_geoip_data "$(geoip_data_dir_for_found)" ) \
                    || warn "country IP database update failed, keeping existing data"
                echo ""
                return
            fi

            # Installed version is older
            version_compare "$FOUND_VERSION" "$VERSION" && true  # dummy to avoid set -e issues
            _cmp_exit=$?
            if [ "$_cmp_exit" = "2" ]; then
                # FOUND_VERSION < VERSION → outdated
                echo ""
                printf '\033[1;33m!\033[0m V2RayDAR v%s is installed, but v%s is available.\n' "$FOUND_VERSION" "$VERSION"
                if [ -n "$FOUND_PATH" ]; then
                    info "location: $FOUND_PATH/$APP_NAME"
                fi
                echo ""
                if [ "${NON_INTERACTIVE:-0}" = "1" ]; then
                    info "non-interactive mode: proceeding with update"
                elif [ -t 0 ] || [ -t 2 ]; then
                    if ! confirm "update from v${FOUND_VERSION} to v${VERSION}?"; then
                        info "cancelled"
                        return
                    fi
                else
                    info "non-interactive mode detected, proceeding with update"
                fi
            else
                # FOUND_VERSION > VERSION → installed is newer than latest (unusual)
                echo ""
                printf '\033[1;32m✓\033[0m V2RayDAR v%s is already installed (newer than latest release v%s).\n' "$FOUND_VERSION" "$VERSION"
                if [ -n "$FOUND_PATH" ]; then
                    info "location: $FOUND_PATH/$APP_NAME"
                fi
                echo ""
                cleanup_legacy_mmdb
                ( refresh_geoip_data "$(geoip_data_dir_for_found)" ) \
                    || warn "country IP database update failed, keeping existing data"
                echo ""
                return
            fi
        else
            # Found binary but couldn't determine version
            echo ""
            warn "V2RayDAR is installed at $FOUND_PATH/$APP_NAME, but could not determine its version."
            echo ""
            if [ "${NON_INTERACTIVE:-0}" = "1" ]; then
                info "non-interactive mode: proceeding with update"
            elif [ -t 0 ] || [ -t 2 ]; then
                if ! confirm "update to latest version?"; then
                    info "cancelled"
                    return
                fi
            else
                info "non-interactive mode detected, proceeding with update"
            fi
        fi
    else
        # Not installed
        echo ""
        info "V2RayDAR is not installed."
    fi

    # Auto mode with an existing install: update in place instead of dropping
    # a second copy at the portable default (explicit -d/-p/-u flags still win).
    if [ "${NON_INTERACTIVE:-0}" = "1" ] && [ -z "$INSTALL_MODE" ] && [ -n "${FOUND_PATH:-}" ] && [ -w "$FOUND_PATH" ]; then
        case "$FOUND_PATH" in
            "$HOME/.local/bin"|"${PREFIX:-/usr/local}/bin")
                INSTALL_MODE="user"
                INSTALL_DIR="$FOUND_PATH"
                ;;
            *)
                INSTALL_MODE="portable"
                INSTALL_DIR="$FOUND_PATH"
                ;;
        esac
        info "auto mode: updating in place at $INSTALL_DIR"
    fi

    # ─── Proceed with installation ──────────────────────────────────────────────
    echo ""

    # Interactive install if no mode specified
    if [ -z "$INSTALL_MODE" ]; then
        interactive_install
    fi

    # Default portable directory: Desktop if it exists, otherwise home
    if [ "$INSTALL_MODE" = "portable" ] && [ -z "$INSTALL_DIR" ]; then
        INSTALL_DIR="$HOME/V2RayDAR"
        if [ -d "$HOME/Desktop" ]; then INSTALL_DIR="$HOME/Desktop/V2RayDAR"; fi
    fi

    # User install default
    if [ "$INSTALL_MODE" = "user" ] && [ -z "$INSTALL_DIR" ]; then
        INSTALL_DIR="$HOME/.local/bin"
    fi

    if [ "$INSTALL_MODE" = "portable" ]; then
        info "will install to: $INSTALL_DIR"
    fi
    if [ "${NON_INTERACTIVE:-0}" = "0" ]; then
        confirm "proceed?" || { info "cancelled"; exit 0; }
    fi

    case "$INSTALL_MODE" in
        portable)   do_portable_install "$INSTALL_DIR" ;;
        user)       do_user_install "$INSTALL_DIR"
                    add_to_path "$INSTALL_DIR" ;;
    esac

    # Fresh country IP database next to the new install (user-mode binaries
    # live in a bin dir, so their data follows the XDG-style app root).
    if [ "$INSTALL_MODE" = "user" ]; then
        _geoip_dir="${XDG_DATA_HOME:-$HOME/.local/share}/V2RayDAR/v2raydar_data/geoip"
    else
        _geoip_dir="$INSTALL_DIR/v2raydar_data/geoip"
    fi
    cleanup_legacy_mmdb
    ( refresh_geoip_data "$_geoip_dir" ) \
        || warn "country IP database update failed, keeping existing data"

    echo ""
    info "done!"
    echo ""
}

main "$@"
