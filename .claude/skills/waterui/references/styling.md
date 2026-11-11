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
- Borders and shadows
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
use waterui::prelude::theme_color::{Accent, Foreground, Surface};

text("Title").foreground(Foreground)
text("Caption").caption().muted()   // `.muted()` is `.foreground(MutedForeground)`
card.background(Surface)
button("Delete").action(|| {}).tint(Color::red())   // this subtree's controls draw with red
```

`.tint(color)` sets the tint the controls in a subtree draw with — buttons, sliders,
progress indicators — instead of the theme's `Accent`. The nearest `.tint` wins, and a
subtree with none uses `Accent`. The color may be a signal, and a change updates the
controls in place.

The complete set:

| Token | Meaning |
|---|---|
| `Background` | Primary window or page background |
| `Surface` | Elevated surface — cards, sheets |
| `SurfaceVariant` | Alternate surface |
| `Border` | Borders and dividers |
| `Foreground` | Primary text and icons |
| `MutedForeground` | Secondary / dimmed text |
| `TertiaryForeground` | Tertiary text, fainter than `MutedForeground` |
| `DisabledForeground` | Text and glyphs of a control or item that cannot be used |
| `Accent` | Interactive accent |
| `AccentContainer` | Container associated with the accent |
| `AccentForeground` | Foreground drawn on accent backgrounds |
| `Tertiary` | Complementary emphasis accent |
| `TertiaryContainer` | Container associated with the tertiary accent |
| `SelectionContainer` | Fill painted behind a selected item |
| `SelectionForeground` | Foreground drawn on the selection container |
| `UnemphasizedSelection` | Fill behind a selected item while its control is not focused |
| `Error` | Destructive / error emphasis — badges, destructive buttons, validation |
| `ErrorForeground` | Foreground drawn on the error color |
| `Scrim` | Dimming backdrop a modal layer paints behind a presented dialog |

Each token is a zero-sized unit struct that is *also* a `View`, so `Surface.size(80.0, 40.0)`
paints a themed rectangle. To use a token as a *value* — say, one arm of a `.select` —
convert it: `let indicator: Color = SurfaceVariant.into();`.

Project-level defaults for these slots go in `Water.toml` under `[theme]` — see
`references/project.md`.

## Installing a theme (dark mode, custom fonts)

`Theme` is a builder and a `Plugin`; install it in `app(mut env)`. `.color_scheme(..)`
takes a **signal**, which is how a runtime dark-mode toggle works with no rebuild:

```rust
use waterui::text::font::{FontWeight, ResolvedFont};

pub fn app(mut env: Environment) -> App {
    let dark = Binding::bool(true);
    env.install(
        Theme::new()
            .color_scheme(dark.select(ColorScheme::Dark, ColorScheme::Light))
            .colors(ColorSettings::new().accent(Srgb::from_hex("#0066CC")))   // per-token setters
            .fonts(FontSettings::new().body(ResolvedFont::with_family(16.0, FontWeight::Normal, "Inter"))),
    );
    App::new(move || root(dark.clone()), env)
}
```

`ColorSettings` has one setter per token above (`.accent(..)`, `.muted_foreground(..)`, …,
`.scrim(..)`); `FontSettings` one per font slot (`.body`, `.title`, `.headline`,
`.subheadline`, `.caption`, `.footnote`), each taking a signal. Only the fields you set
overwrite the ambient theme; `FontSettings::default_scale()` starts from the full default
type scale instead of an empty override set. A custom family such as `Inter` is bundled by
declaring it under `[[assets.font]]` in `Water.toml` (see `references/project.md`).
To read the active scheme, `waterui::theme::current_color_scheme(&env)` returns a
`Computed<ColorScheme>` (a contract violation if no backend or theme installed one: panics in
debug, logs and yields light in release); `installed_color_scheme(&env)` returns
`Option<Computed<ColorScheme>>` for code that must tolerate its absence.
`env.install(theme)` on the app environment applies globally; `.install(theme)` on a view
scopes it to that subtree.

## Concrete colors

```rust
Color::srgb_hex("#3B82F6")
Color::srgb(59, 130, 246)          // 0-255 integers
Color::srgb_u32(0x3B82F6)          // 0xRRGGBB
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

