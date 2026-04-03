#!/usr/bin/env bash
# build.sh — multi-architecture release builder for `via`
#
# Supported targets:
#   linux-x86_64   x86_64-unknown-linux-musl   (static-pie, no glibc)
#   linux-aarch64  aarch64-unknown-linux-musl   (static-pie, no glibc)
#   macos-x86_64   x86_64-apple-darwin
#   macos-aarch64  aarch64-apple-darwin
#
# Usage:
#   ./build.sh                               # build all targets
#   ./build.sh linux-x86_64                  # one target
#   ./build.sh linux-x86_64 linux-aarch64    # multiple targets
#   ./build.sh --debug linux-x86_64          # debug profile
#
# Environment (optional; unset → sensible default):
#   VIA_MUSL_BIN          Directory with musl-cross *-linux-musl-gcc / *-ar
#   VIA_DIST_DIR          Output directory for copied binaries (default: ./dist)
#   CARGO_TARGET_*_LINKER / *_AR   Override per-target linker (default: under VIA_MUSL_BIN)
#   CARGO_PROFILE_RELEASE_*        Override release profile (default: LTO, strip, etc.)

set -euo pipefail

# musl-cross bin dir: VIA_MUSL_BIN wins; else Homebrew ARM / Intel heuristic
if [ -n "${VIA_MUSL_BIN:-}" ]; then
    MUSL_BIN="${VIA_MUSL_BIN}"
elif [ -d "/opt/homebrew/opt/musl-cross/bin" ]; then
    MUSL_BIN="/opt/homebrew/opt/musl-cross/bin"
elif [ -d "/usr/local/opt/musl-cross/bin" ]; then
    MUSL_BIN="/usr/local/opt/musl-cross/bin"
else
    MUSL_BIN="/opt/homebrew/opt/musl-cross/bin"
fi

DIST_DIR="${VIA_DIST_DIR:-$(cd "$(dirname "$0")" && pwd)/dist}"
ALL_TARGETS="linux-x86_64 linux-aarch64 macos-x86_64 macos-aarch64"

# ── helpers ───────────────────────────────────────────────────────────────────

bold() { printf '\033[1m%s\033[0m\n' "$*"; }
info() { printf '  \033[34m•\033[0m %s\n' "$*"; }
ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
skip() { printf '  \033[33m–\033[0m %s\n' "$*"; }
err()  { printf '  \033[31m✗\033[0m %s\n' "$*" >&2; }

alias_to_triple() {
    case "$1" in
        linux-x86_64)  echo "x86_64-unknown-linux-musl"  ;;
        linux-aarch64) echo "aarch64-unknown-linux-musl" ;;
        macos-x86_64)  echo "x86_64-apple-darwin"        ;;
        macos-aarch64) echo "aarch64-apple-darwin"       ;;
        *) echo ""; return 1 ;;
    esac
}

human_size() {
    local b="$1"
    if   [ "$b" -ge 1048576 ]; then awk "BEGIN{printf \"%.1f MB\", $b/1048576}"
    elif [ "$b" -ge 1024 ];    then awk "BEGIN{printf \"%.1f KB\", $b/1024}"
    else echo "${b} B"
    fi
}

check_musl_toolchain() {
    case "$1" in
        linux-x86_64)
            if [ ! -x "${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER}" ]; then
                err "x86_64 musl toolchain not found (linker: ${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER}); brew install musl-cross or set VIA_MUSL_BIN / CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER"
                return 1
            fi ;;
        linux-aarch64)
            if [ ! -x "${CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER}" ]; then
                err "aarch64 musl toolchain not found (linker: ${CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER}); brew install musl-cross or set VIA_MUSL_BIN / CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER"
                return 1
            fi ;;
    esac
    return 0
}

ensure_target() {
    local triple="$1"
    if ! rustup target list --installed | grep -q "^${triple}$"; then
        info "installing rustup target ${triple} …"
        rustup target add "${triple}"
    fi
}

# ── argument parsing ──────────────────────────────────────────────────────────

