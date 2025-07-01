# WaterUI Color Utilities

Color handling and manipulation utilities for the WaterUI framework.

## Overview

`waterui-color` provides color representation and manipulation capabilities for the WaterUI framework. This crate is currently in early development and will be expanded to include comprehensive color handling features.

## Current Implementation

### Color Structure

```rust
use waterui_color::Color;

// Basic color structure (placeholder)
let color = Color::default();
```

## Planned Features

### Color Representation

- **RGB/RGBA**: Standard red, green, blue, alpha color representation
- **HSL/HSLA**: Hue, saturation, lightness color model
- **Color Spaces**: sRGB, P3, wide gamut support
- **Named Colors**: Predefined color constants

### Color Manipulation

- **Brightness**: Adjust color brightness
- **Saturation**: Modify color saturation
- **Hue Shifting**: Rotate colors around the color wheel
- **Alpha Blending**: Transparency and opacity operations
- **Color Mixing**: Blend and interpolate between colors

### Theme Support

- **Dynamic Colors**: Colors that adapt to system themes
- **Semantic Colors**: Intent-based color definitions (primary, secondary, etc.)
- **Accessibility**: High contrast and color blindness support
- **Platform Integration**: Native color picker integration

### Gradients

- **Linear Gradients**: Color transitions along a line
- **Radial Gradients**: Circular color transitions
- **Conic Gradients**: Angular color transitions
- **Multi-stop Gradients**: Complex gradient definitions

## Future API Design

```rust
use waterui_color::*;

// Color creation
let red = Color::rgb(255, 0, 0);
let blue = Color::hsl(240.0, 1.0, 0.5);
let transparent = Color::rgba(0, 0, 0, 0.5);

// Named colors
let primary = Color::PRIMARY;
let secondary = Color::SECONDARY;
let accent = Color::ACCENT;

// Color manipulation
let darker = red.darken(0.2);
let lighter = red.lighten(0.3);
let desaturated = red.desaturate(0.5);

// Gradients
let gradient = LinearGradient::new()
    .start(Color::RED)
    .end(Color::BLUE)
    .angle(45.0);

// Theme colors
let adaptive = Color::adaptive()
    .light(Color::BLACK)
    .dark(Color::WHITE);
```

## Dependencies

Currently minimal dependencies. Future versions will include:
- Color space conversion libraries
- Platform-specific color handling
- Accessibility utilities

## Contributing

This crate is in early development. Contributions for color handling features are welcome:

1. Color space conversions
2. Color manipulation algorithms
3. Platform-specific integrations
4. Accessibility features
5. Performance optimizations

## Example Usage

```rust
use waterui_color::Color;

struct ColoredView {
    background: Color,
}

impl View for ColoredView {
    fn body(self, env: &Environment) -> impl View {
        rectangle()
            .fill(self.background)
            .frame_size(100.0, 100.0)
    }
}
```