`Color::srgb_hex` panics on a malformed string; `Color::try_srgb_hex(..)` returns a
`Result` for runtime input. Named material-palette colors are unit structs and views, and
each has a lower-case `Color` constructor (`Color::red()`, `Color::deep_orange()`, …):

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

Without it, `.fill(binding)` fails to type-check and `.fill(binding.snapshot())` freezes —
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

`Material` is the content-layer frosted pane (SwiftUI's `.regularMaterial`). `UltraThin` and
`Thin` blend what lies behind the window: Hydrolysis realizes them only as a window's
background (`Window::background`), and panics on them as a view's background. Liquid Glass
is a separate type, `Glass`, because it is a different surface with its own parameters,
not a sixth thickness. Glass carries its own outline — a capsule unless you say
otherwise — because a mask over glass would destroy its refraction; do not `.clip()` it.
Apple backends project it onto `UIGlassEffect` / `NSGlassEffectView`; other backends
approximate or ignore it.

`.material_group()` (a `ViewExt` modifier) groups the glass/material backgrounds inside a
subtree: members share one capture of what lies behind the group (so none sees another),
and a style that supports it may merge nearby members into one shape. The nearest enclosing
group wins; a material outside every group is a group of its own; a style that does not
group renders the content unchanged. Merge behaviour and distance belong to the style, never
the view. Example: `hstack((a.background(Glass::regular()), b.background(Glass::regular()))).material_group()`.

```rust
use waterui::background::Glass;

view.background(Glass::regular())                       // capsule pill, the default
view.background(Glass::clear().interactive(true))       // over media; reacts to touch
view.background(Glass::regular().tint(Accent).shape(RoundedRectangle::new(0.2)))
```

Gradients are views in `waterui::gradient` (the prelude carries `Gradient` and
`MeshGradient`), so they work anywhere a view does — on their own or inside
`.background(..)`. They are authored in unit space, `(0, 0)` the top-left, and take
`WorkingColor` (linear Display P3): `Srgb::from_hex(..).resolve()` makes one from a hex
colour, and `color::working::from_linear_srgb` from linear components, where values
above `1.0` are HDR.

```rust
use waterui::color::working::from_linear_srgb;
use waterui::gradient::{
    AnimatedMeshGradient, AnimatedMeshGradientConfig, FlowingGradient, Gradient, MeshGradient,
};

let rose = Srgb::from_hex("#FF4D80").resolve();
let sky = from_linear_srgb([0.3, 0.5, 1.0], 1.0);

Gradient::linear(vec![(0.0, rose), (1.0, sky)], [0.0, 0.0], [1.0, 1.0])   // fixed colours
view.background(Gradient::radial(vec![(0.0, rose), (1.0, sky)], [0.5, 0.5], 0.0, 0.7))
Gradient::angular(vec![(0.0, rose), (1.0, sky)], [0.5, 0.5], 0.0, core::f32::consts::TAU)   // radians
Gradient::mesh(2, 2, vertices, true)   // vertices: Vec<([f32; 2], WorkingColor)>, row-major
MeshGradient::new(3, 3, colors.clone()).size(300.0, 200.0)   // colors: any signal of WorkingColors
AnimatedMeshGradient::new(AnimatedMeshGradientConfig::aqua_bloom())   // flows with the canvas clock, on every backend
FlowingGradient::new()   // drifting noise bands, GPU only
```

A radial gradient's radii are fractions of the box's *shorter side*, so it stays circular
on a non-square box (the end radius must be above `0.0`); an angular sweep is above `0` and
at most `TAU`. The constructors panic on a violated bound. `MeshGradient` follows signals
for its colours, its control points (`.points(..)`) and its interpolation
(`.smooths_colors(..)`), and `AnimatedMeshGradient::new` takes a
signal of its whole configuration, so an animated gradient is a signal change, not a
rebuild. An animated mesh with `speed` `0.0` holds still and lets the engine idle.

## Shapes

A shape is an outline, not a view: `.fill(color)` turns it into one (a `FilledShape`,
which fills the space it is given), and `.clip(shape)` uses it as a mask. Two uses:

```rust
use waterui::shape::{Capsule, Circle, Ellipse, FixedRoundedRectangle,
                     FixedUnevenRoundedRectangle, Path, Rectangle, RoundedRectangle,
                     ShapeExt, UnevenRoundedRectangle};

Circle.fill(Color::srgb_hex("#3B82F6")).size(80.0, 80.0)   // draw the shape
photo.clip(Circle)                                          // clip a view to the shape
photo.clip(FixedRoundedRectangle::new(12.0))                // absolute points
```

**Corner radii are normalized, not points.** `RoundedRectangle::new(r)` takes a fraction
of the shape's *shorter side*: `0.1` is a subtle round, `0.5` is fully rounded, and
larger values silently saturate at 0.5. `RoundedRectangle::new(12.0)` therefore compiles
and produces a capsule — the single most common shape mistake. For "fully rounded at any
size", use `Capsule`. `UnevenRoundedRectangle::new(top_leading, top_trailing,
bottom_leading, bottom_trailing)` takes four normalized radii in reading order — *not*
clockwise. When a spec names the radius in points ("12dp card"), use
`FixedRoundedRectangle::new(points)` or `FixedUnevenRoundedRectangle::new(..)` (same
corner order): the radius stays constant as the shape resizes and clamps to half the
shorter side.

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

`quad_to(cx, cy, x, y)` and `cubic_to(c1x, c1y, c2x, c2y, x, y)` add curves, and
`arc(cx, cy, rx, ry, start, sweep)` an elliptical arc (radians, positive sweep
clockwise). Because coordinates are unit-space, a path-drawn corner stretches with the aspect ratio — that is why built-in shapes carry
their kind instead of path commands, and why shape code is tested against a deliberately
non-square rectangle, never a square.

Morphing is a self-animating view over the **built-in shapes only** (Rectangle,
Circle, Ellipse, RoundedRectangle, UnevenRoundedRectangle, Capsule and the two `Fixed…`
rounded rectangles — not `Path`, which panics); the
shapes' parameters interpolate and every backend draws the same path:

```rust
Circle
    .morph_to(RoundedRectangle::new(0.22), Color::srgb_hex("#3B82F6"))
    .duration(Duration::from_millis(600))
    .autoreverse(true)                       // also .repeat(bool)
    .size(90.0, 90.0)

