//! Snippets from `.claude/skills/waterui/references/styling.md`, in file order.
//! Transcription conventions are documented in the crate README.
//!
//! The two Material 3 blocks need the `hydrolysis-m3` dev-dependency, so they
//! live behind the `compile-gate-tests` feature at the bottom of this file.

use waterui::prelude::*;

// ---------------------------------------------------------------------------
// styling.md § "## Theme color tokens" — rust block 1/15
// Listing: three token applications.
// ---------------------------------------------------------------------------
pub fn styling_block_01() {
    use waterui::prelude::theme_color::{Accent, Foreground, MutedForeground, Surface};

    let _: Option<Accent> = None;

    let _ = { text("Title").foreground(Foreground) };
    let _ = { text("Caption").caption().foreground(MutedForeground) };
    let card = Divider;
    let _ = { card.background(Surface) };
}

// ---------------------------------------------------------------------------
// styling.md § "## Theme color tokens" (prose): converting a token to a value,
// `let indicator: Color = SurfaceVariant.into();`, and the complete token set.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn styling_token_as_value_prose() {
    use waterui::prelude::theme_color::{
        AccentContainer, AccentForeground, Background, Border, SelectionContainer,
        SelectionForeground, SurfaceVariant, Tertiary, TertiaryContainer,
    };

    let indicator: Color = SurfaceVariant.into();
    let _ = indicator;

    let _ = (Background, Border, AccentContainer, AccentForeground);
    let _ = (
        Tertiary,
        TertiaryContainer,
        SelectionContainer,
        SelectionForeground,
    );
    let _ = SurfaceVariant.size(80.0, 40.0);
}

// ---------------------------------------------------------------------------
// styling.md § "## Installing a theme (dark mode, custom fonts)" — rust block 2/15
// ---------------------------------------------------------------------------
pub mod styling_block_02 {
    use waterui::app::App;
    use waterui::prelude::*;

    fn root(_dark: Binding<bool>) -> impl View {
        text("root")
    }

    pub fn app(mut env: Environment) -> App {
        let dark = Binding::bool(true);
        env.install(
            Theme::new()
                .color_scheme(dark.select(ColorScheme::Dark, ColorScheme::Light))
                .fonts(FontSettings::new()), // per-slot setters, each a signal slot
        );
        App::new(move || root(dark.clone()), env)
    }
}

// ---------------------------------------------------------------------------
// styling.md § "## Concrete colors" — rust block 3/15
// Listing: six `Color` constructors, then the `Srgb` group.
// ---------------------------------------------------------------------------
pub fn styling_block_03() {
    let (r, g, b) = (0.23_f32, 0.51_f32, 0.96_f32);
    let (l, c, h) = (0.6_f32, 0.15_f32, 260.0_f32);

    let _ = { Color::srgb_hex("#3B82F6") };
    let _ = {
        Color::srgb(59, 130, 246) // 0-255 integers
    };
    let _ = { Color::srgb_f32(0.23, 0.51, 0.96) };
    let _ = { Color::p3(r, g, b) };
    let _ = { Color::oklch(l, c, h) };
    let _ = { Color::transparent() };

    use waterui::color::Srgb;
    const BRAND: Srgb = Srgb::from_hex("#3B82F6"); // const-evaluable — works in const fn too
    let _ = {
        Srgb::new(0.8, 0.9, 1.0) // 0.0-1.0 floats (unlike Color::srgb)
    };
    let _ = { Srgb::WHITE };
    let _ = { Srgb::BLACK };

    let _ = BRAND;
}

// ---------------------------------------------------------------------------
// styling.md § "## Concrete colors" (prose): `Srgb` is itself a view and a
// valid `.foreground()` / `.background()` argument, and the color transforms.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn styling_color_transforms_prose() {
    use waterui::color::Srgb;

    let _ = Divider.foreground(Srgb::from_hex("#3B82F6"));
    let _ = Divider.background(Srgb::from_hex("#3B82F6"));
    let _ = Color::from(Srgb::from_hex("#3B82F6"));

    let c = Color::srgb_hex("#3B82F6");
    let _ = c.clone().lighten(0.1);
    let _ = c.clone().darken(0.1);
    let _ = c.clone().saturate(0.1);
    let _ = c.clone().desaturate(0.1);
    let _ = c.clone().hue_rotate(30.0);
    let _ = c.clone().mix(Color::transparent(), 0.5);
    let _ = c.with_opacity(Binding::f32(0.5));
}

