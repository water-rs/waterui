//! Icons Example - Demonstrates WaterUI's Icon System
//!
//! This example showcases 4 icon packs:
//! - SF Symbols (Apple platforms only, native rendering)
//! - Material Design Icons (SVG rendering)
//! - Font Awesome 7 (webfont rendering)
//! - Lucide Icons (SVG rendering)

use waterui::app::App;
use waterui::prelude::*;

use waterui_icons_fontawesome7 as fa;
use waterui_icons_lucide as lucide;
use waterui_icons_material_icon as mdi;
#[cfg(target_vendor = "apple")]
use waterui_icons_sf_symbol as sf;

/// Demo: SF Symbols (Apple only)
#[cfg(target_vendor = "apple")]
fn sf_symbols_demo() -> impl View {
    vstack((
        text("SF Symbols (Apple)").size(18.0),
        hstack((
            sf::HOUSE_FILL,
            sf::PERSON_FILL,
            sf::GEARSHAPE,
            sf::HEART_FILL,
            sf::STAR_FILL,
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Material Design Icons (SVG)
fn material_icons_demo() -> impl View {
    vstack((
        text("Material Design Icons").size(18.0),
        hstack((
            mdi::home(),
            mdi::account(),
            mdi::cog(),
            mdi::heart(),
            mdi::star(),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Lucide Icons (SVG)
fn lucide_icons_demo() -> impl View {
    vstack((
        text("Lucide Icons").size(18.0),
        hstack((
            lucide::house(),
            lucide::user(),
            lucide::settings(),
            lucide::heart(),
            lucide::star(),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Font Awesome Solid icons (webfont)
fn fa_solid_demo() -> impl View {
    vstack((
        text("Font Awesome Solid").size(18.0),
        hstack((
            fa::solid::HOUSE.with_size(24.0),
            fa::solid::USER.with_size(24.0),
            fa::solid::GEAR.with_size(24.0),
            fa::solid::HEART.with_size(24.0),
            fa::solid::STAR.with_size(24.0),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Font Awesome Regular icons (webfont)
fn fa_regular_demo() -> impl View {
    vstack((
        text("Font Awesome Regular").size(18.0),
        hstack((
            fa::regular::HEART.with_size(24.0),
            fa::regular::STAR.with_size(24.0),
            fa::regular::BELL.with_size(24.0),
            fa::regular::BOOKMARK.with_size(24.0),
            fa::regular::USER.with_size(24.0),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Font Awesome Brand icons (webfont)
fn fa_brands_demo() -> impl View {
    vstack((
        text("Font Awesome Brands").size(18.0),
        hstack((
            fa::brands::GITHUB.with_size(24.0),
            fa::brands::TWITTER.with_size(24.0),
            fa::brands::APPLE.with_size(24.0),
            fa::brands::GOOGLE.with_size(24.0),
            fa::brands::DISCORD.with_size(24.0),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Colored icons
fn colored_icons_demo() -> impl View {
    vstack((
        text("Colored Icons").size(18.0),
        hstack((
            fa::solid::HEART
                .with_size(32.0)
                .color(Color::srgb_hex("#EF4444")),
            fa::solid::STAR
                .with_size(32.0)
                .color(Color::srgb_hex("#F59E0B")),
            fa::solid::CIRCLE_CHECK
                .with_size(32.0)
                .color(Color::srgb_hex("#10B981")),
            fa::solid::CIRCLE_INFO
                .with_size(32.0)
                .color(Color::srgb_hex("#3B82F6")),
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
        fa_solid_demo(),
        Divider,
        fa_regular_demo(),
        Divider,
        fa_brands_demo(),
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

#[hot_reload]
fn main() -> impl View {
    scroll(
        vstack((
            text("WaterUI Icon Packs").size(28.0),
            text("4 icon packs: SF Symbols, MDI, FA7, Lucide"),
            Divider,
            all_demos(),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
