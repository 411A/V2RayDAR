#!/bin/sh
# shellcheck shell=dash
# V2RayDAR Installer — https://github.com/411A/V2RayDAR
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh -s -- --help
#   curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh -s -- --portable
#   curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh -s -- --user

set -eu

REPO="411A/V2RayDAR"
APP_NAME="v2raydar"
GITHUB_API="https://api.github.com/repos/${REPO}/releases/latest"
GITHUB_DOWNLOAD="https://github.com/${REPO}/releases/download"

# Data files preserved across updates (portable mode)
DATA_FILES="configs.yaml data.db"
DATA_DIRS="v2raydar_data"

# ─── Helpers ───────────────────────────────────────────────────────────────────

info()  { printf '\033[1;34m>\033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33m!\033[0m %s\n' "$*"; }
err()   { printf '\033[1;31m✗\033[0m %s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || err "required command not found: $1"
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
        "")    [ "$_default" = "y" ] && return 0 || return 1 ;;
        *)     return 0 ;;
    esac
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
    [ -d "/data/data/com.termux/files/usr" ] && IS_TERMUX=1
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
    _version="$(curl -fsSL "$GITHUB_API" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"v\([^"]*\)".*/\1/')"
    [ -n "$_version" ] || err "failed to query latest version from GitHub"
    echo "$_version"
}

download_file() {
    _url="$1"
    _dest="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fSL --progress-bar "$_url" -o "$_dest"
    elif command -v wget >/dev/null 2>&1; then
        wget -q --show-progress "$_url" -O "$_dest"
    else
        err "neither curl nor wget found"
    fi
}

verify_checksum() {
    _file="$1"
    _checksums_url="${GITHUB_DOWNLOAD}/v${VERSION}/checksums.txt"
    _checksums="$(curl -fsSL "$_checksums_url" 2>/dev/null)" || { warn "could not fetch checksums, skipping verification"; return 0; }

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
            _tmpdir="$(mktemp -d)"
            unzip -qo "$_file" -d "$_tmpdir"
            # Find the v2raydar binary inside the .app bundle
            _app_binary="$(find "$_tmpdir" -name "$APP_NAME" -type f -perm +111 2>/dev/null | head -1)"
            [ -n "$_app_binary" ] || err "could not find $APP_NAME binary inside archive"
            cp "$_app_binary" "$_dest/$APP_NAME"
            # Copy sing-box if present
            _sing_box="$(find "$_tmpdir" -name "sing-box" -type f 2>/dev/null | head -1)"
            [ -n "$_sing_box" ] && cp "$_sing_box" "$_dest/sing-box" 2>/dev/null || true
            rm -rf "$_tmpdir"
            ;;
    esac
}

# ─── Backup Data ───────────────────────────────────────────────────────────────

backup_data() {
    _dir="$1"
    _tmpdir="$(mktemp -d)"

    for _file in $DATA_FILES; do
        [ -f "$_dir/$_file" ] && cp "$_dir/$_file" "$_tmpdir/"
    done
    for _dir_name in $DATA_DIRS; do
        [ -d "$_dir/$_dir_name" ] && cp -r "$_dir/$_dir_name" "$_tmpdir/"
    done

    echo "$_tmpdir"
}

restore_data() {
    _dir="$1"
    _tmpdir="$2"

    for _file in $DATA_FILES; do
        [ -f "$_tmpdir/$_file" ] && cp "$_tmpdir/$_file" "$_dir/"
    done
    for _dir_name in $DATA_DIRS; do
        [ -d "$_tmpdir/$_dir_name" ] && cp -r "$_tmpdir/$_dir_name" "$_dir/"
    done

    rm -rf "$_tmpdir"
}

# ─── Install ───────────────────────────────────────────────────────────────────

do_portable_install() {
    _target="$1"

    # Check for existing installation
    if [ -f "$_target/$APP_NAME" ] || [ -f "$_target/${APP_NAME}.exe" ]; then
        info "existing V2RayDAR installation found at $_target"
        if confirm "update to latest version?"; then
            _tmpdata="$(backup_data "$_target")"

            _tmpdir="$(mktemp -d)"
            _archive="$_tmpdir/$ASSET"

            info "downloading ${ASSET}..."
            download_file "${GITHUB_DOWNLOAD}/v${VERSION}/${ASSET}" "$_archive"
            verify_checksum "$_archive"

            info "updating..."
            extract_archive "$_archive" "$_target"
            rm -rf "$_tmpdir"

            restore_data "$_target" "$_tmpdata"

            info "updated to v${VERSION}"
        else
            info "keeping current version"
            return
        fi
    else
        info "fresh install to $_target"
        mkdir -p "$_target"

        _tmpdir="$(mktemp -d)"
        _archive="$_tmpdir/$ASSET"

        info "downloading ${ASSET}..."
        download_file "${GITHUB_DOWNLOAD}/v${VERSION}/${ASSET}" "$_archive"
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
    info "run:  cd $_target && ./$APP_NAME --portable"
}

do_user_install() {
    _bin_dir="$1"

    if [ -f "$_bin_dir/$APP_NAME" ]; then
        info "existing V2RayDAR binary found at $_bin_dir/$APP_NAME"
        if confirm "update to latest version?"; then
            _tmpdir="$(mktemp -d)"
            _archive="$_tmpdir/$ASSET"

            info "downloading ${ASSET}..."
            download_file "${GITHUB_DOWNLOAD}/v${VERSION}/${ASSET}" "$_archive"
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

        _tmpdir="$(mktemp -d)"
        _archive="$_tmpdir/$ASSET"

        info "downloading ${ASSET}..."
        download_file "${GITHUB_DOWNLOAD}/v${VERSION}/${ASSET}" "$_archive"
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
    info "run:  $APP_NAME"
}

# ─── PATH Management ──────────────────────────────────────────────────────────

add_to_path() {
    _dir="$1"
    _shell_rc=""

    case "${SHELL:-}" in
        */bash) [ -f "$HOME/.bashrc" ] && _shell_rc="$HOME/.bashrc"
                [ -z "$_shell_rc" ] && _shell_rc="$HOME/.profile" ;;
        */zsh)  _shell_rc="$HOME/.zshrc" ;;
        */fish) _shell_rc="$HOME/.config/fish/config.fish" ;;
        *)      _shell_rc="$HOME/.profile" ;;
    esac

    [ -n "$_shell_rc" ] || return 1

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
    _detected_os="$OS"
    _detected_arch="$ARCH"
    [ "$IS_TERMUX" = "1" ] && _detected_os="termux"

    echo ""
    echo "  ========================================"
    echo "       V2RayDAR Installer v${VERSION}"
    echo "  ========================================"
    echo ""
    info "Detected: ${_detected_os} ${_detected_arch}"
    echo ""

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
    else
        printf '\033[1;36m?\033[0m Choose mode [1-2, default: 1]: '
        read -r CHOICE </dev/tty || CHOICE=""
        CHOICE="${CHOICE:-1}"
    fi

    case "$CHOICE" in
        1)
            INSTALL_MODE="portable"
            if [ "${NON_INTERACTIVE:-0}" = "1" ]; then
                INSTALL_DIR="${INSTALL_DIR:-$_portable_default}"
            else
                printf '\033[1;36m?\033[0m Install directory [%s]: ' "$_portable_default"
                read -r _input_dir </dev/tty || _input_dir=""
                INSTALL_DIR="${_input_dir:-$_portable_default}"
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

