# Image Example

Demonstrates GPU-accelerated image processing using `filtrate` filters on `Image` and `Photo` components.

## Components Demonstrated

| Component | Description |
|-----------|-------------|
| **Image** | GPU-accelerated image display from RGBA pixel data |
| **Photo** | Async image loading from URL with GPU filters |

## GPU Filters (filtrate)

| Filter | Parameter | Range | Description |
|--------|-----------|-------|-------------|
| **Blur** | radius | 0+ | Gaussian blur effect |
| **Brightness** | amount | -1.0 to 1.0 | Lighten or darken |
| **Saturation** | amount | 0+ | Color intensity (0 = grayscale) |
| **Contrast** | amount | 0+ | Color contrast |
| **Grayscale** | intensity | 0-1.0 | Convert to grayscale |
| **Hue Rotation** | angle | 0-360 | Rotate colors around color wheel |
| **Invert** | - | - | Invert all colors |
| **Sepia** | intensity | 0-1.0 | Vintage sepia tone |
| **Vignette** | radius, softness | 0-1.0 | Darkened corners |
| **Sharpen** | amount | 0+ | Sharpen details |

## Running the Example

```bash
# iOS Simulator
water run --platform ios --example image

# Android Device/Emulator
water run --platform android --example image
```

## Code Highlights

### Image from Pixel Data

```rust
// Create RGBA pixel data
let pixels: Vec<u8> = generate_pattern(width, height);

// Display with GPU filters
Image::new(pixels, width, height)
    .blur(5.0)
    .brightness(0.1)
    .saturation(1.2)
```

### Photo from URL

```rust
// Async load from URL with filters
Photo::new("https://www.rust-lang.org/logos/rust-logo-512x512.png")
    .blur(2.0)
    .sepia(0.7)
    .vignette(0.5, 0.5)
```

### Combined Filters

```rust
// Chain multiple GPU filters
Image::new(pixels, 200, 150)
    .blur(2.0)
    .saturation(1.3)
    .vignette(0.6, 0.4)
```

### Filter Presets

```rust
// Vintage style
Photo::new(url)
    .sepia(0.7)
    .contrast(1.2)
    .vignette(0.5, 0.5)

// Dreamy effect
Image::new(pixels, w, h)
    .blur(3.0)
    .saturation(0.7)
    .brightness(0.1)
```

## Architecture

- **filtrate**: Standalone GPU filter crate using wgpu compute shaders
- **Image**: View wrapping `GpuSurface` with filtrate integration
- **Photo**: Async URL fetching + decoding → creates Image view

## Platform Notes

- All filters run on GPU via wgpu compute shaders
- Image decoding uses the `image` crate (PNG, JPEG, GIF, WebP, BMP, ICO)
- Photo loading is async with automatic view update when complete