// ---------------------------------------------------------------------------
// styling.md § "## Concrete colors" — rust block 4/15
//
// A bare list of names. Each is a unit struct that is also a `View`, so each is
// transcribed as its own expression.
// ---------------------------------------------------------------------------
pub fn styling_block_04() {
    let _ = Red;
    let _ = Pink;
    let _ = Purple;
    let _ = DeepPurple;
    let _ = Indigo;
    let _ = Blue;
    let _ = LightBlue;
    let _ = Cyan;
    let _ = Teal;
    let _ = Green;
    let _ = LightGreen;
    let _ = Lime;
    let _ = Yellow;
    let _ = Amber;
    let _ = Orange;
    let _ = DeepOrange;
    let _ = Brown;
    let _ = Grey;
    let _ = BlueGrey;
}

// ---------------------------------------------------------------------------
// styling.md § "## Concrete colors" — rust block 5/15
// Listing: three uses of a named palette color.
// ---------------------------------------------------------------------------
pub fn styling_block_05() {
    let view = Divider;
    let _ = { view.background(Blue) };
    let _ = {
        Blue.size(80.0, 80.0) // a colored rectangle
    };
    let _ = { Blue.with_opacity(0.5) };
}

// ---------------------------------------------------------------------------
// styling.md § "## Reactive colors" — rust block 6/15
// ---------------------------------------------------------------------------
pub fn styling_block_06() -> impl View {
    use waterui::prelude::theme_color::SurfaceVariant;
    use waterui::shape::{Rectangle, ShapeExt};

    let selected = Binding::bool(true);
    let indicator: Color = SurfaceVariant.into();
    let clear = Color::transparent();

    let fill = signal_color(selected.select(indicator, clear).computed());
    Rectangle.fill(fill).size(64.0, 32.0)
}

// ---------------------------------------------------------------------------
// styling.md § "## Backgrounds, materials, gradients" — rust block 7/15
// ---------------------------------------------------------------------------
pub fn styling_block_07() {
    use waterui::prelude::theme_color::Surface;
    use waterui::shape::{RoundedRectangle, ShapeExt};

    use waterui::background::Material;

    let view = Divider;
    let _ = {
        view.background(Surface) // a color or token
    };
    let view = Divider;
    let _ = {
        view.background(Material::Regular) // platform blur material
    };
    // Material::UltraThin | Thin | Regular | Thick | UltraThick
    let _ = (
        Material::UltraThin,
        Material::Thin,
        Material::Thick,
        Material::UltraThick,
    );
    let view = Divider;
    let _ = {
        // any view is a valid background
        view.background(RoundedRectangle::new(0.18).fill(Surface))
    };
}

// ---------------------------------------------------------------------------
// styling.md § "## Backgrounds, materials, gradients" (prose): the prelude's
// background-descriptor gradient family. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn styling_prelude_gradients_prose() {
    use waterui::gradient::{
        AngularGradient, ColorStop, LinearGradient, MeshGradient, MeshVertex, RadialGradient,
        UnitPoint,
    };

    let _: Option<AngularGradient> = None;
    let _: Option<ColorStop> = None;
    let _: Option<LinearGradient> = None;
    let _: Option<MeshGradient> = None;
    let _: Option<MeshVertex> = None;
    let _: Option<RadialGradient> = None;
    let _: Option<UnitPoint> = None;
}

