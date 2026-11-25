//! Icons Example - Demonstrates `WaterUI`'s Icon System
//!
//! This example showcases 3 icon packs:
//! - SF Symbols (Apple platforms only, native rendering)
//! - Material Design Icons (SVG rendering)
//! - Lucide Icons (SVG rendering)

use waterui::app::{App, Windows};
use waterui::prelude::*;
use waterui::preview;

#[cfg(target_vendor = "apple")]
use waterui_apple::SfSymbol;
use waterui_icons_lucide as lucide;
use waterui_icons_material_icon as mdi;

/// Demo: SF Symbols (Apple only)
#[cfg(target_vendor = "apple")]
fn sf_symbols_demo() -> impl View {
    vstack((
        text("SF Symbols (Apple)").font_size(18.0),
        hstack((
            SfSymbol::HOUSE_FILL,
            SfSymbol::PERSON_FILL,
            SfSymbol::GEARSHAPE,
            SfSymbol::HEART_FILL,
            SfSymbol::STAR_FILL,
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Material Design Icons (SVG)
fn material_icons_demo() -> impl View {
    use mdi::{account, cog, heart, home, star};

    vstack((
        text("Material Design Icons").font_size(18.0),
        hstack((home(), account(), cog(), heart(), star())).spacing(16.0),
    ))
    .padding()
}

/// Demo: Lucide Icons (SVG)
fn lucide_icons_demo() -> impl View {
    use lucide::{heart, house, settings, star, user};

    vstack((
        text("Lucide Icons").font_size(18.0),
        hstack((house(), user(), settings(), heart(), star())).spacing(16.0),
    ))
    .padding()
}

/// Demo: Colored icons
fn colored_icons_demo() -> impl View {
    use lucide::star;
    use mdi::{check_circle, heart, information};

    vstack((
        text("Colored Icons").font_size(18.0),
        hstack((
            heart()
                .foreground(Color::srgb_hex("#EF4444"))
                .size(32.0, 32.0),
            star()
                .foreground(Color::srgb_hex("#F59E0B"))
                .size(32.0, 32.0),
            check_circle()
                .foreground(Color::srgb_hex("#10B981"))
                .size(32.0, 32.0),
            information()
                .foreground(Color::srgb_hex("#3B82F6"))
                .size(32.0, 32.0),
        ))
        .spacing(16.0),
    ))
    .padding()
}

fn icon_demos() -> impl View {
    vstack((
        material_icons_demo(),
        Divider,
        lucide_icons_demo(),
        Divider,
        colored_icons_demo(),
    ))
}

#[cfg(target_vendor = "apple")]
fn all_demos() -> impl View {
    vstack((sf_symbols_demo(), Divider, icon_demos()))
}

#[cfg(not(target_vendor = "apple"))]
fn all_demos() -> impl View {
    icon_demos()
}

/// The icons example page, listing the icon packs available on this platform.
#[preview]
pub fn demo() -> impl View {
    scroll(
        vstack((
            text("WaterUI Icon Packs").font_size(28.0),
            text("Icon packs: SF Symbols, MDI, Lucide"),
            Divider,
            all_demos(),
        ))
        .padding_with(16.0),
    )
}

/// Builds the icons example app.
pub fn app(env: Environment) -> App<impl Windows> {
    App::new(env).window(Window::new("", demo))
}
