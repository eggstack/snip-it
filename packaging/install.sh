#!/usr/bin/env bash
# Install released snip-it binaries, falling back to an exact-version Cargo
# build only when the selected host has no published binary asset.
set -euo pipefail

readonly INSTALL_GITHUB_BASE_DEFAULT="https://github.com/eggstack/snip-it/releases/download"
readonly INSTALL_CRATES_BASE_DEFAULT="https://crates.io/api/v1"
INSTALL_TEMP_DIRS=()

cleanup_temp_dirs() {
    local directory
    for directory in "${INSTALL_TEMP_DIRS[@]}"; do
        rm -rf "$directory"
    done
}

install_error() {
    echo "snip-it installer: $*" >&2
}

usage() {
    cat <<'EOF'
Usage: install.sh [--server|--both] [--version X.Y.Z]

Install the snp client by default. Use --server for snip-sync or --both for
both binaries. --version pins a single component; --both requires independent
crate versions and therefore cannot be combined with --version.
EOF
}

is_test_mode() {
    [[ "${SNP_INSTALL_TEST_MODE:-0}" == "1" ]]
}

github_base() {
    if is_test_mode && [[ -n "${SNP_INSTALL_GITHUB_BASE:-}" ]]; then
        printf '%s\n' "${SNP_INSTALL_GITHUB_BASE%/}"
    else
        printf '%s\n' "$INSTALL_GITHUB_BASE_DEFAULT"
    fi
}

crates_base() {
    if is_test_mode && [[ -n "${SNP_INSTALL_CRATES_API_BASE:-}" ]]; then
        printf '%s\n' "${SNP_INSTALL_CRATES_API_BASE%/}"
    else
        printf '%s\n' "$INSTALL_CRATES_BASE_DEFAULT"
    fi
}

target_for_unix() {
    local os="$1"
    local arch="$2"
    case "$os:$arch" in
        Linux:x86_64|Linux:amd64) printf '%s\n' 'x86_64-unknown-linux-gnu' ;;
        Linux:aarch64|Linux:arm64) printf '%s\n' 'aarch64-unknown-linux-gnu' ;;
        Linux:armv7l|Linux:armv7) printf '%s\n' 'armv7-unknown-linux-gnueabihf' ;;
        Darwin:x86_64|Darwin:amd64) printf '%s\n' 'x86_64-apple-darwin' ;;
        Darwin:arm64|Darwin:aarch64) printf '%s\n' 'aarch64-apple-darwin' ;;
        *) printf '%s\n' 'source-only' ;;
    esac
}

component_package() {
    case "$1" in
        snp) printf '%s\n' 'snip-it' ;;
        server) printf '%s\n' 'snip-sync' ;;
        *) install_error "unknown component '$1'"; return 2 ;;
    esac
}

component_binary() {
    case "$1" in
        snp) printf '%s\n' 'snp' ;;
        server) printf '%s\n' 'snip-sync' ;;
        *) install_error "unknown component '$1'"; return 2 ;;
    esac
}

component_tag() {
    local component="$1"
    local version="$2"
    case "$component" in
        snp) printf 'v%s\n' "$version" ;;
        server) printf 'snip-sync-v%s\n' "$version" ;;
        *) install_error "unknown component '$component'"; return 2 ;;
    esac
}

asset_filename() {
    local component="$1"
    local target="$2"
    printf '%s-%s\n' "$(component_binary "$component")" "$target"
}

release_asset_url() {
    local component="$1"
    local version="$2"
    local target="$3"
    local tag asset
    tag="$(component_tag "$component" "$version")"
    asset="$(asset_filename "$component" "$target")"
    printf '%s/%s/%s\n' "$(github_base)" "$tag" "$asset"
}

source_only_target() {
    [[ "$1" == 'source-only' || "$1" == 'armv7-unknown-linux-gnueabihf' ]]
}

validate_stable_version() {
    local version="$1"
    [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
        install_error "invalid stable crate version '$version'"
        return 1
    }
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || {
        install_error "required command '$1' is not on PATH"
        return 1
    }
}

download_http() {
    local url="$1"
    local destination="$2"
    local status

    if ! status="$(curl --silent --show-error --location --compressed \
        --connect-timeout 10 --max-time 60 --retry 2 --retry-delay 1 \
        --proto '=https,http' -o "$destination" -w '%{http_code}' "$url")"; then
        install_error "transport failure downloading $url"
        return 2
    fi
    case "$status" in
        200) return 0 ;;
        404) return 44 ;;
        400|401|403|408|429|500|501|502|503|504)
            install_error "HTTP $status downloading $url"
            return 1
            ;;
        *)
            install_error "unexpected HTTP $status downloading $url"
            return 1
            ;;
    esac
}

