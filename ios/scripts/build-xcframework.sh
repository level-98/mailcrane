#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
IOS_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
ROOT=$(CDPATH= cd -- "$IOS_DIR/.." && pwd)
CORE_DIR="$ROOT/core"
BUILD_DIR="$IOS_DIR/build"
XCFRAMEWORK="$BUILD_DIR/MailcraneFFI.xcframework"
HEADERS="$BUILD_DIR/Headers"
DEVICE_LIB="$BUILD_DIR/libmailcrane_ffi-device.a"
SIM_ARM_LIB="$BUILD_DIR/libmailcrane_ffi-sim-arm64.a"
SIM_X86_LIB="$BUILD_DIR/libmailcrane_ffi-sim-x86_64.a"
SIM_LIB="$BUILD_DIR/libmailcrane_ffi-simulator.a"

rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios

cd "$CORE_DIR"
cargo build -p mailcrane-ffi --lib --target aarch64-apple-ios
cargo build -p mailcrane-ffi --lib --target aarch64-apple-ios-sim
cargo build -p mailcrane-ffi --lib --target x86_64-apple-ios

mkdir -p "$HEADERS" "$IOS_DIR/MailcraneCore/Sources/MailcraneCore"
rm -rf "$XCFRAMEWORK" "$DEVICE_LIB" "$SIM_ARM_LIB" "$SIM_X86_LIB" "$SIM_LIB"

DEVICE_SOURCE="$CORE_DIR/target/aarch64-apple-ios/debug/libmailcrane_ffi.a"
SIM_ARM_SOURCE="$CORE_DIR/target/aarch64-apple-ios-sim/debug/libmailcrane_ffi.a"
SIM_X86_SOURCE="$CORE_DIR/target/x86_64-apple-ios/debug/libmailcrane_ffi.a"
cp "$DEVICE_SOURCE" "$DEVICE_LIB"
cp "$SIM_ARM_SOURCE" "$SIM_ARM_LIB"
cp "$SIM_X86_SOURCE" "$SIM_X86_LIB"
lipo -create "$SIM_ARM_LIB" "$SIM_X86_LIB" -output "$SIM_LIB"

BINDINGS="$BUILD_DIR/bindings"
rm -rf "$BINDINGS"
mkdir -p "$BINDINGS"
cargo run -p mailcrane-ffi --features bindgen --bin uniffi-bindgen -- \
  generate --library "$DEVICE_SOURCE" --language swift --out-dir "$BINDINGS"

SWIFT_FILE=$(find "$BINDINGS" -name '*.swift' -print -quit)
HEADER_FILE=$(find "$BINDINGS" -name '*.h' -print -quit)
if [ -z "$SWIFT_FILE" ] || [ -z "$HEADER_FILE" ]; then
  echo "UniFFI did not produce Swift and C header files" >&2
  exit 1
fi
cp "$SWIFT_FILE" "$IOS_DIR/MailcraneCore/Sources/MailcraneCore/MailcraneCore.swift"
cp "$HEADER_FILE" "$HEADERS/MailcraneFFI.h"
cat > "$HEADERS/module.modulemap" <<'EOF'
module MailcraneFFI {
    header "MailcraneFFI.h"
    export *
}
EOF

rm -rf "$XCFRAMEWORK"
xcodebuild -create-xcframework \
  -library "$DEVICE_LIB" -headers "$HEADERS" \
  -library "$SIM_LIB" -headers "$HEADERS" \
  -output "$XCFRAMEWORK"
