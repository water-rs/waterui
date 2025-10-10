#!/bin/sh
set -e

# This script builds the Rust library for all Android targets.

# You must have the Android NDK installed and the following environment variables set:
# export ANDROID_NDK_HOME="/path/to/your/android-ndk"
# You also need to install the Rust targets:
# rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android

CRATE_NAME=__CRATE_NAME__

# Build for all Android targets
cargo build --target aarch64-linux-android --release
cargo build --target armv7-linux-androideabi --release
cargo build --target i686-linux-android --release
cargo build --target x86_64-linux-android --release

# Copy the libraries to the jniLibs directory
JNI_DIR="android/app/src/main/jniLibs"

mkdir -p "$JNI_DIR/arm64-v8a"
cp "target/aarch64-linux-android/release/lib${CRATE_NAME}.so" "$JNI_DIR/arm64-v8a/"

mkdir -p "$JNI_DIR/armeabi-v7a"
cp "target/armv7-linux-androideabi/release/lib${CRATE_NAME}.so" "$JNI_DIR/armeabi-v7a/"

mkdir -p "$JNI_DIR/x86"
cp "target/i686-linux-android/release/lib${CRATE_NAME}.so" "$JNI_DIR/x86/"

mkdir -p "$JNI_DIR/x86_64"
cp "target/x86_64-linux-android/release/lib${CRATE_NAME}.so" "$JNI_DIR/x86_64/"

echo "Rust libraries copied to $JNI_DIR"