shape.morph_to(target, fill).progress(t.clone())   // or drive it from a 0..=1 signal instead
```

`FilledShape::morph_to(target)` (no fill argument) keeps an already-applied fill.

## Borders and shadows

```rust
use waterui::border::Border;
use waterui::prelude::theme_color::Accent;
use waterui::shape::RoundedRectangle;

card.border(Color::srgb_hex("#E5E7EB"), 1.0)                       // all edges, square corners
card.border_with(Border::new(Accent, 1.0).corner_radius(12.0).edges(EdgeSet::BOTTOM))
card.shadow(Shadow::new(
    Color::srgb(0, 0, 0).with_opacity(0.2),   // color, with alpha
    Vector::new(0.0, 2.0),                    // offset
    8.0,                                      // blur radius
    RoundedRectangle::new(0.1),               // the caster's own shape
))
```

`Border`'s `corner_radius` is in **points** (unlike `RoundedRectangle`'s normalized radius)
and `Shadow`'s third argument is its blur radius. A shadow follows its `silhouette`
argument, so pass the shape the view is filled or clipped with, or the shadow will not
match its outline. `Shadow::splat(v)` is a black shadow of a rectangle.

## Floating surfaces

`.floating()` promotes any view to a themed elevated surface (shadow, clip radius,
insets) while preserving the wrapped control's semantic identity; `.floating_with(style)`
supplies explicit `FloatingStyle` tokens. Themes may install a `FloatingStyle` in the
environment — read it with `env.get::<FloatingStyle>()` (falling back to
`FloatingStyle::default()`) when custom chrome must match floating controls.

`FloatingStyle::elevation` is a `StateValue<FloatingElevation>` — the surface's
ambient and key shadows resolve against the state the control inside reports
through `.interaction_state`, so a themed FAB can lift on hover
(`elevation.when(InteractionState::HOVERED, ..)`) without view code wiring
anything.

## Icons

Icons come from packaged icon-set crates. **Pick one set per app** and depend on it:

| Crate | Set |
|---|---|
| `waterui-icons-material-icon` | Material Design Icons (Pictogrammers), SVG |
| `waterui-icons-lucide` | Lucide, SVG |
| `waterui-icons-fontawesome7` | Font Awesome 7 Free (webfont by default, SVG optional) — constants such as `fa::solid::HOUSE`, `.with_size(32.0)` |
| `waterui-icons-sf-symbol` | SF Symbols — **Apple platforms only** |
| `waterui-icons-native` | Curated SF Symbol names (`home()`, `settings()`, …) — **Apple platforms only** |

```rust
use waterui_icons_material_icon as mdi;
use waterui_icons_lucide as lucide;

