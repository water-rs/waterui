# Styling: colors, themes, icons, shapes

## Contents

- The default-appearance rule
- Theme color tokens
- Installing a theme (dark mode, custom fonts)
- Concrete colors
- Reactive colors
- HDR
- Backgrounds, materials, gradients
- Shapes
- Floating surfaces
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
paints a themed rectangle. To use a token as a *value* — say, one arm of a `.select` —
convert it: `let indicator: Color = SurfaceVariant.into();`.

Project-level defaults for these slots go in `Water.toml` under `[theme]` — see
`references/project.md`.

## Installing a theme (dark mode, custom fonts)

`Theme` is a builder and a `Plugin`; install it in `app(mut env)`. `.color_scheme(..)`
takes a **signal**, which is how a runtime dark-mode toggle works with no rebuild:

```rust
pub fn app(mut env: Environment) -> App {
    let dark = Binding::bool(true);
    env.install(
        Theme::new()
            .color_scheme(dark.select(ColorScheme::Dark, ColorScheme::Light))
            .fonts(FontSettings::new()),          // per-slot setters, each a signal slot
    );
    App::new(move || root(dark.clone()), env)
}
```

Only the fields you set overwrite the ambient theme. `env.install(theme)` on the app
environment applies globally; `.install(theme)` on a view scopes it to that subtree.

## Concrete colors

```rust
Color::srgb_hex("#3B82F6")
Color::srgb(59, 130, 246)          // 0-255 integers
Color::srgb_f32(0.23, 0.51, 0.96)
Color::p3(r, g, b)
Color::oklch(l, c, h)
Color::transparent()

use waterui::color::Srgb;
const BRAND: Srgb = Srgb::from_hex("#3B82F6");   // const-evaluable — works in const fn too
Srgb::new(0.8, 0.9, 1.0)                          // 0.0-1.0 floats (unlike Color::srgb)
Srgb::WHITE / Srgb::BLACK
```

`Srgb` is itself a view and a valid argument to `.foreground()` / `.background()`; it is
also the *required* color type for chart `.color(..)`. Convert to the semantic type with
`Color::from(srgb)` when an API wants a `Color`.

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
`.hue_rotate(deg)`, `.mix(other, factor)`, `.with_opacity(signal)`. Keep the two
opacities straight: `Color::with_opacity(..)` bakes alpha into the color *value*;
`.opacity(signal)` on a view is the reactive modifier — cross-fades stack both layers and
drive each layer's `.opacity`.

## Reactive colors

APIs that take a concrete `Color` (`.fill(..)`, `.background(..)`, `.foreground(..)`) do
not accept a signal directly — wrap the signal with `signal_color` (in the prelude), which
produces a `Color` that updates in place:

```rust
let fill = signal_color(selected.select(indicator, clear).computed());
Rectangle.fill(fill).size(64.0, 32.0)
```

Without it, `.fill(binding)` fails to type-check and `.fill(binding.get())` freezes —
the rule-1 bug in a hat.

## HDR

`color.with_headroom(1.5)` marks a color as extended-range (up to 2.5× SDR white), and
the *view* must additionally opt in: `.color_space(ColorSpace::Hdr)` (import
`waterui::metadata::secure::ColorSpace`). `.color_space(..)` takes a plain enum, not a
signal, and erases its receiver to `AnyView`. Compare SDR/HDR variants by stacking both
and cross-fading opacity — not by swapping subtrees.

## Backgrounds, materials, gradients

```rust
use waterui::background::Material;

view.background(Surface)                  // a color or token
view.background(Material::Regular)        // platform blur material
// Material::UltraThin | Thin | Regular | Thick | UltraThick
view.background(RoundedRectangle::new(0.18).fill(Surface))   // any view is a valid background
```

Gradients come in **two families that share names — pick the import deliberately**:

- `waterui::gradient::*` (prelude): background-descriptor types (`LinearGradient`,
  `RadialGradient`, `AngularGradient`, `MeshGradient` over `MeshVertex`, `ColorStop`,
  `UnitPoint`) for `.background(..)`.
- `waterui_graphics::{Gradient, MeshGradient, ResolvedColor}` (the `waterui-graphics`
  crate, `features = ["gpu"]`): standalone GPU-rendered gradient *views*. Because the
  prelude already binds `Gradient`, calling `Gradient::linear(..)` after a prelude glob
  resolves to the wrong type — import the graphics one explicitly.

```rust
use waterui_graphics::{AnimatedMeshGradient, AnimatedMeshGradientConfig, MeshGradient, ResolvedColor};

// Stops take ResolvedColor: a plain struct of five public f32 fields (struct-literal it).
let stop = ResolvedColor { red: 1.0, green: 0.3, blue: 0.5, opacity: 1.0, headroom: 0.0 };

MeshGradient::new(3, 3, colors.clone()).size(300.0, 200.0)   // colors: any signal of ResolvedColors
AnimatedMeshGradient::new(AnimatedMeshGradientConfig::aqua_bloom())   // animates in-shader, zero CPU
```

