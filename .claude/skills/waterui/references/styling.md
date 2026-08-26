# Styling: colors, themes, icons, shapes

## Contents

- The default-appearance rule
- Theme color tokens
- Concrete colors
- Backgrounds, materials, gradients
- Shapes
- Icons
- Material 3 with the Hydrolysis renderer

## The default-appearance rule

View code that writes `.foreground()`, `.background()`, or `text("…")` with no extra
modifiers must already look right on every platform. Producing platform-correct defaults
is the framework's job, not the app's.

So: **reach for a theme token first, a concrete color only when the design genuinely calls
for that specific hue.** A screen built out of `Foreground`, `MutedForeground`, `Surface`,
`Border`, and `Accent` adapts to light and dark mode, to the platform's own palette, and to
a Material 3 theme, with no conditional code.

If defaults look wrong somewhere, that is a backend bug to report, not something to paper
over by hardcoding a color in the view.

## Theme color tokens

```rust
use waterui::prelude::theme_color::{Accent, Foreground, MutedForeground, Surface};

text("Title").foreground(Foreground)
text("Caption").caption().foreground(MutedForeground)
card.background(Surface)
```

The complete set:

| Token | Meaning |
|---|---|
| `Background` | Primary window or page background |
| `Surface` | Elevated surface — cards, sheets |
| `SurfaceVariant` | Alternate surface |
| `Border` | Borders and dividers |
| `Foreground` | Primary text and icons |
| `MutedForeground` | Secondary / dimmed text |
| `Accent` | Interactive accent |
| `AccentContainer` | Container associated with the accent |
| `AccentForeground` | Foreground drawn on accent backgrounds |
| `Tertiary` | Complementary emphasis accent |
| `TertiaryContainer` | Container associated with the tertiary accent |
| `SelectionContainer` | Fill painted behind a selected item |
| `SelectionForeground` | Foreground drawn on the selection container |

Each token is a zero-sized unit struct that is *also* a `View`, so `Surface.size(80.0, 40.0)`
paints a themed rectangle.

Project-level defaults for these slots go in `Water.toml` under `[theme]` — see
`references/project.md`.

## Concrete colors

```rust
Color::srgb_hex("#3B82F6")
Color::srgb(59, 130, 246)
Color::srgb_f32(0.23, 0.51, 0.96)
Color::p3(r, g, b)
Color::oklch(l, c, h)
Color::transparent()

const BRAND: Srgb = Srgb::from_hex("#3B82F6");   // const-evaluable, for constants
```

Named material-palette colors are unit structs and views:

```rust
Red Pink Purple DeepPurple Indigo Blue LightBlue Cyan Teal Green LightGreen
Lime Yellow Amber Orange DeepOrange Brown Grey BlueGrey
```

```rust
view.background(Blue)
Blue.size(80.0, 80.0)              // a colored rectangle
Blue.with_opacity(0.5)
```

Colors transform: `.lighten(a)`, `.darken(a)`, `.saturate(a)`, `.desaturate(a)`,
`.hue_rotate(deg)`, `.mix(other, factor)`, `.with_opacity(signal)`. `.with_opacity` on a
`Color` takes a signal, so opacity can animate without rebuilding.

## Backgrounds, materials, gradients

```rust
use waterui::background::Material;

view.background(Surface)                  // a color or token
view.background(Material::Regular)        // platform blur material
// Material::UltraThin | Thin | Regular | Thick | UltraThick
```

```rust
use waterui::gradient::{
    AngularGradient, ColorStop, LinearGradient, MeshGradient, RadialGradient, UnitPoint,
};

MeshGradient::new(3, 3, colors.clone()).size(300.0, 200.0)   // colors may be a signal
AnimatedMeshGradient::new(AnimatedMeshGradientConfig::aqua_bloom())
```

Gradients take signals directly, so an animated gradient is a signal change, not a rebuild.

## Shapes

Shapes are views that fill the space they are given. Two uses:

```rust
use waterui::shape::{Capsule, Circle, Ellipse, Path, Rectangle, RoundedRectangle,
                     ShapeExt, UnevenRoundedRectangle};

Circle.fill(Color::srgb_hex("#3B82F6")).size(80.0, 80.0)   // draw the shape
photo.clip(Circle)                                          // clip a view to the shape
```

`RoundedRectangle`, `UnevenRoundedRectangle`, and `Path` cover the rest;
`.morph_to(target, fill)` animates between two shapes.

A clip and a fill must describe the *same* shape. Passing a rounded rectangle's path
commands where the shape kind is expected stretches corner radii by the view's aspect
ratio — verify shape code against a deliberately non-square rectangle, never a square.

## Icons

Icons come from packaged icon-set crates. **Pick one set per app** and depend on it:

| Crate | Set |
|---|---|
| `waterui-icons-material-icon` | Material Symbols |
| `waterui-icons-lucide` | Lucide |
| `waterui-icons-fontawesome7` | Font Awesome 7 |
| `waterui-icons-sf-symbol` | SF Symbols — **Apple platforms only** |

```rust
use waterui_icons_material_icon as mdi;
use waterui_icons_lucide as lucide;

mdi::check_circle()
mdi::delete().size(20.0, 20.0)
mdi::flag().foreground(Accent)                      // theme color
lucide::star().tint(Color::srgb_hex("#F59E0B"))     // explicit tint
```

Match the set to the design language: a Material 3 app uses Material icons, not Lucide.

There is deliberately no cross-platform "system icon" fallback. Apple ships SF Symbols and
Android has no OS-supplied icon catalog, so the asymmetry is documented rather than hidden
behind a bundled font pretending to be the system's. Portable code depends on a packaged
set; `SystemIcon` is for Apple-only code.

## Material 3 with the Hydrolysis renderer

Hydrolysis (the self-drawn GPU renderer) gets widget chrome from a backend-neutral
`WidgetTheme`. For Material 3 output, install the theme package before running:

```rust
hydrolysis_m3::install(&mut env);        // light
hydrolysis_m3::install_dark(&mut env);
hydrolysis_m3::install_with_seed(&mut env, seed_color, mode);
```

For full Material You control, build schemes from a source color:

```rust
use hydrolysis_m3::{MaterialColorSource, install_with_source, install_with_color_schemes};

let source = MaterialColorSource::new(seed);   // variant, contrast, spec version, platform
let schemes = source.schemes();                // paired light/dark
install_with_color_schemes(&mut env, schemes, mode);
```

Material-specific role tokens live in `hydrolysis_m3::color::*` (`Primary`, `OnPrimary`,
`PrimaryContainer`, `SurfaceContainerHighest`, `OnSurfaceVariant`, …) and resolve from the
installed scheme. WaterUI's portable `theme_color::*` tokens map onto the active theme —
prefer the portable ones in views that should also run on native backends, and use the
Material roles only where the design is specifically Material.

To see it:

```bash
water preview my_view --backend hydrolysis --theme material3 --output preview.png
```
