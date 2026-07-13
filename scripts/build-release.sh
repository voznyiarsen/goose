#!/usr/bin/env bash
# build-release.sh — Build release binaries for multiple targets.
#
# Usage:
#   scripts/build-release.sh all          # build all three targets
#   scripts/build-release.sh amd64        # x86_64-unknown-linux-gnu, default features
#   scripts/build-release.sh arm64        # aarch64-unknown-linux-gnu, portable-default
#   scripts/build-release.sh android      # aarch64-linux-android, reduced features
#
# Output: each binary is placed at:
#   release/goose-<target-triple>
#
# Prerequisites (auto-installed to ~/.cache/goose-build):
#   - Android NDK r27c       (for android target)
#   - zig 0.13.0             (for arm64 target, via cargo-zigbuild)
#   - cargo-zigbuild         (for arm64 target)

set -euo pipefail
IFS=$'\n\t'

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CACHE_DIR="${HOME}/.cache/goose-build"
export PATH="${CACHE_DIR}/bin:$PATH"
export RUSTC_WRAPPER="${RUSTC_WRAPPER:-sccache}"
: "${CARGO_BUILD_JOBS:=$(free -m | awk '/^Mem:/ {t=int($7/2048); print (t>0?t:1)}')}"
export CARGO_BUILD_JOBS

TARGET="${1:-}"
STRIP="${STRIP:-true}"
OUTPUT_DIR="${REPO_ROOT}/release"

# ── utilities ──────────────────────────────────────────────────────────

info()  { printf "\033[1;34m==>\033[0m %s\n" "$*"; }
ok()    { printf "\033[1;32m  OK\033[0m   %s\n" "$*"; }
warn()  { printf "\033[1;33m WARN\033[0m   %s\n" "$*" >&2; }
die()   { printf "\033[1;31mFAIL\033[0m   %s\n" "$*" >&2; exit 1; }

activate_hermit() {
  if [[ -f "${REPO_ROOT}/bin/activate-hermit" ]]; then
    # shellcheck disable=SC1091
    source "${REPO_ROOT}/bin/activate-hermit"
  fi
}

require_cmd() {
  if ! command -v "$1" &>/dev/null; then
    die "missing required command: $1"
  fi
}

# ── target detection ───────────────────────────────────────────────────

HOST_ARCH="$(uname -m)"
HOST_OS="$(uname -s)"

case "$HOST_OS" in
  Linux)  HOST_TARGET="${HOST_ARCH}-unknown-linux-gnu" ;;
  Darwin) HOST_TARGET="${HOST_ARCH}-apple-darwin" ;;
  *)      die "unsupported host OS: $HOST_OS" ;;
esac

# ── dependency setup ───────────────────────────────────────────────────

setup_cargo_zigbuild() {
  if command -v cargo-zigbuild &>/dev/null; then
    ok "cargo-zigbuild already installed"
    return
  fi
  info "installing cargo-zigbuild ..."
  cargo install cargo-zigbuild --locked
  ok "cargo-zigbuild installed"
}

setup_zig() {
  if command -v zig &>/dev/null; then
    ok "zig found at $(command -v zig)"
    return
  fi

  ZIG_VERSION="0.13.0"
  ARCH="$(uname -m)"
  ZIG_ARCH="unknown"
  case "$ARCH" in
    x86_64) ZIG_ARCH="x86_64" ;;
    aarch64|arm64) ZIG_ARCH="aarch64" ;;
    *) die "unsupported arch for zig: $ARCH" ;;
  esac

  ZIG_DIR="${CACHE_DIR}/zig-${ZIG_VERSION}"
  ZIG_BIN="${ZIG_DIR}/zig"

  if [[ -x "$ZIG_BIN" ]]; then
    ok "zig cached at $ZIG_BIN"
    export PATH="${ZIG_DIR}:$PATH"
    return
  fi

  info "downloading zig ${ZIG_VERSION} (${ZIG_ARCH}) ..."
  mkdir -p "$CACHE_DIR"
  ZIG_TAR="zig-linux-${ZIG_ARCH}-${ZIG_VERSION}.tar.xz"
  ZIG_URL="https://ziglang.org/download/${ZIG_VERSION}/${ZIG_TAR}"

  curl -fsSL --connect-timeout 10 --max-time 120 -o "${CACHE_DIR}/${ZIG_TAR}" "$ZIG_URL" || \
    die "failed to download zig from $ZIG_URL"
  tar -xf "${CACHE_DIR}/${ZIG_TAR}" -C "$CACHE_DIR" || die "failed to extract zig"
  mv "${CACHE_DIR}/zig-linux-${ZIG_ARCH}-${ZIG_VERSION}" "$ZIG_DIR" || die "failed to rename zig dir"
  rm -f "${CACHE_DIR}/${ZIG_TAR}"

  if [[ ! -x "$ZIG_BIN" ]]; then
    die "zig binary not found after extraction at $ZIG_BIN"
  fi

  export PATH="${ZIG_DIR}:$PATH"
  ok "zig ${ZIG_VERSION} ready at $ZIG_BIN"
}

