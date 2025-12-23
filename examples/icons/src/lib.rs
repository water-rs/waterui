//! Icons Example - Demonstrates WaterUI's icon system
//!
//! This example showcases:
//! - SF Symbols for Apple platforms
//! - Material Design Icons
//! - Font Awesome 7 icons
//! - Cross-platform native icons

use waterui::app::App;
use waterui::prelude::*;

use waterui_icons_fontawesome7 as fa;
use waterui_icons_material_icon as mdi;
use waterui_icons_native as icons;
use waterui_icons_sf_symbol as sf;

fn icon_row(label: &'static str, icon: impl View) -> impl View {
    hstack((text(label), spacer(), icon))
}

fn sf_symbols_section() -> impl View {
    vstack((
        text("SF Symbols (Apple)").sub_headline(),
        icon_row("House", sf::HOUSE),
        icon_row("Gear", sf::GEAR),
        icon_row("Person", sf::PERSON),
        icon_row("Star", sf::STAR),
        icon_row("Heart", sf::HEART),
        icon_row("Bell", sf::BELL),
    ))
}

fn material_icons_section() -> impl View {
    vstack((
        text("Material Design Icons").sub_headline(),
        icon_row("Home", mdi::home()),
        icon_row("Settings", mdi::cog()),
        icon_row("Account", mdi::account()),
        icon_row("Star", mdi::star()),
        icon_row("Heart", mdi::heart()),
        icon_row("Bell", mdi::bell()),
    ))
}

fn fontawesome_section() -> impl View {
    vstack((
        text("Font Awesome 7 - Solid").sub_headline(),
        icon_row("House", fa::solid::house()),
        icon_row("Gear", fa::solid::gear()),
        icon_row("User", fa::solid::user()),
        icon_row("Star", fa::solid::star()),
        icon_row("Heart", fa::solid::heart()),
        Divider,
        text("Font Awesome 7 - Brands").sub_headline(),
        icon_row("GitHub", fa::brands::github()),
        icon_row("Apple", fa::brands::apple()),
        icon_row("Android", fa::brands::android()),
    ))
}

fn native_icons_section() -> impl View {
    vstack((
        text("Native (Cross-platform)").sub_headline(),
        icon_row("Home", icons::HOME),
        icon_row("Settings", icons::SETTINGS),
        icon_row("Search", icons::SEARCH),
        icon_row("Person", icons::PERSON),
        icon_row("Star", icons::STAR),
        icon_row("Heart", icons::HEART),
        icon_row("Add", icons::ADD),
        icon_row("Delete", icons::DELETE),
    ))
}

fn main() -> impl View {
    scroll(
        vstack((
            text("WaterUI Icons").title(),
            "Comprehensive icon system with multiple libraries",
            Divider,
            sf_symbols_section(),
            Divider,
            material_icons_section(),
            Divider,
            fontawesome_section(),
            Divider,
            native_icons_section(),
            Divider,
            text("Icon Attribution").footnote(),
            text("Material Design Icons by Pictogrammers - Apache 2.0").caption(),
            text("Font Awesome Free by Fonticons, Inc. - CC BY 4.0").caption(),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main(), env)
}

waterui_ffi::export!();
