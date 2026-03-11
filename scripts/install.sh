#!/usr/bin/env bash
#
# Statespace CLI Installer
# https://github.com/statespace-tech/statespace
#
# Usage:
#   curl -fsSL https://statespace.com/install.sh | bash
#   STATESPACE_VERSION=0.1.0 curl -fsSL https://statespace.com/install.sh | bash
#

set -euo pipefail
umask 022

readonly REPO="statespace-tech/statespace"
readonly BINARY_NAME="statespace"
readonly GITHUB_API="https://api.github.com/repos/${REPO}/releases"
readonly GITHUB_RELEASES="https://github.com/${REPO}/releases/download"
readonly PATH_MARKER="# statespace PATH"
readonly DEBUG_LOG="${STATESPACE_DEBUG_LOG:-statespace-install-debug.log}"

if [[ -t 1 ]]; then
    readonly RED=$'\033[0;31m' GREEN=$'\033[0;32m' BLUE=$'\033[0;34m' BOLD=$'\033[1m' NC=$'\033[0m'
else
    readonly RED='' GREEN='' BLUE='' BOLD='' NC=''
fi

info() { printf '%s==>%s %s\n' "$BLUE" "$NC" "$1"; }

error() {
    printf '%s==>%s %s\n' "$RED" "$NC" "$1" >&2
    write_debug_log "$1"
    exit 1
}

write_debug_log() {
    local error_msg="$1"
    local log_path
    log_path="$(pwd)/$DEBUG_LOG"

    {
        printf '=== Statespace Install Failure ===\n'
        printf 'Date:    %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || date)"
        printf 'Error:   %s\n\n' "$error_msg"

        printf '--- Environment ---\n'
        printf 'OS:      %s\n' "$(uname -s 2>/dev/null || echo unknown)"
        printf 'Arch:    %s\n' "$(uname -m 2>/dev/null || echo unknown)"
        printf 'Kernel:  %s\n' "$(uname -r 2>/dev/null || echo unknown)"
        printf 'Shell:   %s\n' "${SHELL:-unknown}"
        printf 'Bash:    %s\n' "${BASH_VERSION:-unknown}"
        printf 'HOME:    %s\n' "${HOME:-unset}"
        printf 'USER:    %s\n' "${USER:-unset}"
        printf 'EUID:    %s\n' "${EUID:-$(id -u 2>/dev/null || echo unknown)}"
        printf 'umask:   %s\n' "$(umask)"
        printf 'PATH:    %s\n\n' "${PATH:-unset}"

        printf '--- Installer Config ---\n'
        printf 'STATESPACE_VERSION:     %s\n' "${STATESPACE_VERSION:-unset}"
        printf 'STATESPACE_INSTALL_DIR: %s\n' "${STATESPACE_INSTALL_DIR:-unset}"
        printf 'INSTALL_DIR:            %s\n' "${INSTALL_DIR:-unset}"
        printf 'BIN_DIR:                %s\n\n' "${BIN_DIR:-unset}"

        printf '--- Available Tools ---\n'
        for cmd in curl wget tar mktemp sha256sum shasum grep sed awk; do
            printf '%-12s %s\n' "$cmd:" "$(command -v "$cmd" 2>/dev/null || echo 'not found')"
        done
        printf '\n'

        if command -v curl &>/dev/null; then
            printf 'curl version: %s\n' "$(curl --version 2>/dev/null | head -1)"
        fi
        if command -v wget &>/dev/null; then
            printf 'wget version: %s\n' "$(wget --version 2>/dev/null | head -1)"
        fi

        if [[ -f /etc/os-release ]]; then
            printf '\n--- OS Release ---\n'
            cat /etc/os-release
        fi

        printf '\n--- Disk Space ---\n'
        df -h "${INSTALL_DIR:-$HOME}" 2>/dev/null || printf 'unavailable\n'
    } > "$log_path" 2>/dev/null || return 0

    printf '%s==>%s Debug log written to: %s\n' "$BLUE" "$NC" "$log_path" >&2
    printf '    Include this file when reporting issues at:\n' >&2
    printf '    https://github.com/%s/issues\n' "$REPO" >&2
}

# --- Validation ---