// ---------------------------------------------------------------------------
// styling.md § "## Backgrounds, materials, gradients" — rust block 8/15
// ---------------------------------------------------------------------------
pub fn styling_block_08() {
    use waterui_graphics::{
        AnimatedMeshGradient, AnimatedMeshGradientConfig, MeshGradient, ResolvedColor,
    };

    // Stops take ResolvedColor: a plain struct of five public f32 fields (struct-literal it).
    let stop = ResolvedColor {
        red: 1.0,
        green: 0.3,
        blue: 0.5,
        opacity: 1.0,
        headroom: 0.0,
    };

    let colors = Binding::container(vec![stop; 9]);

    let _ = {
        // colors: any signal of ResolvedColors
        MeshGradient::new(3, 3, colors.clone()).size(300.0, 200.0)
    };
    let _ = {
        // animates in-shader, zero CPU
        AnimatedMeshGradient::new(AnimatedMeshGradientConfig::aqua_bloom())
    };
}

// ---------------------------------------------------------------------------
// styling.md § "## Shapes" — rust block 9/15
// ---------------------------------------------------------------------------
pub fn styling_block_09() {
    let photo = Divider;

    use waterui::shape::{
        Capsule, Circle, Ellipse, Path, Rectangle, RoundedRectangle, ShapeExt,
        UnevenRoundedRectangle,
    };

    let _ = {
        Circle.fill(Color::srgb_hex("#3B82F6")).size(80.0, 80.0) // draw the shape
    };
    let _ = {
        photo.clip(Circle) // clip a view to the shape
    };

    let _: Option<Capsule> = None;
    let _: Option<Ellipse> = None;
    let _: Option<Path> = None;
    let _: Option<Rectangle> = None;
    let _: Option<RoundedRectangle> = None;
    let _ = UnevenRoundedRectangle::new(0.1, 0.1, 0.2, 0.2);
    fn needs_shape_ext<S: ShapeExt>(_s: S) {}
    needs_shape_ext(Circle);
}

// ---------------------------------------------------------------------------
// styling.md § "## Shapes" — rust block 10/15
// ---------------------------------------------------------------------------
pub fn styling_block_10() -> impl View {
    use waterui::prelude::theme_color::Accent;
    use waterui::shape::{Path, ShapeExt};

    let triangle = Path::new()
        .move_to(0.5, 0.0)
        .line_to(1.0, 1.0)
        .line_to(0.0, 1.0)
        .close();
    triangle.fill(Accent).size(80.0, 80.0)
}

// ---------------------------------------------------------------------------
// styling.md § "## Shapes" (prose): `cubic_to(c1x, c1y, c2x, c2y, x, y)`.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn styling_path_cubic_prose() {
    use waterui::shape::Path;

    let _ = Path::new()
        .move_to(0.0, 0.0)
        .cubic_to(0.2, 0.0, 0.8, 1.0, 1.0, 1.0);
}

// ---------------------------------------------------------------------------
// styling.md § "## Shapes" — rust block 11/15
// Listing: the fill-carrying form, then the progress-driven form.
// ---------------------------------------------------------------------------
pub fn styling_block_11() {
    use core::time::Duration;
    use waterui::shape::{Circle, Rectangle, RoundedRectangle, ShapeExt};

    let shape = Circle;
    let target = Rectangle;
    let fill = Color::srgb_hex("#22C55E");
    let t = Binding::f32(0.5);

    let _ = {
        Circle
            .morph_to(RoundedRectangle::new(0.22), Color::srgb_hex("#3B82F6"))
            .duration(Duration::from_millis(600))
            .autoreverse(true) // also .repeat(bool)
            .size(90.0, 90.0)
    };

    let _ = {
        // or drive it from a 0..=1 signal instead
        shape.morph_to(target, fill).progress(t.clone())
    };

    // `.repeat(bool)`, named in the trailing comment.
    let _ = Circle
        .morph_to(Rectangle, Color::transparent())
        .repeat(true);
}

// ---------------------------------------------------------------------------
// styling.md § "## Shapes" (prose): `FilledShape::morph_to(target)` with no
// fill argument keeps an already-applied fill. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn styling_filled_shape_morph_prose() {
    use waterui::shape::{Circle, Rectangle, ShapeExt};

    let _ = Circle.fill(Color::transparent()).morph_to(Rectangle);
}

