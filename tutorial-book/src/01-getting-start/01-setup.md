# Installation and Setup

Before we dive into building applications with WaterUI, let's set up a proper development environment. This chapter will guide you through installing Rust, setting up your editor, and creating your first WaterUI project.

## Installing Rust

WaterUI requires Rust 1.85 or later with the 2024 edition. The easiest way to install Rust is through rustup.

### On macOS, Linux, or WSL

```bash,ignore
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

### On Windows

1. Download the installer from [rustup.rs](https://rustup.rs/)
2. Run the downloaded `.exe` file
3. Follow the installation prompts
4. Restart your command prompt or PowerShell

### Verify Installation

After installation, verify that everything works:

```bash,ignore
rustc --version
cargo --version
```

You should see output like:
```text
rustc 1.85.0 (a28077b28 2024-02-28)
cargo 1.85.0 (1e91b550c 2024-02-27)
```

> **Note**: WaterUI requires Rust 1.85 or later. If you have an older version, update with `rustup update`.

## Editor Setup

While you can use any text editor, we recommend VS Code for the best WaterUI development experience.

### Visual Studio Code (Recommended)

1. **Install VS Code**: Download from [code.visualstudio.com](https://code.visualstudio.com/)

2. **Install Essential Extensions**:
   - **rust-analyzer**: Provides IntelliSense, error checking, and code completion
   - **CodeLLDB**: Debugger for Rust applications
   - **Better TOML**: Syntax highlighting for Cargo.toml files

3. **Optional Extensions**:
   - **Error Lens**: Inline error messages
   - **Bracket Pair Colorizer**: Colorizes matching brackets
   - **GitLens**: Enhanced Git integration

### Other Popular Editors

**IntelliJ IDEA / CLion**:
- Install the "Rust" plugin
- Excellent for complex projects and debugging

**Vim / Neovim**:
- Use `rust.vim` for syntax highlighting
- Use `coc-rust-analyzer` for LSP support

**Emacs**:
- Use `rust-mode` for syntax highlighting
- Use `lsp-mode` with rust-analyzer

## Creating Your First Project

Let's create a new WaterUI project from scratch:

```bash,ignore
cargo new hello-waterui
cd hello-waterui
```

This creates a new Rust project with the following structure:

```text
hello-waterui/
├── Cargo.toml
├── src/
│   └── main.rs
└── .gitignore
```

### Adding WaterUI Dependencies

Edit your `Cargo.toml` file to include WaterUI:

**Filename**: `Cargo.toml`
```toml
[package]
name = "hello-waterui"
version = "0.1.0"
edition = "2024"

[dependencies]
waterui = { path = ".." }
# Backend will be chosen in the future
```

### Web Development (WebAssembly)

For web development, install additional tools:

```bash,ignore
# Install wasm-pack for building WebAssembly packages
curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh

# Add WebAssembly target
rustup target add wasm32-unknown-unknown
```

## Hello,world!

Let's create a simple "Hello, World!" application to verify everything works.

**Filename**: `src/main.rs`
```rust,ignore
use waterui::prelude::*;

fn home() -> impl View { "Hello, WaterUI! 🌊" }

fn main() {
    // Backend-specific initialization will be added here
    // For now, we just define the view
}
```

### Building and Running

Build and run your application:

```bash,ignore
cargo run
```

If everything is set up correctly, you should see a window with "Hello, WaterUI! 🌊" displayed.

## Troubleshooting Common Issues

### Rust Version Too Old

**Error**: `error: package requires Rust version 1.85`

**Solution**: Update Rust:
```bash,ignore
rustup update
```

### Windows Build Issues

**Error**: Various Windows compilation errors

**Solutions**:
1. Ensure you have the Microsoft C++ Build Tools installed
2. Use the `x86_64-pc-windows-msvc` toolchain
3. Consider using WSL2 for a Linux-like environment