validate_env() {
    : "${HOME:?HOME must be set}"
    [[ "$HOME" = /* ]] || error "HOME must be an absolute path"
    [[ -d "$HOME" ]] || error "HOME directory does not exist: $HOME"

    if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
        error "do not run this installer as root — install per-user instead"
    fi
}

validate_abs_path() {
    local path="$1" name="$2"
    [[ "$path" = /* ]] || error "$name must be an absolute path"
    [[ "$path" != *$'\n'* && "$path" != *$'\r'* ]] || error "$name contains invalid characters"
    [[ "$path" != *:* ]] || error "$name cannot contain ':' (breaks PATH)"
}

validate_version() {
    local v="$1"
    [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.a-zA-Z0-9_-]*)?$ ]] || \
        error "invalid version format: $v"
}

validate_install_dir() {
    local dir="$1"
    if [[ -e "$dir" && -L "$dir" ]]; then
        error "install directory is a symlink (security risk): $dir"
    fi
}

# --- Platform Detection ---

detect_target() {
    local os arch

    case "$(uname -s)" in
        Darwin) os="apple-darwin" ;;
        Linux) os="unknown-linux-musl" ;;
        MINGW*|MSYS*|CYGWIN*|Windows_NT)
            error "Windows is not supported. Use WSL: https://learn.microsoft.com/en-us/windows/wsl/install"
            ;;
        *) error "unsupported operating system: $(uname -s)" ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64) arch="x86_64" ;;
        arm64|aarch64) arch="aarch64" ;;
        *) error "unsupported architecture: $(uname -m)" ;;
    esac

    printf '%s-%s' "$arch" "$os"
}

# --- Requirements ---

require_cmd() {
    command -v "$1" &>/dev/null || error "'$1' is required but not found"
}

check_requirements() {
    require_cmd tar
    require_cmd mktemp

    if ! command -v curl &>/dev/null && ! command -v wget &>/dev/null; then
        error "curl or wget is required"
    fi

    if ! command -v sha256sum &>/dev/null && ! command -v shasum &>/dev/null; then
        error "sha256sum or shasum is required for checksum verification"
    fi
}

# --- HTTP ---

fetch() {
    local url="$1"
    if command -v curl &>/dev/null; then
        curl --proto '=https' --tlsv1.2 --retry 3 --retry-delay 1 \
            --connect-timeout 10 --max-time 120 -fsSL "$url"
    else
        wget --https-only --secure-protocol=TLSv1_2 \
            --timeout=10 --tries=3 --waitretry=1 -qO- "$url"
    fi
}

download() {
    local url="$1" dest="$2"
    if command -v curl &>/dev/null; then
        curl --proto '=https' --tlsv1.2 --retry 3 --retry-delay 1 \
            --connect-timeout 10 --max-time 300 -fsSL "$url" -o "$dest"
    else
        wget --https-only --secure-protocol=TLSv1_2 \
            --timeout=10 --tries=3 --waitretry=1 -q "$url" -O "$dest"
    fi
}

# --- Version ---

get_latest_version() {
    local response
    response=$(fetch "$GITHUB_API") || error "failed to fetch releases from GitHub API"

    local version
    version=$(printf '%s' "$response" | grep -o '"tag_name"[[:space:]]*:[[:space:]]*"v[^"]*"' | head -1 | \
        sed 's/.*"v\([^"]*\)".*/\1/')

    [[ -n "$version" ]] || error "no CLI releases found"
    printf '%s' "$version"
}

# --- Checksum ---

compute_sha256() {
    local file="$1"
    if command -v sha256sum &>/dev/null; then
        sha256sum "$file" | cut -d' ' -f1
    else
        shasum -a 256 "$file" | cut -d' ' -f1
    fi
}

verify_checksum() {
    local file="$1" expected="$2"
    local actual
    actual=$(compute_sha256 "$file")

    if [[ "$actual" != "$expected" ]]; then
        error "checksum verification failed
  expected: $expected
  actual:   $actual

This could indicate a corrupted download or a security issue.
Please report this at: https://github.com/${REPO}/issues"
    fi
}

# --- PATH ---

path_contains_bin_dir() {
    local d="$1" p
    IFS=':' read -r -a parts <<< "${PATH:-}"
    for p in "${parts[@]}"; do
        [[ "$p" == "$d" ]] && return 0
    done
    return 1
}

detect_shell_config() {
    local shell_name
    shell_name=$(basename "${SHELL:-bash}")

    case "$shell_name" in
        bash)
            if [[ -f "$HOME/.bash_profile" ]]; then
                printf '%s' "$HOME/.bash_profile"
            else
                printf '%s' "$HOME/.bashrc"
            fi
            ;;
        zsh)       printf '%s' "$HOME/.zshrc" ;;
        fish)      printf '%s' "$HOME/.config/fish/config.fish" ;;
        nu|nushell) printf '%s' "$HOME/.config/nushell/env.nu" ;;
        *)         printf '%s' "$HOME/.profile" ;;
    esac
}

path_export_line() {
    local shell_name bin_dir_escaped
    shell_name=$(basename "${SHELL:-bash}")
    bin_dir_escaped=$(printf "'%s'" "${BIN_DIR//\'/\'\\\'\'}")

    case "$shell_name" in
        fish)      printf 'fish_add_path %s' "$bin_dir_escaped" ;;
        nu|nushell) printf '$env.PATH = ($env.PATH | prepend %s)' "$bin_dir_escaped" ;;
        *)         printf 'export PATH=%s:$PATH' "$bin_dir_escaped" ;;
    esac
}

