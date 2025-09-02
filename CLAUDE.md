# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development Commands

### Building and Testing
- `cargo build --all-features --workspace` - Build all crates with features
- `cargo test --all-features --workspace` - Run all tests
- `cargo clippy --all-targets --all-features --workspace -- -D warnings` - Run linting
- `cargo fmt --all -- --check` - Check code formatting
- `cargo doc --all-features --no-deps --workspace` - Generate documentation

### Memory Safety Testing
- `cargo +nightly miri test -p waterui-str` - Run Miri memory safety checks on specific crates
- Uses configuration in `utils/str/miri.toml` for memory safety validation

## Architecture Overview

WaterUI is a cross-platform UI framework built in Rust with these key architectural principles:

### Core System (`core/`)
- **View Trait**: Declarative UI components that compose recursively via `fn body(self, env: Environment) -> impl View`
- **Environment**: Type-based dependency injection system for propagating context through view hierarchy
- **Reactive State**: Integration with `nami` crate for automatic UI updates when data changes
- **AnyView**: Type erasure mechanism for heterogeneous view collections

### Component Organization
- `components/` - UI components organized by domain:
  - `form/` - Input controls and form elements
  - `layout/` - Layout containers (HStack, VStack, ZStack, Grid)
  - `media/` - Image, Video, LivePhoto components
  - `navigation/` - Navigation views, tabs, links
  - `text/` - Text rendering and typography

### Utilities (`utils/`)
- `color/` - Color management and type-safe color operations
- `str/` - String utilities optimized for UI contexts

### Backend Architecture (`backends/`)
- Platform-agnostic core with platform-specific rendering backends
- Currently includes `gtk4/` backend
- Designed for cross-platform deployment (desktop, mobile, web, embedded)

### Key Dependencies
- **nami**: Reactive state management system (local dependency)
- **native-executor**: Async task execution
- **typeshare**: Type sharing for cross-language compatibility

## Workspace Structure

This is a Cargo workspace with members across multiple directories. The main crate is `waterui` which re-exports functionality from:
- Core framework (`waterui-core`)
- Component libraries (`waterui-{text,media,form,layout,navigation}`)
- Utility libraries (`waterui-{str,color}`)

## Code Quality Standards

The workspace enforces strict linting via `workspace.lints` in `Cargo.toml`:
- Missing documentation warnings
- Comprehensive Clippy checks across all categories
- Rust edition 2024 with minimum version 1.85

## No-std Support

The framework supports `no-std` environments for embedded deployment. Use the `std` feature flag to enable standard library functionality when available.
- Use tuples to wrap mutiple views, for example, vstack((view1,view2))
- Use text! macro to create Text view, do not use format! macro when you work with reactive, it would loss reactive
- Use `pub fn view() -> impl View` unless you need to storage value in a struct. For example, if you are writing a clock, may you need to storage style in Clock struct, only create a new struct and use View trait to implement for struct in this case.
- The project is belonging to `water-rs`, leaded by Lexo Liu