The graphics-crate gradients take signals directly, so an animated gradient is a signal
change, not a rebuild.

## Shapes

Shapes are views that fill the space they are given. Two uses:

```rust
use waterui::shape::{Capsule, Circle, Ellipse, Path, Rectangle, RoundedRectangle,
                     ShapeExt, UnevenRoundedRectangle};

Circle.fill(Color::srgb_hex("#3B82F6")).size(80.0, 80.0)   // draw the shape
photo.clip(Circle)                                          // clip a view to the shape
```

**Corner radii are normalized, not points.** `RoundedRectangle::new(r)` takes a fraction
of the shape's *shorter side*: `0.1` is a subtle round, `0.5` is fully rounded, and
larger values silently saturate at 0.5. `RoundedRectangle::new(12.0)` therefore compiles
and produces a capsule — the single most common shape mistake. For "fully rounded at any
size", use `Capsule`. `UnevenRoundedRectangle::new(top_leading, top_trailing,
bottom_leading, bottom_trailing)` takes four normalized radii in reading order — *not*
clockwise.

Custom paths are unit-space (0.0–1.0 fractions of the bounds, stretched to fit) and the
builder consumes `self` — reassign when building in a loop:

```rust
let triangle = Path::new()
    .move_to(0.5, 0.0)
    .line_to(1.0, 1.0)
    .line_to(0.0, 1.0)
    .close();
triangle.fill(Accent).size(80.0, 80.0)
```

`cubic_to(c1x, c1y, c2x, c2y, x, y)` adds a curve. Because coordinates are unit-space, a
path-drawn corner stretches with the aspect ratio — that is why built-in shapes carry
their kind instead of path commands, and why shape code is tested against a deliberately
non-square rectangle, never a square.

Morphing is a self-animating view over the **SDF-backed built-ins only** (Rectangle,
Circle, Ellipse, RoundedRectangle, UnevenRoundedRectangle, Capsule — not `Path`):

```rust
Circle
    .morph_to(RoundedRectangle::new(0.22), Color::srgb_hex("#3B82F6"))
    .duration(Duration::from_millis(600))
    .autoreverse(true)                       // also .repeat(bool)
    .size(90.0, 90.0)

shape.morph_to(target, fill).progress(t.clone())   // or drive it from a 0..=1 signal instead
```

`FilledShape::morph_to(target)` (no fill argument) keeps an already-applied fill.

## Floating surfaces

`.floating()` promotes any view to a themed elevated surface (shadow, clip radius,
insets) while preserving the wrapped control's semantic identity; `.floating_with(style)`
supplies explicit `FloatingStyle` tokens. Themes may install a `FloatingStyle` in the
environment — read it with `env.get::<FloatingStyle>()` (falling back to
`FloatingStyle::default()`) when custom chrome must match floating controls.

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
behind a bundled font pretending to be the system's. Using SF Symbols in portable code
takes a *pair* of gates that must agree — a target-scoped dependency and a cfg on every
use site — plus cfg-selected sibling functions rather than a cfg inside a tuple (stack
tuple arity cannot vary by cfg):

```toml
[target.'cfg(target_vendor = "apple")'.dependencies]
waterui-icons-sf-symbol = "…"
```

```rust
#[cfg(target_vendor = "apple")]
use waterui_icons_sf_symbol as sf;
```

## Material 3 with the Hydrolysis renderer

Hydrolysis (the self-drawn GPU renderer) gets widget chrome from a backend-neutral
`WidgetTheme`. For Material 3 output, install the theme package before running:

```rust
hydrolysis_m3::install(&mut env);        // light baseline
hydrolysis_m3::install_dark(&mut env);
```

Seed-based Material You theming takes an `Argb` seed (a tuple struct over `0xAARRGGBB`,
re-exported by `hydrolysis_m3`) and a `MaterialColorMode`:

```rust
use hydrolysis_m3::{Argb, MaterialColorMode, MaterialColorSource, install_with_color_schemes};

hydrolysis_m3::install_with_seed(&mut env, Argb(0xFF6750A4));                       // light
hydrolysis_m3::install_with_seed_mode(&mut env, Argb(0xFF6750A4), MaterialColorMode::Dark);

// Full control: build paired schemes from a source, install one by reference.
let source = MaterialColorSource::new(Argb(0xFF6750A4));   // variant, contrast, spec version
let schemes = source.schemes();                            // paired light/dark
install_with_color_schemes(&mut env, &schemes, MaterialColorMode::Light);
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
