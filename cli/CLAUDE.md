# CLI Crate Architecture

This document describes the architecture and conventions of the `waterui-cli` crate.

## Crate Structure

The CLI is split into two parts:

1. **Library (`cli/src/lib.rs`)** - Core logic, platform abstractions, device management
2. **Terminal (`cli/src/terminal/`)** - User interface, argument parsing, output formatting

**Key principle: Terminal handles interaction only, library handles real logic.**

```
cli/src/
├── lib.rs              # Library entry point (re-exports modules)
├── terminal/           # Binary entry point (UI layer)
│   ├── main.rs         # CLI argument parsing (clap)
│   ├── shell.rs        # Output formatting (spinners, colors, macros)
│   └── commands/       # Command implementations (thin wrappers)
├── device.rs           # Device trait and types
├── platform.rs         # Platform trait
├── project.rs          # Project management (Water.toml, Cargo.toml)
├── apple/              # Apple platform implementation
│   ├── device.rs       # AppleSimulator, MacOS, AppleDevice
│   ├── platform.rs     # ApplePlatform (iOS, macOS, simulator)
│   ├── backend.rs      # Apple backend configuration
│   └── toolchain.rs    # Xcode toolchain checking
├── android/            # Android platform implementation
│   ├── device.rs       # AndroidDevice, AndroidEmulator
│   ├── platform.rs     # AndroidPlatform
│   ├── backend.rs      # Android backend configuration
│   └── toolchain.rs    # Android SDK/NDK toolchain
├── build.rs            # RustBuild for compiling Rust code
├── toolchain/          # Toolchain utilities (doctor, brew, cmake)
├── debug/              # Hot reload server
└── templates/          # Project scaffolding templates
```

## Core Abstractions

### Device Trait (`device.rs`)

Represents something that can run an app (simulator, emulator, physical device).

```rust
pub trait Device: Send {
    type Platform: Platform;
    
    /// Launch the device (boot simulator/emulator). No-op for physical devices.
    fn launch(&self) -> impl Future<Output = eyre::Result<()>> + Send;
    
    /// Run an artifact on the device. Device must be launched first.
    fn run(&self, artifact: Artifact, options: RunOptions) 
        -> impl Future<Output = Result<Running, FailToRun>> + Send;
    
    fn platform(&self) -> Self::Platform;
}
```

**Important**: `launch()` handles booting. For emulators that need to be started from cold, `launch()` should start the emulator process and wait until it's ready. For already-connected devices, `launch()` is a no-op.

Implementations:
- `AppleSimulator` - iOS/tvOS/watchOS simulator (boots via `simctl boot`)
- `MacOS` - Current machine (no-op launch)
- `AndroidDevice` - Connected Android device (waits for device via adb)
- `AndroidEmulator` - AVD that needs to be launched (starts emulator process)

### Platform Trait (`platform.rs`)

Represents a build target platform.

```rust
pub trait Platform: Send {
    type Toolchain: Toolchain;
    type Device: Device;
    
    fn scan(&self) -> impl Future<Output = eyre::Result<Vec<Self::Device>>> + Send;
    fn build(&self, project: &Project, options: BuildOptions) -> impl Future<...>;
    fn package(&self, project: &Project, options: PackageOptions) -> impl Future<...>;
    fn clean(&self, project: &Project) -> impl Future<...>;
    fn toolchain(&self) -> Self::Toolchain;
    fn triple(&self) -> Triple;
}
```

Implementations:
- `ApplePlatform` - iOS, iOS Simulator, macOS, tvOS, etc.
- `AndroidPlatform` - Android with different ABIs (arm64-v8a, x86_64, etc.)

### Project (`project.rs`)

Manages `Water.toml` manifest and orchestrates builds.

Key methods:
- `Project::open()` - Open existing project
- `Project::create()` - Create new project
- `Project::run()` - Build, package, launch device, and run app
- `Project::build()` - Build Rust library
- `Project::package()` - Package for platform

## Terminal Layer Conventions

Terminal commands in `cli/src/terminal/commands/` should:

1. **Parse arguments** using clap
2. **Show progress** using `shell::spinner()`, `success!()`, `error!()`, etc.
3. **Delegate to library** for actual work
4. **Format output** for the user

Example pattern:
```rust
pub async fn run(args: Args) -> Result<()> {
    let project = Project::open(&args.path).await?;
    
    // Show progress
    let spinner = shell::spinner("Building...");
    
    // Delegate to library
    let result = project.build(platform, options).await;
    
    // Handle result with user-friendly output
    match result {
        Ok(_) => success!("Build complete"),
        Err(e) => error!("Build failed: {e}"),
    }
}
```

**Do NOT put heavy logic in terminal commands.** If you find yourself writing complex logic (loops, polling, process management), it belongs in the library layer.

## Device Lifecycle

The correct flow for running an app:

1. **Scan** - `Platform::scan()` returns available devices
2. **Select** - Choose a device (or create an emulator device if none available)
3. **Launch** - Terminal calls `Device::launch()` to boot simulator/emulator (can run in background while building)
4. **Run** - `Project::run()` builds, packages, and runs on the device (assumes device is already launched)

**Important**: `Project::run()` does NOT launch the device - it assumes the device is already launched and ready. The terminal layer (`water run` command) is responsible for:
- Spawning `device.launch()` as a background task
- Building and packaging the app in parallel with device launch
- Waiting for device to be ready before running the app

This allows simulator/emulator boot time to overlap with build time for better UX.

## Adding New Device Types

When adding a new device type:

1. Create a struct in the platform's `device.rs`
2. Implement `Device` trait
3. Put launching logic in `launch()` method
4. Reuse existing device's `run()` when possible

Example: `AndroidEmulator` (in `android/device.rs`):
```rust
pub struct AndroidEmulator {
    avd_name: String,
    device: OnceLock<AndroidDevice>,  // Set after launch
}

impl Device for AndroidEmulator {
    async fn launch(&self) -> Result<()> {
        // Start emulator process
        // Wait for it to boot (poll adb devices)
        // Store resulting AndroidDevice in self.device
    }
    
    async fn run(&self, artifact, options) -> Result<Running, FailToRun> {
        // Delegate to the inner AndroidDevice
        self.device.get().unwrap().run(artifact, options).await
    }
}
```

The terminal command just needs to create the right device type:
```rust
// In terminal/commands/run.rs
if let Some(dev) = devices.into_iter().next() {
    Ok(SelectedDevice::AndroidDevice(dev))
} else {
    // No connected devices - create an emulator device
    let avd = AndroidPlatform::list_avds().await?.first()...;
    Ok(SelectedDevice::AndroidEmulator(AndroidEmulator::new(avd)))
}
```

Then `Project::run()` calls `device.launch()` which handles the emulator startup.

## Error Handling

- Use `color_eyre::eyre::Result` for library functions
- Use `thiserror` for custom error enums (e.g., `FailToRun`, `FailToOpenProject`)
- Terminal layer converts errors to user-friendly messages

## Async Runtime

Uses `smol` for async:
- `smol::process::Command` for spawning processes
- `smol::spawn()` for background tasks
- `smol::Timer` for delays
- `smol::channel` for event streaming
- `smol::future::zip` for parallel operations