crate_version() {
    local package="$1"
    local metadata
    local version
    metadata="$(mktemp)"
    if ! download_http "$(crates_base)/crates/$package" "$metadata"; then
        rm -f "$metadata"
        install_error "could not read crates.io metadata for $package"
        return 1
    fi
    # crates.io exposes max_stable_version in the crate object. Keep the
    # parser deliberately narrow: installers accept only stable X.Y.Z values.
    version="$(sed -n 's/.*"max_stable_version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$metadata" | head -n 1)"
    [[ -n "$version" ]] || version="$(sed -n 's/.*"max_version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$metadata" | head -n 1)"
    if ! validate_stable_version "$version"; then
        rm -f "$metadata"
        return 1
    fi
    rm -f "$metadata"
    printf '%s\n' "$version"
}

checksum_file() {
    local file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" | awk '{print tolower($1)}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$file" | awk '{print tolower($1)}'
    else
        install_error "neither sha256sum nor shasum is available"
        return 1
    fi
}

verify_candidate() {
    local candidate="$1"
    local checksum="$2"
    local asset="$3"
    local expected name extra actual
    local line_count

    line_count="$(grep -cve '^[[:space:]]*$' "$checksum" || true)"
    [[ "$line_count" == 1 ]] || {
        install_error "malformed checksum sidecar for $asset"
        return 1
    }
    read -r expected name extra < <(tr -d '\r' < "$checksum" | sed '/^[[:space:]]*$/d')
    [[ "$expected" =~ ^[0-9a-fA-F]{64}$ && "$name" == "$asset" && -z "${extra:-}" ]] || {
        install_error "malformed checksum sidecar for $asset"
        return 1
    }
    actual="$(checksum_file "$candidate")"
    [[ "${actual,,}" == "${expected,,}" ]] || {
        install_error "SHA-256 mismatch for $asset"
        return 1
    }
    chmod +x "$candidate"
    verify_binary_identity "$candidate" "$asset"
}

verify_binary_identity() {
    local candidate="$1"
    local asset="$2"
    local identity
    if ! identity="$("$candidate" version 2>&1)"; then
        install_error "candidate '$asset' could not execute"
        return 1
    fi
    local expected_identity
    expected_identity="$(component_binary "${CURRENT_COMPONENT}") ${CURRENT_VERSION}"
    [[ "$identity" == "$expected_identity" ]] || {
        install_error "candidate identity '$identity' does not match '$expected_identity'"
        return 1
    }
}

build_cargo_candidate() {
    local component="$1"
    local version="$2"
    local root="$3"
    local package binary candidate
    package="$(component_package "$component")"
    binary="$(component_binary "$component")"
    require_command cargo || {
        install_error "no prebuilt binary exists for target '$CURRENT_TARGET'"
        install_error "install Rust/Cargo and run: cargo install $package --version '=$version' --locked"
        return 1
    }
    cargo install "$package" --version "=$version" --locked --root "$root" >&2
    candidate="$root/bin/$binary"
    [[ -x "$candidate" ]] || {
        install_error "Cargo did not produce $candidate"
        return 1
    }
    verify_binary_identity "$candidate" "$binary"
    printf '%s\n' "$candidate"
}

install_component() {
    local component="$1"
    local requested_version="${2:-}"
    local package binary version target asset temp candidate checksum destination
    package="$(component_package "$component")"
    binary="$(component_binary "$component")"
    version="$requested_version"
    if [[ -z "$version" ]]; then
        version="$(crate_version "$package")"
    else
        validate_stable_version "$version"
    fi
    CURRENT_COMPONENT="$component"
    CURRENT_VERSION="$version"
    target="$(target_for_unix "$(uname -s)" "$(uname -m)")"
    CURRENT_TARGET="$target"
    temp="$(mktemp -d)"
    INSTALL_TEMP_DIRS+=("$temp")

    candidate=''
    if source_only_target "$target"; then
        echo "$binary $version: target $target is source-only; using Cargo fallback."
        candidate="$(build_cargo_candidate "$component" "$version" "$temp/cargo-root")"
    else
        asset="$(asset_filename "$component" "$target")"
        candidate="$temp/$asset"
        checksum="$temp/$asset.sha256"
        if download_http "$(release_asset_url "$component" "$version" "$target")" "$candidate"; then
            if ! download_http "$(release_asset_url "$component" "$version" "$target").sha256" "$checksum"; then
                install_error "checksum download failed for $asset; refusing Cargo fallback"
                return 1
            fi
            verify_candidate "$candidate" "$checksum" "$asset"
        else
            local download_status=$?
            if [[ "$download_status" == 44 ]]; then
                echo "$binary $version: release asset $asset is unavailable; using Cargo fallback."
                candidate="$(build_cargo_candidate "$component" "$version" "$temp/cargo-root")"
            else
                install_error "release asset $asset was not downloaded; refusing Cargo fallback"
                return 1
            fi
        fi
    fi

    destination="$(destination_dir)"
    mkdir -p "$destination"
    install -m 0755 "$candidate" "$destination/$binary"
    echo "Installed $binary $version at $destination/$binary"
    if [[ "$destination" == "$HOME/.local/bin" ]] && ! path_contains "$destination"; then
        echo "Add $destination to PATH to run $binary directly."
    fi
    if [[ "$component" == server ]]; then
        post_install_server "$destination/$binary"
    fi
}

