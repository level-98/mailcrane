#!/bin/sh
# Builds mailcrane-ffi and generates the Kotlin and Swift bindings into
# core/target/bindings/<lang>. M0-08 will wrap this as `cargo xtask bindings`.
set -eu

core_dir=$(cd "$(dirname "$0")/.." && pwd)
cd "$core_dir"

target_dir=${CARGO_TARGET_DIR:-$core_dir/target}

cargo build -p mailcrane-ffi --features bindgen

case "$(uname -s)" in
    Darwin) lib="$target_dir/debug/libmailcrane_ffi.dylib" ;;
    *) lib="$target_dir/debug/libmailcrane_ffi.so" ;;
esac

for lang in kotlin swift; do
    cargo run -q -p mailcrane-ffi --features bindgen --bin uniffi-bindgen -- \
        generate --library "$lib" --language "$lang" \
        --out-dir "$target_dir/bindings/$lang"
done
