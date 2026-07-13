# Android Build Instructions

This document covers building the `goose` binary for Android (ARM64) using the Android NDK.

## Prerequisites

- **Linux host** (x86_64) — Android builds require a Linux host
- **Rust toolchain** with `aarch64-linux-android` target
- **Android NDK r27c** (automatically downloaded by the build script)
- **Hermit** environment for reproducible builds (optional but recommended)

## Quick Start

```bash
# Activate Hermit environment (recommended)
source bin/activate-hermit

# Build Android ARM64 binary
./scripts/build.sh android

# Output binary location
ls -lh release/goose-aarch64-linux-android
```

## Build Script Options

The build script (`scripts/build.sh`) supports multiple targets:

```bash
./scripts/build.sh android     # Build Android ARM64 only
./scripts/build.sh all         # Build amd64, arm64, and android
./scripts/build.sh amd64       # Linux x86_64 only
./scripts/build.sh arm64       # Linux ARM64 (via cargo-zigbuild)
```

## Android Build Details

### Features Enabled

The Android build uses a specific feature subset optimized for mobile:

```toml
--features tui,aws-providers,telemetry,otel,rustls-tls,update
```

**Excluded features** (not compatible with Android):
- `code-mode` — requires local code execution
- `local-inference` — requires llama.cpp (no Android build)
- `nostr` — network protocol dependencies
- `system-keyring` — no Android keyring implementation

### Build Configuration

The build script configures these environment variables for the Android target:

```bash
export CC_aarch64_linux_android="${NDK_TOOLCHAIN}/aarch64-linux-android24-clang"
export CXX_aarch64_linux_android="${NDK_TOOLCHAIN}/aarch64-linux-android24-clang++"
export AR_aarch64_linux_android="${NDK_TOOLCHAIN}/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${NDK_TOOLCHAIN}/aarch64-linux-android24-clang"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_AR="${NDK_TOOLCHAIN}/llvm-ar"
export CARGO_BUILD_TARGET="aarch64-linux-android"
export CFLAGS_aarch64_linux_android="-Dindex=strchr -D__USE_MISC"
export CARGO_BUILD_TARGET="aarch64-linux-android"
```

### NDK Management

The script automatically downloads and caches Android NDK r27c:

```bash
# Cache location
~/.cache/goose/android-ndk-r27c/

# Toolchain path
~/.cache/goose/android-ndk-r27c/toolchains/llvm/prebuilt/linux-x86_64/bin/
```

## Manual Build (Without Build Script)

If you need to build manually or integrate into another build system:

```bash
# Install Android target
rustup target add aarch64-linux-android

# Setup NDK (or use pre-installed)
export NDK_HOME=/path/to/android-ndk-r27c
export NDK_TOOLCHAIN="${NDK_HOME}/toolchains/llvm/prebuilt/linux-x86_64/bin"

# Setup Android environment
export CC_aarch64_linux_android="${NDK_TOOLCHAIN}/aarch64-linux-android24-clang"
export CXX_aarch64_linux_android="${NDK_TOOLCHAIN}/aarch64-linux-android24-clang++"
export AR_aarch64_linux_android="${NDK_TOOLCHAIN}/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${NDK_TOOLCHAIN}/aarch64-linux-android24-clang"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_AR="${NDK_TOOLCHAIN}/llvm-ar"
export CARGO_BUILD_TARGET="aarch64-linux-android"
export CFLAGS_aarch64_linux_android="-Dindex=strchr -D__USE_MISC"

# Build
cargo build --release -p goose-cli --bin goose \
  --target aarch64-linux-android \
  --no-default-features \
  --features tui,aws-providers,telemetry,otel,rustls-tls,update

# Optional: Strip binary
${NDK_TOOLCHAIN}/llvm-strip target/aarch64-linux-android/release/goose
```

## Running on Android

### Requirements

- Android 7.0+ (API 24+)
- ARM64 device (aarch64)
- Termux or ADB shell access

### Installation

```bash
# Via ADB
adb push release/goose-aarch64-linux-android /data/local/tmp/goose
adb shell chmod +x /data/local/tmp/goose

# Or via Termux
cp release/goose-aarch64-linux-android $PREFIX/bin/goose
chmod +x $PREFIX/bin/goose
```

### Running

```bash
# First-time setup (requires internet for provider config)
goose configure

# Run TUI
goose tui

# Or run a prompt directly
goose run "your prompt here"
```

## Troubleshooting

### "NDK toolchain not found"

```bash
# Force re-download
rm -rf ~/.cache/goose/android-ndk-r27c
./scripts/build.sh android
```

### Linker errors for `index`/`strchr`

The `CFLAGS_aarch64_linux_android="-Dindex=strchr -D__USE_MISC"` flag resolves missing `index` symbol (Android uses `strchr`).

### "failed to run custom build command for `ring`"

Ensure you're using the NDK's clang (not system clang). The `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER` must point to the NDK clang.

### TUI doesn't render correctly

Ensure you're running in a proper terminal emulator (Termux, ADB shell with `-t` flag). Raw VT100/ANSI support required.

### Provider configuration fails

Android's certificate store differs from Linux. You may need to configure providers manually or ensure `webpki-roots` feature is enabled in `rustls-tls`.

## CI/CD Integration

### GitHub Actions Example

```yaml
- name: Build Android
  run: |
    source bin/activate-hermit
    ./scripts/build.sh android
  env:
    CARGO_TERM_COLOR: always
```

### Artifact

The build outputs:
- `release/goose-aarch64-linux-android` — stripped ARM64 binary (~15-20 MB)

## Architecture Notes

| Aspect | Details |
|--------|---------|
| Target triple | `aarch64-linux-android` |
| Minimum API | 24 (Android 7.0) |
| Toolchain | NDK r27c clang 17+ |
| Stdlib | Android Bionic libc |
| TLS | rustls with webpki-roots |
| Terminal | VT100/ANSI via crossterm |

## File Structure

```
release/
└── goose-aarch64-linux-android    # Android ARM64 binary
```

## Security Notes

- Binary is stripped by default (removes debug symbols)
- No network permissions beyond what providers require
- Runs in user-space (no root required)
- Provider API keys stored in app-private config directory