PROFILE="release"
BUILD_TARGETS=""

for arg in "$@"; do
    case "$arg" in
        --debug)   PROFILE="debug" ;;
        --release) PROFILE="release" ;;
        -*)
            err "unknown flag: $arg"
            exit 1 ;;
        *)
            if alias_to_triple "$arg" >/dev/null 2>&1; then
                BUILD_TARGETS="${BUILD_TARGETS} ${arg}"
            else
                err "unknown target: $arg  (valid: $ALL_TARGETS)"
                exit 1
            fi ;;
    esac
done

[ -z "$BUILD_TARGETS" ] && BUILD_TARGETS="$ALL_TARGETS"

# ── Cargo settings (same as repo .cargo/config.toml) via env ────────────────
# Pre-set CARGO_* or VIA_MUSL_BIN override; otherwise defaults below apply.

export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER:-${MUSL_BIN}/x86_64-linux-musl-gcc}"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_AR="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_AR:-${MUSL_BIN}/x86_64-linux-musl-ar}"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER="${CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER:-${MUSL_BIN}/aarch64-linux-musl-gcc}"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_AR="${CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_AR:-${MUSL_BIN}/aarch64-linux-musl-ar}"

if [ "$PROFILE" = "release" ]; then
    export CARGO_PROFILE_RELEASE_OPT_LEVEL="${CARGO_PROFILE_RELEASE_OPT_LEVEL:-3}"
    export CARGO_PROFILE_RELEASE_LTO="${CARGO_PROFILE_RELEASE_LTO:-true}"
    export CARGO_PROFILE_RELEASE_CODEGEN_UNITS="${CARGO_PROFILE_RELEASE_CODEGEN_UNITS:-1}"
    export CARGO_PROFILE_RELEASE_STRIP="${CARGO_PROFILE_RELEASE_STRIP:-true}"
fi

# ── version ───────────────────────────────────────────────────────────────────

VERSION=$(grep '^version' "$(dirname "$0")/Cargo.toml" \
    | head -1 | sed 's/.*= *"\(.*\)"/\1/')

# ── build loop ────────────────────────────────────────────────────────────────

mkdir -p "${DIST_DIR}"

bold ""
bold "via v${VERSION} — ${PROFILE}"
bold ""

BUILT=0; SKIPPED=0; FAILED=0

for alias in $BUILD_TARGETS; do
    triple=$(alias_to_triple "$alias")
    out="${DIST_DIR}/via-${alias}"

    echo "── ${alias}  (${triple})"

    case "$alias" in
        linux-*)
            if ! check_musl_toolchain "$alias"; then
                skip "skipped (missing toolchain)"
                SKIPPED=$((SKIPPED+1))
                echo ""
                continue
            fi ;;
    esac

    ensure_target "$triple"

    if [ "$PROFILE" = "release" ]; then
        cargo_flags="--release"
    else
        cargo_flags=""
    fi

    if cargo build $cargo_flags --target "$triple" 2>&1 | sed 's/^/    /'; then
        src="$(dirname "$0")/target/${triple}/${PROFILE}/via"
        cp "$src" "$out"
        size=$(stat -f%z "$out" 2>/dev/null || stat -c%s "$out")
        ok "$(printf '%-36s %s' "via-${alias}" "$(human_size "$size")")"
        BUILT=$((BUILT+1))
    else
        err "build failed for ${alias}"
        FAILED=$((FAILED+1))
    fi

    echo ""
done

# ── summary ───────────────────────────────────────────────────────────────────

bold "── dist"
for f in "${DIST_DIR}"/via-*; do
    [ -f "$f" ] || continue
    size=$(stat -f%z "$f" 2>/dev/null || stat -c%s "$f")
    info "$(printf '%-36s %s' "$(basename "$f")" "$(human_size "$size")")"
done

echo ""
bold "built: ${BUILT}  skipped: ${SKIPPED}  failed: ${FAILED}"
echo ""

[ "$FAILED" -eq 0 ]