print_path_instructions() {
    local config_file export_line
    config_file=$(detect_shell_config)
    export_line=$(path_export_line)

    printf '\nTo add statespace to your PATH, add this to %s:\n\n' "$config_file"
    printf '  %s\n' "$export_line"
    printf '\nThen restart your shell, or run now with:\n  %s/%s --help\n' "$BIN_DIR" "$BINARY_NAME"
}

setup_path() {
    if path_contains_bin_dir "$BIN_DIR"; then
        return 0
    fi

    local config_file export_line
    config_file=$(detect_shell_config)
    export_line=$(path_export_line)

    if [[ ! -t 0 ]]; then
        print_path_instructions
        return 0
    fi

    printf '\nstatespace is not in your PATH.\n'
    printf 'Add it to %s%s%s? [y/N] ' "$BOLD" "$config_file" "$NC"
    read -r answer </dev/tty

    case "$answer" in
        [yY]|[yY][eE][sS])
            if [[ -f "$config_file" ]] && grep -qF "$PATH_MARKER" "$config_file" 2>/dev/null; then
                info "PATH entry already exists in $config_file"
                return 0
            fi
            local config_dir
            config_dir=$(dirname -- "$config_file")
            mkdir -p -- "$config_dir"
            printf '\n%s %s\n' "$export_line" "$PATH_MARKER" >> "$config_file"
            info "added to $config_file"
            printf '\nRestart your shell to pick up the change.\n'
            ;;
        *)
            print_path_instructions
            ;;
    esac
}

# --- Main ---

main() {
    printf '%sStatespace CLI Installer%s\n\n' "$BOLD" "$NC"

    validate_env
    check_requirements

    local install_dir="${STATESPACE_INSTALL_DIR:-$HOME/.statespace}"
    validate_abs_path "$install_dir" "STATESPACE_INSTALL_DIR"
    validate_install_dir "$install_dir"

    readonly INSTALL_DIR="$install_dir"
    readonly BIN_DIR="$INSTALL_DIR/bin"

    local target version
    target=$(detect_target)
    info "detected platform: $target"

    if [[ -n "${STATESPACE_VERSION:-}" ]]; then
        version="$STATESPACE_VERSION"
    else
        version=$(get_latest_version)
    fi
    validate_version "$version"
    info "installing version: $version"

    TMP_DIR=$(mktemp -d) || error "failed to create temp directory"
    chmod 700 -- "$TMP_DIR"
    trap 'rm -rf -- "$TMP_DIR"' EXIT

    local archive_name="${BINARY_NAME}-v${version}-${target}.tar.gz"
    local archive_path="$TMP_DIR/$archive_name"
    local checksum_path="$TMP_DIR/${archive_name}.sha256"
    local base_url="${GITHUB_RELEASES}/v${version}"

    info "fetching checksum..."
    download "$base_url/${archive_name}.sha256" "$checksum_path" || \
        error "failed to download checksum file"
    local expected_checksum
    expected_checksum=$(tr -d '\r' < "$checksum_path" | awk 'NF { print $1; exit }')
    [[ "$expected_checksum" =~ ^[0-9a-fA-F]{64}$ ]] || error "invalid checksum format in checksum file"

    info "downloading $archive_name..."
    download "$base_url/$archive_name" "$archive_path" || \
        error "download failed (version $version may not support $target)"

    info "verifying checksum..."
    verify_checksum "$archive_path" "$expected_checksum"

    local expected_member="${BINARY_NAME}-v${version}-${target}/${BINARY_NAME}"
    info "extracting..."
    tar -tzf "$archive_path" | grep -Fxq -- "$expected_member" || \
        error "unexpected archive layout — expected member: $expected_member"

    # Atomic install: write to temp file in BIN_DIR then rename, so a partial
    # write (crash, disk full) never leaves a broken binary at the final path.
    install -d -m 0755 -- "$INSTALL_DIR" "$BIN_DIR"
    validate_install_dir "$BIN_DIR"

    local tmp_bin
    tmp_bin=$(mktemp "$BIN_DIR/.${BINARY_NAME}.tmp.XXXXXX") || error "failed to create temp file in $BIN_DIR"
    trap 'rm -f -- "$tmp_bin"; rm -rf -- "$TMP_DIR"' EXIT

    tar -xOzf "$archive_path" -- "$expected_member" > "$tmp_bin" || {
        rm -f -- "$tmp_bin"
        error "failed to extract binary"
    }
    chmod 0755 -- "$tmp_bin"
    mv -f -- "$tmp_bin" "$BIN_DIR/$BINARY_NAME" || error "failed to install binary"

    trap 'rm -rf -- "$TMP_DIR"' EXIT

    ln -sfn -- "$BINARY_NAME" "$BIN_DIR/s2"

    info "installed to $BIN_DIR/$BINARY_NAME"

    printf '\n%sInstalled successfully!%s\n' "$GREEN" "$NC"

    setup_path
}

main "$@"
