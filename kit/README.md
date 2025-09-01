# WaterUI Kit

High-level development kit and utility tools for the WaterUI framework.

## Overview

`waterui-kit` provides a comprehensive development kit with utilities, tools, and helper functions that make working with WaterUI more convenient and efficient. This crate includes development utilities, testing helpers, and common patterns.

## Features

- **Development Tools**: Utilities for debugging and development
- **Testing Helpers**: Test utilities for WaterUI components
- **Common Patterns**: Reusable UI patterns and compositions
- **Build Tools**: Integration with build systems and asset management

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui-kit = "0.1.0"
```

## Development Utilities

The kit provides various utilities for development and debugging:

```rust
use waterui_kit::{debug, inspect, performance};

// Debug views in development
let debug_view = debug(my_view())
    .show_bounds(true)
    .show_layout_info(true);

// Performance monitoring
let monitored = performance::monitor(expensive_view())
    .track_render_time()
    .track_memory_usage();
```

## Testing Support

Helper functions for testing WaterUI components:

```rust
use waterui_kit::testing::*;

#[test]
fn test_my_component() {
    let component = MyComponent::new();
    let rendered = test_render(component);
    
    assert_eq!(rendered.text_content(), "Expected text");
    assert!(rendered.has_class("my-component"));
}
```

## Dependencies

- `waterui-core`: Core framework functionality

This crate is designed to be used during development and testing, and may not be needed in production builds.