# ─── Main ──────────────────────────────────────────────────────────────────────

usage() {
    cat <<EOF
V2RayDAR Installer

Usage:
    curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh
    curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh -s -- --help

Options:
    -v, --version VERSION    Install a specific version (default: latest)
    -d, --dir DIR            Install to a specific directory (portable mode)
    -p, --portable           Install in portable mode (everything in one directory)
    -u, --user               Install in user mode (binary to ~/.local/bin)
    -y, --yes                Skip all confirmation prompts
    -h, --help               Show this help message
EOF
}

main() {
    VERSION=""
    INSTALL_DIR=""
    INSTALL_MODE=""
    NON_INTERACTIVE=0

    while [ $# -gt 0 ]; do
        case "$1" in
            -v|--version)  VERSION="$2"; shift 2 ;;
            -d|--dir)      INSTALL_DIR="$2"; INSTALL_MODE="portable"; shift 2 ;;
            -p|--portable) INSTALL_MODE="portable"; shift ;;
            -u|--user)     INSTALL_MODE="user"; shift ;;
            -y|--yes)      NON_INTERACTIVE=1; shift ;;
            -h|--help)     usage; exit 0 ;;
            *)             err "unknown option: $1 (use --help)" ;;
        esac
    done

    need curl
    need uname
    need mktemp

    detect_os
    detect_arch
    detect_termux

    [ -n "$VERSION" ] || VERSION="$(get_latest_version)"
    info "version: $VERSION"

    select_asset
    info "asset: $ASSET"

    # Interactive install if no mode specified
    if [ -z "$INSTALL_MODE" ]; then
        interactive_install
    fi

    # Default portable directory: Desktop if it exists, otherwise home
    if [ "$INSTALL_MODE" = "portable" ] && [ -z "$INSTALL_DIR" ]; then
        INSTALL_DIR="$HOME/V2RayDAR"
        [ -d "$HOME/Desktop" ] && INSTALL_DIR="$HOME/Desktop/V2RayDAR"
    fi

    # User install default
    if [ "$INSTALL_MODE" = "user" ] && [ -z "$INSTALL_DIR" ]; then
        INSTALL_DIR="$HOME/.local/bin"
    fi

    echo ""
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

    echo ""
    info "done!"
    echo ""
}

main "$@"
