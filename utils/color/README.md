# waterui-color

Cross-platform color handling with support for multiple color spaces and perceptually-uniform transformations.

## Overview

`waterui-color` provides the color system for WaterUI, offering comprehensive support for sRGB, Display P3, and OKLCH color spaces. The crate emphasizes perceptually-uniform color manipulation through the OKLCH color space, enabling predictable lightness, chroma, and hue adjustments that maintain consistent contrast relationships across light and dark themes.

Colors in WaterUI integrate seamlessly with the reactive system (`nami`) and can be used both as standalone values and as renderable views. The crate handles gamma correction, color space conversions, HDR headroom, and opacity management automatically.

This crate is part of the WaterUI framework and is typically accessed through the `waterui::color` module or the `waterui::prelude`.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui-color = "0.1.0"
```

Or use through the main WaterUI crate:

```toml
[dependencies]
waterui = "0.2"
```

## Quick Start

```rust
use waterui::prelude::*;

// Create colors in different color spaces
let red = Color::srgb(255, 0, 0);
let p3_green = Color::p3(0.0, 1.0, 0.0);
let perceptual_blue = Color::oklch(0.6, 0.25, 264.0);

// Parse from hex strings
let orange = Color::srgb_hex("#FF9800");

// Apply transformations
let semi_transparent = Color::srgb_hex("#2196F3").with_opacity(0.5);
let lighter = Color::srgb(100, 100, 100).lighten(0.2);
let desaturated = Color::oklch(0.7, 0.3, 45.0).desaturate(0.5);

// Mix colors
let blended = Color::srgb(0, 0, 255).mix(Color::srgb(255, 0, 0), 0.5);
```

## Core Concepts

### Color Spaces

**sRGB** - Standard RGB color space used in most displays and web content. Values are gamma-encoded for display.

**Display P3** - Wide color gamut space supporting more vivid colors than sRGB, commonly available on modern displays.

**OKLCH** - Perceptually-uniform color space recommended for UI work. Allows independent adjustment of:

- **Lightness** (0.0-1.0): Perceived brightness
- **Chroma**: Color intensity/saturation
- **Hue** (0-360 degrees): Color angle

### Resolved Colors

All color types resolve to `ResolvedColor`, which stores colors in linear sRGB space with extended range support. This internal representation enables:

- Accurate color blending and interpolation
- HDR headroom for high dynamic range displays
- Opacity/alpha channel management
- Efficient conversion between color spaces

### Reactive Integration

Colors implement the `Resolvable` trait, allowing them to work with WaterUI's reactive system. Colors can be derived from reactive signals and automatically update when dependencies change.

## Examples

### Basic Color Creation

```rust
use waterui::prelude::*;

// 8-bit RGB components
let red = Color::srgb(255, 0, 0);

// Floating-point RGB (0.0-1.0)
let green = Color::srgb_f32(0.0, 1.0, 0.0);

// Hexadecimal strings (with or without '#' prefix)
let blue = Color::srgb_hex("#0000FF");
let cyan = Color::srgb_hex("00FFFF");

// Packed 32-bit RGB
let magenta = Color::srgb_u32(0xFF00FF);

// OKLCH (perceptually uniform)
let orange = Color::oklch(0.7, 0.15, 45.0);

// Display P3 wide gamut
let vivid = Color::p3(1.0, 0.0, 0.5);

// Transparent color
let clear = Color::transparent();
```

### Perceptual Color Manipulation

```rust
use waterui::prelude::*;

let base = Color::srgb_hex("#4CAF50");

// Lightness adjustments (perceptually uniform)
let lighter = base.clone().lighten(0.2);    // Increase lightness by 20%
let darker = base.clone().darken(0.15);     // Decrease lightness by 15%

// Saturation adjustments
let vibrant = base.clone().saturate(0.3);   // Increase saturation
let muted = base.clone().desaturate(0.5);   // Decrease saturation

// Hue rotation
let complementary = base.clone().hue_rotate(180.0);  // Rotate 180 degrees
let analogous = base.clone().hue_rotate(30.0);       // Rotate 30 degrees
```

### Opacity and Mixing

```rust
use waterui::prelude::*;

// Apply opacity (alpha channel)
let semi_transparent = Color::srgb_hex("#FF5722").with_opacity(0.5);
let faded = Color::srgb(100, 200, 150).with_alpha(0.3);  // Alias for with_opacity

// Mix two colors with linear interpolation
let purple = Color::srgb(255, 0, 0).mix(Color::srgb(0, 0, 255), 0.5);

// Create gradient points
let start = Color::srgb_hex("#FF6B6B");
let end = Color::srgb_hex("#4ECDC4");
let midpoint = start.mix(end, 0.5);
```

### Color as a View

```rust
use waterui::prelude::*;

// Color implements View and can be used directly
fn colored_box() -> impl View {
    Color::srgb_hex("#2196F3")
        .frame()
        .width(100.0)
        .height(100.0)
}

// Use as background
fn text_with_background() -> impl View {
    text("Hello, World!")
        .background(Color::srgb_hex("#4CAF50"))
        .foreground(Color::srgb(255, 255, 255))
}
```

### Named Color Constants

```rust
use waterui::prelude::*;

// Pre-defined color constants (implement View)
fn color_palette() -> impl View {
    vstack((
        Red,
        Blue,
        Green,
        Yellow,
        Orange,
        Purple,
        Pink,
        Cyan,
        Teal,
        Amber,
        Grey,
    ))
}
```

## API Overview

### Color Creation

- `Color::srgb(r, g, b)` - Create from 8-bit RGB values (0-255)
- `Color::srgb_f32(r, g, b)` - Create from float RGB values (0.0-1.0)
- `Color::srgb_hex(hex)` - Parse from hex string ("#RRGGBB", "0xRRGGBB", or "RRGGBB")
- `Color::srgb_u32(rgb)` - Create from packed 32-bit value
- `Color::oklch(l, c, h)` - Create from OKLCH components
- `Color::p3(r, g, b)` - Create from Display P3 values
- `Color::transparent()` - Fully transparent black

### Color Transformations

- `.lighten(amount)` - Increase lightness in OKLCH space
- `.darken(amount)` - Decrease lightness in OKLCH space
- `.saturate(amount)` - Increase chroma/saturation
- `.desaturate(amount)` - Decrease chroma/saturation
- `.hue_rotate(degrees)` - Rotate hue by degrees
- `.with_opacity(opacity)` - Set opacity (0.0-1.0)
- `.with_alpha(alpha)` - Alias for `with_opacity`
- `.mix(other, factor)` - Linear interpolation between colors

### Advanced

- `.with_headroom(headroom)` - Set HDR headroom value
- `.resolve(env)` - Resolve to concrete color in given environment
- `Color::try_srgb_hex(hex)` - Fallible hex parsing (returns `Result`)

### Color Space Types

- `Srgb` - sRGB color space representation
- `P3` - Display P3 color space representation
- `Oklch` - OKLCH color space representation
- `ResolvedColor` - Internal linear RGB representation with metadata

### Named Color Views

Material Design-inspired color constants that implement `View`:
`Red`, `Pink`, `Purple`, `DeepPurple`, `Indigo`, `Blue`, `LightBlue`, `Cyan`, `Teal`, `Green`, `LightGreen`, `Lime`, `Yellow`, `Amber`, `Orange`, `DeepOrange`, `Brown`, `Grey`, `BlueGrey`
