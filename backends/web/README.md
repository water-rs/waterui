# WaterUI Web Backend

A WebAssembly-based web backend for the WaterUI framework, enabling modern web applications with native-like performance.

## Features

- **WebAssembly Support**: Compiles to WASM for efficient execution in browsers
- **DOM Integration**: Direct manipulation of web elements using web-sys
- **Form Components**: Text fields, toggles, sliders, steppers, and pickers
- **Layout Support**: Flexible layout components (simplified stack implementation)
- **Event Handling**: Event listeners for interactive components
- **Reactive Updates**: Integration with WaterUI's reactive state management

## Getting Started

### Prerequisites

- Rust toolchain with WebAssembly target
- `wasm-pack` for building WebAssembly packages

```bash
# Install wasm-pack
curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh

# Add WebAssembly target
rustup target add wasm32-unknown-unknown
```

### Basic Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui-web = { path = "../backends/web" }
waterui = { path = "../.." }
```

Example application:

```rust
use wasm_bindgen::prelude::*;
use waterui_web::{WebApp, init};
use waterui::{text, Environment};

#[wasm_bindgen(start)]
pub fn main() {
    init();
    
    let app = WebApp::new("app");
    let env = Environment::new();
    
    let content = text("Hello, WaterUI Web!");
    
    app.environment(env).render(content).unwrap();
}
```

### HTML Integration

Create an HTML file with a container element:

```html
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>WaterUI Web App</title>
</head>
<body>
    <div id="app"></div>
    <script type="module">
        import init from './pkg/your_app.js';
        init();
    </script>
</body>
</html>
```

## Building

Build the WebAssembly package:

```bash
wasm-pack build --target web --out-dir pkg
```

## Architecture

The web backend follows WaterUI's component architecture:

- **WebElement**: DOM element wrapper with utility methods
- **ViewDispatcher**: Routes view components to appropriate renderers
- **Widget Renderers**: Convert WaterUI components to web elements
- **Event System**: Handles user interactions and state updates

## Supported Components

### Text Components
- `Text` - Styled text rendering
- `Label` - Simple text labels

### Form Components
- `TextField` - Text input fields
- `Toggle` - Checkboxes and switches
- `Slider` - Range input controls
- `Stepper` - Number input controls
- `Picker` - Select dropdowns
- `ColorPicker` - Color input controls
- `DatePicker` - Date input controls

### Layout Components
- `Stack` - Flexible layout container (simplified implementation)

## Development Status

This is an initial implementation focusing on core functionality. Advanced features like complex layouts, animations, and advanced styling are planned for future releases.

## Contributing

The web backend is part of the larger WaterUI framework. Please refer to the main project's contribution guidelines.