mdi::check_circle()
mdi::delete().size(20.0, 20.0)
mdi::flag().foreground(Accent)                      // theme color
lucide::star().tint(Color::srgb_hex("#F59E0B"))     // explicit tint
```

Icon names are `snake_case` (`account-circle` → `account_circle()`); Material and Lucide
icons are 24×24 by default. Font Awesome's default webfont mode draws glyphs from the
Font Awesome font files (`fa-solid-900.ttf`, `fa-regular-400.ttf`, `fa-brands-400.ttf`),
which the app must bundle; its `svg` feature needs no font.

Match the set to the design language: a Material 3 app uses Material icons, not Lucide.

The `SystemIcon` type behind the SF Symbol and native packs (`waterui::icon`, name-based)
renders only on Apple platforms. There is deliberately no cross-platform "system icon" fallback. Apple ships SF Symbols and
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

Hydrolysis (the self-drawn GPU renderer) is styled by construction: every entry point
takes a `hydrolysis::Style` value, and `Material3` is the Material Design 3 one:

```rust
hydrolysis::run(app, env, hydrolysis_m3::Material3::defaults());   // light+dark baseline
hydrolysis::run(app, env, hydrolysis_m3::Material3::dark());
```

Tests take the same style through `theme =` — see `references/testing.md`:

```rust
#[waterui::test(view, theme = hydrolysis_m3::Material3::defaults(), offscreen)]
```

To install a scheme into a scoped environment instead of running the whole app under
it, call the `Style` trait's `install_tokens` directly:

```rust
use hydrolysis::Style;

Material3::with_colors(my_scheme).install_tokens(&mut scoped_env);
```

Seed-based Material You styles take an `Argb` seed (a tuple struct over `0xAARRGGBB`,
re-exported by `hydrolysis_m3`) and a `MaterialColorMode`:

```rust
use hydrolysis_m3::{Argb, Material3, MaterialColorMode, MaterialColorSource};

Material3::with_seed(Argb(0xFF6750A4));                               // light
Material3::with_seed_mode(Argb(0xFF6750A4), MaterialColorMode::Dark); // dark

// Full control: build paired schemes from a source, pick one mode.
let source = MaterialColorSource::new(Argb(0xFF6750A4));   // variant, contrast, spec version
let schemes = source.schemes();                          // paired light/dark
Material3::with_color_schemes(&schemes, MaterialColorMode::Light);
```

Material-specific role tokens live in `hydrolysis_m3::color::*` (`Primary`, `OnPrimary`,
`PrimaryContainer`, `SurfaceContainerHighest`, `OnSurfaceVariant`, …) and resolve from the
installed scheme. WaterUI's portable `theme_color::*` tokens map onto the active theme —
prefer the portable ones in views that should also run on native backends, and use the
Material roles only where the design is specifically Material.

To see it:

Declare `backend = "hydrolysis"` under `[platforms.<platform>]` in `Water.toml`,
or use the flag below if that platform has no declaration. A conflicting declaration
is an error; see [backend selection](project.md#platforms-and-backends).

```bash
water preview my_view --backend hydrolysis --theme material3 --output preview.png
```
