#!/usr/bin/env bash
set -euo pipefail

REPO="phalanx-engine/phalanx"
BINARY_NAME="phalanx-daemon"
INSTALL_DIR="${HOME}/.phalanx/bin"

BOLD="$(tput bold 2>/dev/null || echo '')"
GREEN="$(tput setaf 2 2>/dev/null || echo '')"
YELLOW="$(tput setaf 3 2>/dev/null || echo '')"
RED="$(tput setaf 1 2>/dev/null || echo '')"
RESET="$(tput sgr0 2>/dev/null || echo '')"

info() {
    printf "%b==>%b %s\n" "${GREEN}" "${RESET}" "$1"
}
warn() {
    printf "%b[WARNING]%b %s\n" "${YELLOW}" "${RESET}" "$1"
}
error() {
    printf "%b[ERROR]%b %s\n" "${RED}" "${RESET}" "$1" >&2
    exit 1
}

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "${OS}" in
        Linux)
            PLATFORM_OS="unknown-linux-gnu"
            ;;
        Darwin)
            PLATFORM_OS="apple-darwin"
            ;;
        *)
            error "Unsupported Operating System: ${OS}. Phalanx Daemon supports Linux and macOS."
            ;;
    esac

    case "${ARCH}" in
        x86_64|amd64)
            PLATFORM_ARCH="x86_64"
            ;;
        aarch64|arm64)
            PLATFORM_ARCH="aarch64"
            ;;
        *)
            error "Unsupported Architecture: ${ARCH}. Phalanx Daemon supports x86_64 and arm64/aarch64."
            ;;
    esac

    TARGET="${PLATFORM_ARCH}-${PLATFORM_OS}"
}

get_latest_version() {
    info "Querying latest release for target ${BOLD}${TARGET}${RESET}..."
    VERSION=$(curl -sSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' || true)

    if [ -z "${VERSION}" ]; then
        warn "Could not query GitHub Releases API (rate-limit or offline). Falling back to 'v0.2.0'."
        VERSION="v0.2.0"
    fi
}

install_binary() {
    mkdir -p "${INSTALL_DIR}"
    TARBALL="phalanx-daemon-${VERSION}-${TARGET}.tar.gz"
    DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${TARBALL}"

    TMP_DIR=$(mktemp -d)
    trap 'rm -rf "${TMP_DIR}"' EXIT

    info "Downloading ${DOWNLOAD_URL}..."
    if curl -sSLf "${DOWNLOAD_URL}" -o "${TMP_DIR}/${TARBALL}" 2>/dev/null; then
        info "Unpacking binary to ${INSTALL_DIR}..."
        tar -xzf "${TMP_DIR}/${TARBALL}" -C "${TMP_DIR}"
        mv "${TMP_DIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
        chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
    else
        warn "Pre-compiled release not found for ${TARGET}. Falling back to source build via cargo..."
        if command -v cargo >/dev/null 2>&1; then
            RUSTFLAGS="-C target-cpu=native" cargo install --git "https://github.com/${REPO}.git" --bin "${BINARY_NAME}" --root "${HOME}/.phalanx"
        else
            error "Cargo is not installed and pre-compiled asset could not be retrieved. Please install Rust (https://rustup.rs)."
        fi
    fi
}

configure_path() {
    PATH_LINE="export PATH=\"${INSTALL_DIR}:\$PATH\""

    configure_file() {
        local FILE="$1"
        if [ -f "${FILE}" ]; then
            if ! grep -qs "${INSTALL_DIR}" "${FILE}"; then
                printf "\n# Phalanx Daemon CLI\n%s\n" "${PATH_LINE}" >> "${FILE}"
                info "Added ~/.phalanx/bin to ${FILE}"
            fi
        fi
    }

    case "${SHELL:-}" in
        */zsh)
            configure_file "${HOME}/.zshrc"
            configure_file "${HOME}/.zprofile"
            ;;
        */bash)
            configure_file "${HOME}/.bashrc"
            configure_file "${HOME}/.bash_profile"
            configure_file "${HOME}/.profile"
            ;;
        *)
            configure_file "${HOME}/.profile"
            ;;
    esac
}

verify_installation() {
    export PATH="${INSTALL_DIR}:${PATH}"
    if ! command -v "${BINARY_NAME}" >/dev/null 2>&1; then
        error "Binary verification failed. ${INSTALL_DIR}/${BINARY_NAME} is not executable."
    fi

    cat << 'EOF'

    ____  __  _____    __    ___    _   ___  __
   / __ \/ / / /   |  / /   /   |  / | / / |/ /
  / /_/ / /_/ / /| | / /   / /| | /  |/ /|   / 
 / ____/ __  / ___ |/ /___/ ___ |/ /|  //   |  
/_/   /_/ /_/_/  |_/_____/_/  |_/_/ |_//_/|_|  
           IN-MEMORY REVM SIMULATION DAEMON
EOF

    printf "%bPhalanx Daemon %s successfully installed to: %b%s/%s%b\n" "${GREEN}" "${VERSION}" "${BOLD}" "${INSTALL_DIR}" "${BINARY_NAME}" "${RESET}"
    printf "Run %bphalanx-daemon --help%b or restart your terminal to begin.\n\n" "${BOLD}" "${RESET}"
}

main() {
    detect_platform
    get_latest_version
    install_binary
    configure_path
    verify_installation
}

main