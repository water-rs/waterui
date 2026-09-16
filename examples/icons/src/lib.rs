//! Icons Example - Demonstrates WaterUI's Icon System
//!
//! This example showcases 3 icon packs:
//! - SF Symbols (Apple platforms only, native rendering)
//! - Material Design Icons (SVG rendering)
//! - Lucide Icons (SVG rendering)

use waterui::app::App;
use waterui::prelude::*;
use waterui::preview;

use waterui_icons_lucide as lucide;
use waterui_icons_material_icon as mdi;
#[cfg(target_vendor = "apple")]
use waterui_icons_sf_symbol as sf;

/// Demo: SF Symbols (Apple only)
#[cfg(target_vendor = "apple")]
fn sf_symbols_demo() -> impl View {
    use sf::{gearshape, heart_fill, house_fill, person_fill, star_fill};

    vstack((
        text("SF Symbols (Apple)").size(18.0),
        hstack((
            house_fill(),
            person_fill(),
            gearshape(),
            heart_fill(),
            star_fill(),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Material Design Icons (SVG)
fn material_icons_demo() -> impl View {
    use mdi::{account, cog, heart, home, star};

    vstack((
        text("Material Design Icons").size(18.0),
        hstack((home(), account(), cog(), heart(), star())).spacing(16.0),
    ))
    .padding()
}

/// Demo: Lucide Icons (SVG)
fn lucide_icons_demo() -> impl View {
    use lucide::{heart, house, settings, star, user};

    vstack((
        text("Lucide Icons").size(18.0),
        hstack((house(), user(), settings(), heart(), star())).spacing(16.0),
    ))
    .padding()
}

/// Demo: Colored icons
fn colored_icons_demo() -> impl View {
    use lucide::star;
    use mdi::{check_circle, heart, information};

    vstack((
        text("Colored Icons").size(18.0),
        hstack((
            heart().tint(Color::srgb_hex("#EF4444")).size(32.0, 32.0),
            star().tint(Color::srgb_hex("#F59E0B")).size(32.0, 32.0),
            check_circle()
                .tint(Color::srgb_hex("#10B981"))
                .size(32.0, 32.0),
            information()
                .tint(Color::srgb_hex("#3B82F6"))
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

#[preview]
pub fn demo() -> impl View {
    scroll(
        vstack((
            text("WaterUI Icon Packs").size(28.0),
            text("Icon packs: SF Symbols, MDI, Lucide"),
            Divider,
            all_demos(),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
