#!/usr/bin/env bash
# build-ios.sh — Build the scribe-client-ffi XCFramework and generate Swift bindings.
#
# Usage (from the scribe-api-client workspace root):
#   ./scripts/build-ios.sh
#
# Outputs:
#   output/ScribeClientFFI.xcframework   — link in your Xcode project
#   output/ScribeClientFFI.swift         — add to your Xcode target sources
#
# After the first run, open scribe-ios/ScribeApp.xcodeproj in Xcode.
# The project already references these output paths.

set -euo pipefail

WORKSPACE_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_DIR="$WORKSPACE_DIR/target"
OUTPUT_DIR="$WORKSPACE_DIR/output"
IOS_PROJECT_DIR="$WORKSPACE_DIR/../scribe-ios/ScribeApp/Generated"

# Targets we build for
TARGET_DEVICE="aarch64-apple-ios"
TARGET_SIM="aarch64-apple-ios-sim"
LIB_NAME="libscribe_client_ffi.a"
CRATE="scribe-client-ffi"

echo "==> Adding Rust targets..."
rustup target add "$TARGET_DEVICE" "$TARGET_SIM" 2>/dev/null || true

echo "==> Building for $TARGET_DEVICE (device)..."
cargo build --release -p "$CRATE" --target "$TARGET_DEVICE"

echo "==> Building for $TARGET_SIM (simulator)..."
cargo build --release -p "$CRATE" --target "$TARGET_SIM"

# Generate Swift bindings from the device binary (architecture-independent)
DEVICE_LIB="$TARGET_DIR/$TARGET_DEVICE/release/$LIB_NAME"
HEADERS_DIR="$OUTPUT_DIR/uniffi-headers"
SWIFT_BINDINGS_SRC="$OUTPUT_DIR/uniffi-swift"

echo "==> Generating Swift bindings..."
mkdir -p "$HEADERS_DIR" "$SWIFT_BINDINGS_SRC"
cargo run -p uniffi-bindgen -- generate \
    --library "$DEVICE_LIB" \
    --language swift \
    --out-dir "$SWIFT_BINDINGS_SRC"

# uniffi-bindgen generates: scribe_client_ffi.swift, scribe_client_ffiFFI.h, scribe_client_ffiFFI.modulemap
# Copy and rename the header and modulemap into the headers dir for the xcframework
cp "$SWIFT_BINDINGS_SRC/scribe_client_ffiFFI.h"         "$HEADERS_DIR/scribe_client_ffiFFI.h"
cp "$SWIFT_BINDINGS_SRC/scribe_client_ffiFFI.modulemap" "$HEADERS_DIR/module.modulemap"

echo "==> Creating XCFramework..."
rm -rf "$OUTPUT_DIR/ScribeClientFFI.xcframework"
xcodebuild -create-xcframework \
    -library "$TARGET_DIR/$TARGET_DEVICE/release/$LIB_NAME" \
    -headers "$HEADERS_DIR" \
    -library "$TARGET_DIR/$TARGET_SIM/release/$LIB_NAME" \
    -headers "$HEADERS_DIR" \
    -output "$OUTPUT_DIR/ScribeClientFFI.xcframework"

echo "==> Copying Swift bindings to iOS project..."
mkdir -p "$IOS_PROJECT_DIR"
cp "$SWIFT_BINDINGS_SRC/scribe_client_ffi.swift" "$IOS_PROJECT_DIR/ScribeClientFFI.swift"

echo ""
echo "Done."
echo "  XCFramework : $OUTPUT_DIR/ScribeClientFFI.xcframework"
echo "  Swift file  : $IOS_PROJECT_DIR/ScribeClientFFI.swift"
echo ""
echo "In Xcode:"
echo "  1. If first run: drag ScribeClientFFI.xcframework into the project (Do Not Embed)."
echo "     The project file already has the reference at the expected relative path."
echo "  2. Build and run."
