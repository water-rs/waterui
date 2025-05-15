#!/bin/bash
set -e

MODULE_NAME="waterui"
RUST_LIB_DIR="../target/debug"
OUT_DIR="out"
SWIFT_OUT_DIR="swift_build"

echo "🔧 Step 1: Building Rust dynamic library (debug)..."
cargo build

echo "📦 Step 2: Generating Swift bindings..."
cd bindgen
cargo run --bin uniffi-bindgen generate \
  --library $RUST_LIB_DIR/lib${MODULE_NAME}.dylib \
  --language swift \
  --out-dir $OUT_DIR

echo "📁 Step 3: Preparing Swift output directory..."
mkdir -p "$SWIFT_OUT_DIR"

echo "🛠️ Step 4: Compiling Swift module..."
SWIFT_FILES=$(find "$OUT_DIR" -name '*.swift')
MODULEMAPS=$(find "$OUT_DIR" -name '*.modulemap' | sed 's/^/-Xcc -fmodule-map-file=/')

swiftc \
  -module-name "$MODULE_NAME" \
  -emit-library -o "$SWIFT_OUT_DIR/lib${MODULE_NAME}_swift.dylib" \
  -emit-module -emit-module-path "$SWIFT_OUT_DIR" \
  -parse-as-library \
  -L "$RUST_LIB_DIR" \
  -l"$MODULE_NAME" \
  $MODULEMAPS \
  $SWIFT_FILES

echo "✅ Done! All Swift outputs are in bindgen/$SWIFT_OUT_DIR/"