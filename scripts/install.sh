#!/usr/bin/env bash
set -euo pipefail

REPO="neon-tasker/phalanx"
BINARY_NAME="phalanx-daemon"
ALIAS_NAME="phalanx"
INSTALL_DIR="${HOME}/.phalanx/bin"

BOLD="$(tput bold 2>/dev/null || echo '')"
GREEN="$(tput setaf 2 2>/dev/null || echo '')"
YELLOW="$(tput setaf 3 2>/dev/null || echo '')"
RED="$(tput setaf 1 2>/dev/null || echo '')"
RESET="$(tput sgr0 2>/dev/null || echo '')"

log_info() {
    printf "%b==>%b %s\n" "${GREEN}" "${RESET}" "$1"
}
log_warn() {
    printf "%b[WARN]%b %s\n" "${YELLOW}" "${RESET}" "$1"
}
log_error() {
    printf "%b[ERROR]%b %s\n" "${RED}" "${RESET}" "$1" >&2
    exit 1
}

detect_target() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "${OS}" in
        Linux)
            case "${ARCH}" in
                x86_64|amd64)
                    TARGET="x86_64-unknown-linux-musl"
                    ;;
                aarch64|arm64)
                    TARGET="aarch64-unknown-linux-musl"
                    ;;
                *)
                    log_error "Unsupported Linux architecture: ${ARCH}. Supported: x86_64, aarch64."
                    ;;
            esac
            ;;
        Darwin)
            case "${ARCH}" in
                arm64|aarch64)
                    TARGET="aarch64-apple-darwin"
                    ;;
                x86_64|amd64)
                    TARGET="x86_64-apple-darwin"
                    ;;
                *)
                    log_error "Unsupported macOS architecture: ${ARCH}."
                    ;;
            esac
            ;;
        *)
            log_error "Unsupported Operating System: ${OS}. For Windows, run install.ps1 via PowerShell."
            ;;
    esac
}

resolve_version() {
    if [ -n "${PHALANX_VERSION:-}" ]; then
        VERSION="${PHALANX_VERSION}"
        log_info "Using specified version: ${BOLD}${VERSION}${RESET}"
        return
    fi

    log_info "Resolving latest release tag from GitHub..."
    EFFECTIVE_URL=$(curl -fsSLI -o /dev/null -w "%{url_effective}" "https://github.com/${REPO}/releases/latest" || true)
    VERSION=$(echo "${EFFECTIVE_URL}" | awk -F'/' '{print $NF}')

    if [ -z "${VERSION}" ] || [ "${VERSION}" = "latest" ]; then
        log_warn "Header redirect probe failed. Falling back to GitHub releases API..."
        VERSION=$(curl -sSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' || true)
    fi

    if [ -z "${VERSION}" ]; then
        log_error "Unable to resolve release version. Set PHALANX_VERSION manually (e.g. export PHALANX_VERSION=v0.2.2)."
    fi

    log_info "Latest release target: ${BOLD}${VERSION}${RESET}"
}

download_and_verify() {
    TARBALL="phalanx-daemon-${VERSION}-${TARGET}.tar.gz"
    CHECKSUM_FILE="${TARBALL}.sha256"
    BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"

    TMP_DIR=$(mktemp -d)
    trap 'rm -rf "${TMP_DIR}"' EXIT

    log_info "Fetching payload: ${TARBALL}..."
    curl -sSLf "${BASE_URL}/${TARBALL}" -o "${TMP_DIR}/${TARBALL}" || \
        log_error "Failed to download ${BASE_URL}/${TARBALL}. Verify release existence."

    log_info "Fetching checksum: ${CHECKSUM_FILE}..."
    curl -sSLf "${BASE_URL}/${CHECKSUM_FILE}" -o "${TMP_DIR}/${CHECKSUM_FILE}" || \
        log_error "Failed to download checksum manifest."

    log_info "Verifying SHA256 cryptographic signature..."
    cd "${TMP_DIR}"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -c "${CHECKSUM_FILE}" >/dev/null 2>&1 || log_error "SHA256 signature verification failed! File may be corrupted."
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -c "${CHECKSUM_FILE}" >/dev/null 2>&1 || log_error "SHA256 signature verification failed! File may be corrupted."
    else
        log_warn "Neither sha256sum nor shasum found on host. Skipping checksum assertion."
    fi
    cd - >/dev/null

    mkdir -p "${INSTALL_DIR}"
    tar -xzf "${TMP_DIR}/${TARBALL}" -C "${TMP_DIR}"
    
    mv "${TMP_DIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

    ln -sf "${INSTALL_DIR}/${BINARY_NAME}" "${INSTALL_DIR}/${ALIAS_NAME}"
    log_info "Binaries successfully staged to: ${INSTALL_DIR}"
}

configure_shell_path() {
    EXPORT_CMD="export PATH=\"${INSTALL_DIR}:\$PATH\""

    update_rc() {
        local RC_FILE="$1"
        if [ -f "${RC_FILE}" ]; then
            if ! grep -qs "${INSTALL_DIR}" "${RC_FILE}"; then
                printf "\n# Phalanx Engine CLI\n%s\n" "${EXPORT_CMD}" >> "${RC_FILE}"
                log_info "Injected PATH configuration into ${RC_FILE}"
            fi
        fi
    }

    CURRENT_SHELL="$(basename "${SHELL:-bash}")"
    case "${CURRENT_SHELL}" in
        zsh)
            update_rc "${HOME}/.zshrc"
            update_rc "${HOME}/.zprofile"
            ;;
        bash)
            update_rc "${HOME}/.bashrc"
            update_rc "${HOME}/.bash_profile"
            update_rc "${HOME}/.profile"
            ;;
        *)
            update_rc "${HOME}/.profile"
            ;;
    esac
}

verify_execution() {
    export PATH="${INSTALL_DIR}:${PATH}"
    if ! command -v "${BINARY_NAME}" >/dev/null 2>&1; then
        log_error "Installation failed: ${INSTALL_DIR}/${BINARY_NAME} is not executable."
    fi

    cat << 'EOF'

    ____  __  _____    __    ___    _   ___  __
   / __ \/ / / /   |  / /   /   |  / | / / |/ /
  / /_/ / /_/ / /| | / /   / /| | /  |/ /|   / 
 / ____/ __  / ___ |/ /___/ ___ |/ /|  //   |  
/_/   /_/ /_/_/  |_/_____/_/  |_/_/ |_//_/|_|  
           IN-MEMORY REVM SIMULATION DAEMON
EOF

    printf "%bPhalanx Daemon %s (%s)%b successfully deployed!\n" "${GREEN}" "${VERSION}" "${TARGET}" "${RESET}"
    printf "Primary Binary: %b%s/%s%b\n" "${BOLD}" "${INSTALL_DIR}" "${BINARY_NAME}" "${RESET}"
    printf "CLI Alias:      %b%s/%s%b\n\n" "${BOLD}" "${INSTALL_DIR}" "${ALIAS_NAME}" "${RESET}"
    printf "To apply PATH immediately to current session:\n"
    printf "  %bexport PATH=\"%s:\$PATH\"%b\n\n" "${BOLD}" "${INSTALL_DIR}" "${RESET}"
    printf "Run %bphalanx --help%b or %bphalanx verify%b to begin.\n" "${BOLD}" "${RESET}" "${BOLD}" "${RESET}"
}

main() {
    detect_target
    resolve_version
    download_and_verify
    configure_shell_path
    verify_execution
}

main