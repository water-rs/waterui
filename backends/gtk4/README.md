# WaterUI GTK4 Backend

A GTK4 backend implementation for the WaterUI framework, enabling native desktop applications on Linux, Windows, and macOS.

## Features

- **Native GTK4 Widgets**: Maps WaterUI components to native GTK4 widgets for optimal performance and native look & feel
- **Reactive Data Binding**: Full support for WaterUI's reactive system with automatic UI updates
- **Cross-Platform**: Works on Linux, Windows, and macOS with GTK4 installed
- **Form Components**: Complete set of form controls (text fields, toggles, sliders, steppers, color pickers)
- **Layout System**: Full layout support including stacks, grids, and scroll views
- **Event Handling**: Comprehensive event system for user interactions

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
waterui-gtk4 = "0.1.0"
waterui-core = "0.1.0"
waterui-text = "0.1.0"
waterui-form = "0.1.0"
waterui-layout = "0.1.0"
```

## Quick Start

```rust
use waterui_gtk4::{init, Gtk4App};
use waterui_text::text;
use waterui_layout::stack::vstack;
use waterui_core::binding;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize GTK4
    init()?;

    // Create the application
    let app = Gtk4App::new("com.example.my-app");

    // Run with your UI
    let exit_code = app.run(|| {
        vstack([
            text("Hello, GTK4!"),
            text("Built with WaterUI"),
        ])
        .spacing(10.0)
    });

    Ok(())
}
```

## Component Support

### Text Components
- `Text` -> `gtk4::Label`
- Rich text formatting support
- Reactive content updates

### Form Components
- `TextField` -> `gtk4::Entry`
- `Toggle` -> `gtk4::Switch` 
- `Slider` -> `gtk4::Scale`
- `Stepper` -> `gtk4::SpinButton`
- Color picker -> `gtk4::ColorButton`

### Layout Components
- `VStack` -> `gtk4::Box` (vertical)
- `HStack` -> `gtk4::Box` (horizontal)
- `ZStack` -> `gtk4::Overlay`
- `Grid` -> `gtk4::Grid`
- `ScrollView` -> `gtk4::ScrolledWindow`

### Navigation Components
- `TabView` -> `gtk4::Notebook`
- Header bars and navigation buttons

## Examples

See the `examples/` directory for complete working examples:

- `simple.rs` - Basic text and form widgets
- `form.rs` - Comprehensive form with all input types
- `layout.rs` - Layout examples with stacks and grids

Run examples with:
```bash
cargo run --example simple
cargo run --example form
```

## Architecture

The GTK4 backend consists of several key components:

- **Renderer**: Core rendering engine that converts WaterUI views to GTK4 widgets
- **Widget Implementations**: Specific implementations for each WaterUI component type
- **Layout System**: Handles WaterUI layout constraints and converts them to GTK4 layout managers
- **Event System**: Manages user interactions and reactive data binding
- **App Framework**: Application lifecycle and window management

## Platform Support

| Platform | Status | Notes |
|----------|---------|-------|
| Linux    | ✅ Supported | Native GTK4 support |
| macOS    | ✅ Supported | Requires GTK4 installation |
| Windows  | ✅ Supported | Requires GTK4 installation |

## Requirements

- GTK4 development libraries
- Rust 2024 edition or later

### Installing GTK4

**Ubuntu/Debian:**
```bash
sudo apt install libgtk-4-dev
```

**Fedora:**
```bash
sudo dnf install gtk4-devel
```

**macOS (Homebrew):**
```bash
brew install gtk4
```

**Windows:**
Follow the [GTK4 Windows installation guide](https://gtk-rs.org/gtk4-rs/stable/latest/docs/gtk4/installation.html).

## Contributing

Contributions are welcome! Please see the main WaterUI repository for contribution guidelines.

## License

This project is licensed under the MIT License - see the LICENSE file for details.