# Filter Example

Demonstrates WaterUI's visual filter system with interactive controls and animations.

## Filters Demonstrated

| Filter | Parameter | Range | Description |
|--------|-----------|-------|-------------|
| **Blur** | radius | 0+ | Gaussian blur effect |
| **Brightness** | amount | -1.0 to 1.0 | Lighten or darken content |
| **Saturation** | amount | 0+ | Color intensity (0 = grayscale) |
| **Contrast** | amount | 0+ | Color contrast adjustment |
| **Hue Rotation** | degrees | 0-360 | Shift colors around the color wheel |
| **Grayscale** | intensity | 0-1.0 | Convert to grayscale |
| **Opacity** | value | 0-1.0 | Transparency level |
| **Shader effect** | scanline strength | 0-1.0 | Your own WGSL post-process (`src/shaders/crt.wgsl`) |

## Running the Example

```bash
# iOS Simulator
water run --platform ios --example filter

# Android Device/Emulator
water run --platform android --example filter
```

## Usage

Each filter section includes:
- A preview showing the filter applied to sample content
- A slider for continuous adjustment
- Preset buttons for quick value changes

All filter values are animated using spring or ease-in-out curves.

## Code Highlights

### Basic Filter Usage

```rust
// Single filter
view.blur(5.0)
view.brightness(0.2)
view.opacity(0.8)

// Chained filters
view
    .blur(3.0)
    .saturation(1.5)
    .hue_rotation(90.0)
```

### Animated Filters

```rust
let blur_radius = Binding::f32(0.0);

// Apply animation curve to the binding
let animated_blur = blur_radius
    .clone()
    .with(Animation::spring(200.0, 15.0));

// Use animated value with filter
sample_view.blur(animated_blur)
```

### Reactive Filter Values

```rust
// Filters automatically update when bindings change
let is_focused = Binding::bool(false);

let blur_amount = is_focused
    .clone()
    .map(|focused| if focused { 0.0 } else { 10.0 });

background_view.blur(blur_amount)
```

### Your Own Shader

```rust
use waterui::graphics::ShaderEffect;

// The WGSL module defines `@fragment fn main(in: VertexOutput)`; a prelude
// supplies `input_texture`, `input_sampler`, `uniforms.time`, `effect_param(i)`.
let crt = ShaderEffect::new(include_str!("shaders/crt.wgsl"))
    .expect("shaders/crt.wgsl is valid WGSL")
    .animated();

sample_view.filter(crt).param(strength)
```

`ShaderEffect::new` validates the WGSL and returns an error for a broken
shader. Applied to a window's root view, the effect post-processes everything
the window's content draws.

## Platform Notes

- **iOS/macOS**: Filters run through WaterUI's Rust `wgpu` filter pipeline
- **Android**: Uses RenderEffect (API 31+) and ColorMatrix for visual effects
