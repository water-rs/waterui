# WaterUI CLI

The official command-line interface for [WaterUI](https://github.com/water-rs/waterui), a cross-platform reactive UI framework for Rust.

The `water` tool handles project scaffolding, building, packaging, and running applications on Android, iOS, macOS, and the Web. It abstracts away the complexity of platform-specific build systems (Gradle, Xcode) and provides a unified development workflow with features like hot reload.

## Installation

### From Source

To install the CLI from the repository:

```bash
cargo install --path cli
```

Ensure that `~/.cargo/bin` is in your `PATH`.

## Usage

```bash
water <command> [options]
```

### Core Commands

#### `create`
Scaffold a new WaterUI project.

```bash
# Interactive mode
water create

# Create a project with specific backends
water create --name "My App" --backend apple --backend android

# Create a playground for quick experimentation
water create --playground
```

**Options:**
*   `--name <NAME>`: Application display name.
*   `--bundle-identifier <ID>`: Bundle ID (e.g., `com.example.app`).
*   `--backend <BACKEND>`: Backends to include (`apple`, `android`, `web`).
*   `--dev`: Use the development version of WaterUI (requires a local repository path).

#### `run`
Build and run the application on a device or simulator.

```bash
# Run on a connected device or available simulator (interactive selection)
water run

# Run specifically on Android
water run --platform android

# Run on a specific device
water run --device "iPhone 15"

# Rerun the previous configuration
water run again
```

**Key Features:**
*   **Hot Reload:** Enabled by default. Changes to your Rust code are automatically applied to the running app without restarting the application state where possible.
*   **Device Selection:** Automatically detects connected devices and simulators.

#### `doctor`
Check your development environment for required tools and dependencies.

```bash
# Check environment
water doctor

# Attempt to automatically fix issues (e.g., installing missing tools)
water doctor --fix
```

Checks for:
*   Rust toolchain and targets (`rustup`, `cargo`).
*   Android SDK, NDK, CMake.
*   Xcode and command-line tools (macOS).
*   UI automation tools (`idb` for iOS).

#### `backend`
Manage platform backends for your project.

```bash
# List configured backends
water backend list

# Add a new backend to an existing project
water backend add --backend web

# Update a backend to a specific version or commit
water backend update android
```

### Build & Package

#### `build`
Compile the Rust library for a specific target.

```bash
water build aarch64-linux-android
water build aarch64-apple-ios --release
```

#### `package`
Create distributable artifacts (APK, APP).

```bash
# Package for Android
water package --platform android --release

# Package for all configured platforms
water package --all --release
```

### Device Management

#### `devices`
List available devices and simulators.

```bash
water devices
```

#### `device`
Interact with running devices (useful for automation or debugging).

```bash
# Capture a screenshot
water device capture --output screenshot.png

# Tap the screen
water device tap --x 500 --y 500 --device "iPhone 15"

# Type text
water device type "Hello World"
```

## Project Structure

A typical WaterUI project created with `water create`:

```
my-app/
├── Water.toml          # Project configuration
├── Cargo.toml          # Rust dependencies
├── src/
│   └── lib.rs          # Application entry point
├── apple/              # Xcode project (if Apple backend enabled)
├── android/            # Android Studio project (if Android backend enabled)
└── web/                # Web assets (if Web backend enabled)
```

## Configuration (`Water.toml`)

The `Water.toml` file defines project metadata and backend configurations.

```toml
[package]
type = "app"
name = "My App"
bundle_identifier = "com.example.myapp"

[backends.android]
project_path = "android"
version = "0.1.0"

[backends.swift]
project_path = "apple"
scheme = "my-app"
```

## Architecture

The CLI acts as a bridge between the Rust ecosystem and platform-specific build tools:

*   **Android:** Wraps Gradle. `water run` invokes Gradle, which calls back into `water build` to compile the Rust shared library (`.so`), which is then packaged into the APK.
*   **Apple:** Wraps `xcodebuild`. `water run` invokes Xcode, which calls back into `water build` to compile the Rust static library (`.a`), which is linked into the app bundle.

This "Rust-as-a-dependency" approach ensures seamless integration with standard platform tooling while maintaining a unified Rust-centric workflow.
