# WaterUI CLI

WaterUI ships with a dedicated command line interface, exposed as the `water`
binary, to help you bootstrap, run, and package cross‑platform applications that
use the framework. The CLI keeps common platform tooling wired together so you
can stay inside your editor instead of juggling Xcode, Gradle, and browser build
chains.

## Features at a Glance

- Scaffold ready-to-run WaterUI projects with optional Android, Apple (SwiftUI),
  and Web backends.
- Hot-reload your app on Apple simulators/devices, Android emulators, or the
  browser with a single command.
- Package signed artifacts for distribution (APK and iOS app bundles).
- Audit and repair your toolchain with `water doctor`.
- Inspect connected simulators, emulators, and physical devices.
- Clean up build artifacts and caches across supported platforms.

## Installation

The CLI is developed in this repository and built with Rust. Install it with
`cargo` from the workspace root:

```bash
cargo install --path cli
```

During local development you can also run the binary without installing it:

```bash
cargo run -p waterui-cli -- <command> [flags]
```

> **Note:** Release builds embed version information from Git tags. If you are
> working from an unpublished checkout without tags, use the `--dev` flag on
> project-scaffolding commands to depend on the latest framework sources.

### Prerequisites

- A Rust toolchain that supports the 2024 edition (`rustup toolchain install
  stable` keeps you current).
- Git (used by the build script to resolve framework versions).
- Platform tooling for the backends you intend to use:
  - **Apple platforms:** macOS host with Xcode + command line tools.
  - **Android:** Android SDK, NDK, and either an emulator or a connected device.
  - **Web:** a modern browser (the CLI spins up a local development server).

Run `water doctor` any time to verify that everything is configured correctly.

## Global Flags

All commands share a couple of helpful flags:

- `-v / -vv`: increase logging verbosity (DEBUG / TRACE).
- `--format json` / `--json`: emit machine-readable JSON (useful for CI
  integration and scripting). JSON output disables interactive prompts, so pass
  the necessary flags up front (details below).

## Commands

| Command | Purpose | Common Flags |
| --- | --- | --- |
| `water create` | Scaffold a new WaterUI project interactively or from flags. | `--name`, `--directory`, `--backend`, `--dev`, `--yes` |
| `water run` | Build and hot-reload the app on a selected backend. | `--platform`, `--project`, `--device`, `--release`, `--no-watch` |
| `water package` | Produce distributable artifacts without launching them. | `--platform`, `--all`, `--release`, `--skip-native`, `--project` |
| `water doctor` | Check (and optionally fix) toolchain prerequisites. | `--fix` |
| `water devices` | List available simulators, emulators, and devices. |  |
| `water clean` | Remove Cargo, Gradle, Xcode, and workspace caches. | `-y/--yes` |
| `water add-backend` | Add an additional backend to an existing project. | `--project`, `--dev` |

A brief overview of the most common workflows follows.

### Create a Project

Generate a new project and choose which targets you need:

```bash
water create --name "Water Demo" \
  --bundle-identifier com.example.waterdemo \
  --backend swiftui --backend android --backend web
```

If you omit flags the CLI guides you through the options interactively. Newly
created projects include:

- A Rust library with a starter `lib.rs`.
- `Water.toml`, which declares the package metadata and enabled backends.
- Backend folders (`apple/`, `android/`, `web/`) populated with templates and
  build scripts.

Pass `--dev` to use the latest framework sources directly from Git while new
releases are being cut. When running with `--json`, also supply `--yes` (and
any other configuration flags) so the command can stay non-interactive.

### Run with Hot Reload

```bash
water run
```

When no `--platform` is given, the CLI detects available targets and prompts you
to choose (or uses `--device` to match a specific simulator/emulator name).
Source changes trigger incremental rebuilds via a file watcher; disable this
behaviour with `--no-watch`. Pass `--release` for optimized builds once things
are ready for profiling. JSON output requires supplying `--platform` or
`--device` ahead of time to avoid interactive prompts.

### Build Native Artifacts

Use `water build` when you only need the platform-specific Rust outputs—for
example when invoking Gradle/Xcode directly:

```bash
water build android --release
water build apple --release
```

The command replaces the old `build-rust.sh` logic. Android builds compile the
project crate for the requested ABIs and copy the `.so` files (plus the NDK
`libc++_shared.so`) into `android/app/src/main/jniLibs/`. Apple builds emit the
static library that the Xcode project links against and refresh
`apple/rust_build_info.xcconfig`. Both subcommands honour `--no-sccache` and
`--mold`, and they automatically read environment variables such as
`ANDROID_BUILD_TARGETS`, `CONFIGURATION`, and `BUILT_PRODUCTS_DIR` when invoked
from IDE build phases.

### Package Artifacts

Produce platform bundles without launching them:

```bash
water package --platform android --release
water package --platform ios --release
```

Use `--all` to build every configured backend. Android builds respect
`--skip-native` if you need to provide custom Rust artifacts instead of letting
the CLI invoke `water build android` automatically. JSON output requires
specifying `--platform` or `--all` so the command can stay non-interactive.

### Inspect and Fix Your Toolchain

```bash
water doctor --fix
```

`doctor` runs platform-specific health checks (Rust, Swift, Android). With
`--fix` it attempts to repair critical issues automatically; otherwise it offers
interactive confirmation when problems are detected. JSON output removes the
interactive prompts and surfaces structured summaries of each check and fix.

### List Connected Devices

```bash
water devices --json
```

The JSON output is useful for automation—e.g. selecting the first available
device inside a script. Without `--json` (or `--format json`), the CLI prints a
human-readable table.

### Clean Build Artifacts

```bash
water clean
```

The command enumerates pending cleanup actions and asks for confirmation (skip
the prompt with `--yes`). It deletes Cargo, Gradle, Xcode DerivedData, and other
workspace caches to help you reset stubborn build environments. In JSON mode,
the CLI auto-confirms and returns a structured report of what was removed,
skipped, or failed.

### Add Additional Backends

Extend an existing project with another target:

```bash
water add-backend swiftui
```

The CLI updates `Water.toml`, downloads the necessary templates, and ensures any
generated scripts are executable. As with `create`, you can pass `--dev` if you
need the development versions of the framework dependencies. Combine with
`--json` for a structured summary of what changed.

## Automating with JSON Output

Many commands honour the `--format json` global flag (or its shorthand
`--json`). In JSON mode, error messages are structured and every subcommand
returns machine-friendly payloads. Because prompts are disabled, provide any
required confirmation flags (`--yes`, `--platform`, `--device`, etc.) up
front. Commands such as `water clean`, `water doctor`, `water package`, `water
create`, and `water add-backend` emit rich JSON objects describing exactly what
work was performed.

## Troubleshooting

- Use `water doctor` first—it highlights missing SDK components, Rust targets,
  and misconfigured environment variables.
- For verbose logs add multiple `-v` flags (up to TRACE level).
- If you cloned the repository and encounter “WATERUI_VERSION is not set” while
  scaffolding, rerun the command with `--dev` to opt into the unreleased
  framework branches.

## Contributing

The CLI lives in `cli/` inside the main WaterUI workspace. Standard Rust
development workflows apply:

```bash
cargo fmt
cargo clippy --all-targets
cargo test -p waterui-cli
```

Bug reports and pull requests are welcome—open an issue in the main
[`water-rs/waterui`](https://github.com/water-rs/waterui) repository with CLI
details so we can triage it quickly.