setup_ndk() {
  NDK_DIR="${CACHE_DIR}/android-ndk-r27c"
  NDK_BIN="${NDK_DIR}/toolchains/llvm/prebuilt/linux-x86_64/bin"

  if [[ -d "$NDK_BIN" ]]; then
    ok "Android NDK r27c cached at $NDK_DIR"
    export NDK_HOME="$NDK_DIR"
    export NDK_TOOLCHAIN="$NDK_BIN"
    return
  fi

  info "downloading Android NDK r27c ..."
  mkdir -p "$CACHE_DIR"
  NDK_ZIP="${CACHE_DIR}/android-ndk-r27c-linux.zip"
  NDK_URL="https://dl.google.com/android/repository/android-ndk-r27c-linux.zip"

  curl -fsSL --connect-timeout 10 --max-time 600 -o "$NDK_ZIP" "$NDK_URL" || \
    die "failed to download NDK from $NDK_URL"
  unzip -q "$NDK_ZIP" -d "$CACHE_DIR" || die "failed to extract NDK"
  rm -f "$NDK_ZIP"

  if [[ ! -d "$NDK_BIN" ]]; then
    die "NDK toolchain not found after extraction at $NDK_BIN"
  fi

  export NDK_HOME="$NDK_DIR"
  export NDK_TOOLCHAIN="$NDK_BIN"
  ok "Android NDK r27c ready at $NDK_DIR"
}

setup_android_env() {
  export CC_aarch64_linux_android="${NDK_TOOLCHAIN}/aarch64-linux-android24-clang"
  export CXX_aarch64_linux_android="${NDK_TOOLCHAIN}/aarch64-linux-android24-clang++"
  export AR_aarch64_linux_android="${NDK_TOOLCHAIN}/llvm-ar"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${NDK_TOOLCHAIN}/aarch64-linux-android24-clang"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_AR="${NDK_TOOLCHAIN}/llvm-ar"
  export CARGO_BUILD_TARGET="aarch64-linux-android"
  export CFLAGS_aarch64_linux_android="-Dindex=strchr -D__USE_MISC"
}

# ── build targets ──────────────────────────────────────────────────────

check_amd64() {
  if [[ "$HOST_TARGET" != "x86_64-unknown-linux-gnu" ]]; then
    die "amd64 build requires an x86_64 linux host (detected: $HOST_TARGET)"
  fi
}

build_amd64() {
  info "building amd64 (full standard features) ..."
  activate_hermit
  cargo build --release -p goose-cli --bin goose
  [[ "$STRIP" == "true" ]] && strip target/release/goose
  mkdir -p "$OUTPUT_DIR"
  cp target/release/goose "${OUTPUT_DIR}/goose-x86_64-unknown-linux-gnu"
  ok "amd64 binary -> ${OUTPUT_DIR}/goose-x86_64-unknown-linux-gnu"
}

build_arm64() {
  info "building arm64 (portable-default + update) via cargo-zigbuild ..."
  activate_hermit
  require_cmd cargo-zigbuild
  rustup target add aarch64-unknown-linux-gnu
  cargo zigbuild --release -p goose-cli --bin goose \
    --target aarch64-unknown-linux-gnu \
    --no-default-features \
    --features portable-default,update
  if [[ "$STRIP" == "true" ]]; then
    # zigbuild already strips in --release, but ensure no debuginfo remains
    local tmpfile
    tmpfile="$(mktemp)"
    cp "target/aarch64-unknown-linux-gnu/release/goose" "$tmpfile"
    zig objcopy --strip-debug "$tmpfile" "$tmpfile" 2>/dev/null || true
    cp "$tmpfile" "target/aarch64-unknown-linux-gnu/release/goose"
    rm -f "$tmpfile"
  fi
  mkdir -p "$OUTPUT_DIR"
  cp "target/aarch64-unknown-linux-gnu/release/goose" \
     "${OUTPUT_DIR}/goose-aarch64-unknown-linux-gnu"
  ok "arm64 binary -> ${OUTPUT_DIR}/goose-aarch64-unknown-linux-gnu"
}

build_android() {
  info "building android arm64 ..."
  activate_hermit
  if [[ ! -x "${NDK_TOOLCHAIN}/aarch64-linux-android24-clang" ]]; then
    die "NDK toolchain not found at ${NDK_TOOLCHAIN}/aarch64-linux-android24-clang — is setup_ndk missing?"
  fi
  rustup target add aarch64-linux-android
  setup_android_env

  cargo build --release -p goose-cli --bin goose \
    --no-default-features \
    --features tui,aws-providers,telemetry,otel,rustls-tls,update

  if [[ "$STRIP" == "true" ]]; then
    "${NDK_TOOLCHAIN}/llvm-strip" target/aarch64-linux-android/release/goose
  fi

  mkdir -p "$OUTPUT_DIR"
  cp target/aarch64-linux-android/release/goose \
     "${OUTPUT_DIR}/goose-aarch64-linux-android"
  ok "android binary -> ${OUTPUT_DIR}/goose-aarch64-linux-android"
}

# ── main ───────────────────────────────────────────────────────────────

case "$TARGET" in
  all)
    build_amd64

    setup_cargo_zigbuild
    setup_zig
    build_arm64

    setup_ndk
    build_android
    ;;

  amd64)
    check_amd64
    build_amd64
    ;;

  arm64)
    setup_cargo_zigbuild
    setup_zig
    build_arm64
    ;;

  android)
    setup_ndk
    build_android
    ;;

  *)
    echo "Usage: $0 {all|amd64|arm64|android}"
    exit 1
    ;;
esac

echo ""
info "all done — binaries in ${OUTPUT_DIR}/"
ls -lh "$OUTPUT_DIR" 2>/dev/null | tail -n +2
