# WaterUI Terminal Backend

This crate implements the terminal-first (TUI) backend for WaterUI. The backend
is powered by [`crossterm`](https://crates.io/crates/crossterm) and renders
`AnyView` trees into styled terminal output while honouring WaterUI's reactive
data model.

Current status:

- ✅ Crate registered in the Cargo workspace
- ✅ `TuiApp`/`TuiAppBuilder` provide an ergonomic entry point
- ✅ Crossterm powered terminal driver with alternate-screen lifecycle
- ✅ Renderer capable of painting text, layout containers, scroll views and
  navigation shells
- ✅ Colour, font-weight and text attribute mapping for `StyledStr`
- 🚧 Incremental diffing & event routing (future milestone)
- 🚧 Extended component coverage (form controls, media, graphics)

## Usage

```rust
use waterui_tui::{TuiApp, TuiAppBuilder};
use waterui_text::text;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = TuiAppBuilder::new().build()?;
    app.render(text("Hello from WaterUI"))?;
    Ok(())
}
```

### Exercising the backend

```bash
cargo test -p waterui-tui
```

## Layout

- `src/app.rs`: builder/entry point that wires the terminal and renderer.
- `src/terminal.rs`: crossterm backed terminal abstraction (alt screen + raw mode).
- `src/renderer.rs`: view walker translating WaterUI primitives to terminal frames.
- `src/error.rs`: shared error type surfaced by the builder and runtime.
- `PLAN.md`: milestone tracker updated as the backend evolves.

Please keep this README up-to-date as the backend gains additional rendering
capabilities.