destination_dir() {
    destination_dir_for_identity "$(id -u)" "${HOME:?HOME is not set}"
}

destination_dir_for_identity() {
    local uid="$1"
    local home="$2"
    if [[ "$uid" == 0 ]]; then
        printf '%s\n' '/usr/local/bin'
    else
        printf '%s\n' "$home/.local/bin"
    fi
}

path_contains() {
    local wanted="$1"
    local entry
    IFS=: read -r -a entries <<< "${PATH:-}"
    for entry in "${entries[@]}"; do
        [[ "$entry" == "$wanted" ]] && return 0
    done
    return 1
}

invoking_user_home() {
    local user="$1"
    if command -v getent >/dev/null 2>&1; then
        getent passwd "$user" | awk -F: '{print $6}'
    else
        dscl . -read "/Users/$user" NFSHomeDirectory 2>/dev/null | awk '{print $2}'
    fi
}

run_server_as_invoking_user() {
    if [[ "$(id -u)" == 0 && -n "${SUDO_USER:-}" && "$SUDO_USER" != root ]]; then
        local user_home
        user_home="$(invoking_user_home "$SUDO_USER")"
        if [[ -n "$user_home" ]] && command -v runuser >/dev/null 2>&1; then
            runuser -u "$SUDO_USER" -- env HOME="$user_home" "$@"
            return
        fi
        install_error "could not safely run server setup as SUDO_USER=$SUDO_USER"
        install_error "run as $SUDO_USER: $*"
        return 1
    fi
    "$@"
}

post_install_server() {
    local server_binary="$1"
    if ! run_server_as_invoking_user "$server_binary" init --skip-cert; then
        echo "snip-sync binary installed, but layout initialization needs attention."
        return 0
    fi
    if "$server_binary" --help 2>&1 | grep -Eq '(^|[[:space:]])startup([[:space:]]|$)'; then
        if run_server_as_invoking_user "$server_binary" startup install; then
            echo "snip-sync startup registration completed."
        else
            echo "snip-sync installed, but startup registration was not completed."
            echo "Run: $server_binary startup install"
        fi
    else
        echo "snip-sync installed and initialized. Startup registration is provided by Plan 003."
        echo "Run when available: $server_binary startup install"
    fi
}

parse_args() {
    INSTALL_COMPONENTS=()
    INSTALL_VERSION=''
    INSTALL_HELP=0
    local component_selected=0
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --server)
                [[ "$component_selected" == 0 ]] || { install_error 'component selected more than once'; return 2; }
                INSTALL_COMPONENTS=(server); component_selected=1 ;;
            --both)
                [[ "$component_selected" == 0 ]] || { install_error 'component selected more than once'; return 2; }
                INSTALL_COMPONENTS=(snp server); component_selected=1 ;;
            --version)
                [[ $# -ge 2 ]] || { install_error '--version requires X.Y.Z'; return 2; }
                INSTALL_VERSION="$2"; shift ;;
            --version=*)
                INSTALL_VERSION="${1#*=}" ;;
            --help|-h)
                usage; INSTALL_HELP=1 ;;
            *)
                install_error "unknown argument '$1'"; usage >&2; return 2 ;;
        esac
        shift
    done
    [[ "$component_selected" == 1 ]] || INSTALL_COMPONENTS=(snp)
    if [[ "${#INSTALL_COMPONENTS[@]}" -gt 1 && -n "$INSTALL_VERSION" ]]; then
        install_error '--version is ambiguous with --both; use separate installs'
        return 2
    fi
    if [[ -n "$INSTALL_VERSION" ]]; then
        validate_stable_version "$INSTALL_VERSION" || return 2
    fi
}

main() {
    trap cleanup_temp_dirs EXIT
    parse_args "$@" || return $?
    [[ "$INSTALL_HELP" == 1 ]] && return 0
    require_command curl
    local component
    for component in "${INSTALL_COMPONENTS[@]}"; do
        if [[ "${#INSTALL_COMPONENTS[@]}" == 1 ]]; then
            install_component "$component" "$INSTALL_VERSION"
        else
            install_component "$component"
        fi
    done
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
