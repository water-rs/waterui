# WaterUI 🌊

Modern, reactive UI building blocks in Rust — with desktop (GTK4) and web (WASM) backends in this workspace.

## ✨ Features

- 🦀 Rust-first: type-safe and efficient
- 🔄 Reactive: UI updates as your state changes (via Nami)
- 🖥️ GTK4 and 🌐 Web backends in-tree
- 🪶 no-std friendly core building blocks

## 🚀 Quick Start (GTK4)

```rust,ignore
use nami::binding;
use waterui::View;
use waterui::component::button;
use waterui_layout::stack::{vstack, hstack};
use waterui_text::text;
use waterui_gtk4::{init, Gtk4App};

fn counter_view() -> impl View {
    let count = binding(0);

    vstack((
        text!("Count: {}", count),
        hstack((
            button("Increment").action({
                let count = count.clone();
                move |_| count.update(|c| c + 1)
            }),
            button("Reset").action({
                let count = count.clone();
                move |_| count.set(0)
            }),
        )),
    ))
    .padding()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init()?;
    let app = Gtk4App::new("com.example.counter");
    let _exit = app.run(|| counter_view());
    Ok(())
}
```

## 🏗️ Architecture

WaterUI centers on these concepts:

- Views: implement `waterui_core::View` to describe UI
- Reactive State: values from `nami` (`binding`, `s!`) drive updates
- Environment: type-based context you pass through the tree

```rust,ignore
use nami::{binding, s};
use waterui::{View};
use waterui_text::text;

fn greeting_view() -> impl View {
    let username = binding("Alice".to_string());
    let greeting = s!("Hello, {}!", username);
    text!(greeting)
}
```

## 📦 Workspace Crates

- `waterui` (this crate): re-exports core types and common components
- `waterui-core`: `View`, environment, type-erased views, etc.
- `waterui-text`: `Text` and `text!` macro
- `waterui-layout`: stack, grid, scroll primitives
- `waterui-form`: text field, toggle, slider, stepper, color picker
- `waterui-media`: photo, video, live photo, media enum
- `waterui-gtk4`: GTK4 backend and app runner
- `waterui-web`: Web (WASM) backend

## 📱 Platform Support

| Platform        | Backend     | Status |
| --------------- | ----------- | ------ |
| Linux/macOS/Win | GTK4        | ✅     |
| Web Browser     | WebAssembly | 🚧    |

## 🔧 Development

Prereqs for GTK4 examples:
- Linux: `sudo apt install libgtk-4-dev build-essential`
- macOS: `brew install gtk4`
- Windows: MSYS2 with GTK4

Build the workspace:
```bash
cargo build
```

Run examples:
```bash
# GTK4
cargo run -p waterui-gtk4 --example shapes
cargo run -p waterui-gtk4 --example form

# Web (requires wasm target)
wasm-pack build backends/web --target web --out-dir pkg
```

## 📚 Documentation

- Tutorial book: `tutorial-book/` (work-in-progress)
- Architecture notes: `CODEBASE_DOCUMENTATION.md`
- Examples: `backends/gtk4/examples`, `backends/web/examples`, and component `examples/`

## Status

Active development; APIs may evolve. GTK4 paths and core components are usable; the web backend and some docs are still catching up.

## License

MIT — see `LICENSE`.