// ---------------------------------------------------------------------------
// styling.md § "## Floating surfaces" (prose): `.floating()`,
// `.floating_with(style)`, and reading a `FloatingStyle` from the environment.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn styling_floating_prose() {
    use waterui::style::FloatingStyle;

    let _ = Divider.floating();
    let _ = Divider.floating_with(FloatingStyle::default());

    let env = Environment::new();
    let _ = env.get::<FloatingStyle>().cloned().unwrap_or_default();
}

// ---------------------------------------------------------------------------
// styling.md § "## Icons" — rust block 12/15
// ---------------------------------------------------------------------------
pub fn styling_block_12() {
    use waterui::prelude::theme_color::Accent;

    use waterui_icons_lucide as lucide;
    use waterui_icons_material_icon as mdi;

    let _ = { mdi::check_circle() };
    let _ = { mdi::delete().size(20.0, 20.0) };
    let _ = {
        mdi::flag().foreground(Accent) // theme color
    };
    let _ = {
        lucide::star().tint(Color::srgb_hex("#F59E0B")) // explicit tint
    };
}

// ---------------------------------------------------------------------------
// styling.md § "## Icons" — rust block 13/15
//
//     #[cfg(target_vendor = "apple")]
//     use waterui_icons_sf_symbol as sf;
//
// NOT COMPILABLE BY DESIGN here: proving it would mean adding the
// target-scoped `waterui-icons-sf-symbol` dependency the snippet's companion
// TOML fence declares, and this crate is not built for Apple targets only.
// Recorded rather than transcribed.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// styling.md § "## Material 3 with the Hydrolysis renderer" — blocks 14 and 15
// need the `hydrolysis-m3` dev-dependency, so they sit behind the
// `compile-gate-tests` feature. They must never be executed.
// ---------------------------------------------------------------------------
#[cfg(all(test, feature = "compile-gate-tests"))]
mod material3 {
    use waterui::prelude::*;

    // -----------------------------------------------------------------------
    // styling.md § "## Material 3" — rust block 14/15
    // Listing: two installer alternatives.
    // -----------------------------------------------------------------------
    #[test]
    fn styling_block_14() {
        {
            let mut env = Environment::new();
            hydrolysis_m3::install(&mut env); // light baseline
        }
        {
            let mut env = Environment::new();
            hydrolysis_m3::install_dark(&mut env);
        }
    }

    // -----------------------------------------------------------------------
    // styling.md § "## Material 3" — rust block 15/15
    // -----------------------------------------------------------------------
    #[test]
    fn styling_block_15() {
        let mut env = Environment::new();

        use hydrolysis_m3::{
            Argb, MaterialColorMode, MaterialColorSource, install_with_color_schemes,
        };

        hydrolysis_m3::install_with_seed(&mut env, Argb(0xFF6750A4)); // light
        hydrolysis_m3::install_with_seed_mode(&mut env, Argb(0xFF6750A4), MaterialColorMode::Dark);

        // Full control: build paired schemes from a source, install one by reference.
        let source = MaterialColorSource::new(Argb(0xFF6750A4)); // variant, contrast, spec version
        let schemes = source.schemes(); // paired light/dark
        install_with_color_schemes(&mut env, &schemes, MaterialColorMode::Light);
    }

    // -----------------------------------------------------------------------
    // styling.md § "## Material 3" (prose): Material role tokens in
    // `hydrolysis_m3::color::*`. Not counted as a rust block.
    // -----------------------------------------------------------------------
    #[test]
    fn styling_material_role_tokens() {
        use hydrolysis_m3::color::{
            OnPrimary, OnSurfaceVariant, Primary, PrimaryContainer, SurfaceContainerHighest,
        };

        let _ = Primary;
        let _ = OnPrimary;
        let _ = PrimaryContainer;
        let _ = SurfaceContainerHighest;
        let _ = OnSurfaceVariant;
    }
}
