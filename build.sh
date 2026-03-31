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

set -euo pipefail

MUSL_BIN="/opt/homebrew/opt/musl-cross/bin"
DIST_DIR="$(cd "$(dirname "$0")" && pwd)/dist"
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
            if [ ! -x "${MUSL_BIN}/x86_64-linux-musl-gcc" ]; then
                err "x86_64 musl toolchain not found: brew install musl-cross"
                return 1
            fi ;;
        linux-aarch64)
            if [ ! -x "${MUSL_BIN}/aarch64-linux-musl-gcc" ]; then
                err "aarch64 musl toolchain not found: brew install musl-cross"
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
