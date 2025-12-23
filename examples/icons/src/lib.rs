//! Icons Example - Demonstrates WaterUI's Icon System
//!
//! This example showcases Font Awesome 7 webfont icons.
//!
//! # Font Requirements
//!
//! To see the icons render correctly, bundle the Font Awesome webfonts:
//! - `fa-solid-900.ttf` → "FontAwesome7Free-Solid"
//! - `fa-regular-400.ttf` → "FontAwesome7Free-Regular"
//! - `fa-brands-400.ttf` → "FontAwesome7Free-Brands"

use waterui::app::App;
use waterui::prelude::*;
use waterui_icons_fontawesome7 as fa;

/// Demo: Solid icons
fn solid_icons_demo() -> impl View {
    vstack((
        text("Solid Icons").size(18.0),
        hstack((
            fa::solid::HOUSE.with_size(32.0),
            fa::solid::USER.with_size(32.0),
            fa::solid::GEAR.with_size(32.0),
            fa::solid::HEART.with_size(32.0),
            fa::solid::STAR.with_size(32.0),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Regular icons
fn regular_icons_demo() -> impl View {
    vstack((
        text("Regular Icons").size(18.0),
        hstack((
            fa::regular::HEART.with_size(32.0),
            fa::regular::STAR.with_size(32.0),
            fa::regular::BELL.with_size(32.0),
            fa::regular::BOOKMARK.with_size(32.0),
            fa::regular::USER.with_size(32.0),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Brand icons
fn brand_icons_demo() -> impl View {
    vstack((
        text("Brand Icons").size(18.0),
        hstack((
            fa::brands::GITHUB.with_size(32.0),
            fa::brands::TWITTER.with_size(32.0),
            fa::brands::APPLE.with_size(32.0),
            fa::brands::GOOGLE.with_size(32.0),
            fa::brands::DISCORD.with_size(32.0),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Icon sizes
fn icon_sizes_demo() -> impl View {
    vstack((
        text("Icon Sizes").size(18.0),
        hstack((
            fa::solid::STAR.with_size(16.0),
            fa::solid::STAR.with_size(24.0),
            fa::solid::STAR.with_size(32.0),
            fa::solid::STAR.with_size(48.0),
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
                .foreground(Color::srgb_hex("#EF4444")),
            fa::solid::STAR
                .with_size(32.0)
                .foreground(Color::srgb_hex("#F59E0B")),
            fa::solid::CIRCLE_CHECK
                .with_size(32.0)
                .foreground(Color::srgb_hex("#10B981")),
            fa::solid::CIRCLE_INFO
                .with_size(32.0)
                .foreground(Color::srgb_hex("#3B82F6")),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Navigation items
fn navigation_demo() -> impl View {
    vstack((
        text("Navigation").size(18.0),
        vstack((
            hstack((
                fa::solid::HOUSE.with_size(24.0),
                text("Home"),
                spacer(),
                fa::solid::CHEVRON_RIGHT.with_size(16.0),
            ))
            .spacing(12.0),
            hstack((
                fa::solid::USER.with_size(24.0),
                text("Profile"),
                spacer(),
                fa::solid::CHEVRON_RIGHT.with_size(16.0),
            ))
            .spacing(12.0),
            hstack((
                fa::solid::GEAR.with_size(24.0),
                text("Settings"),
                spacer(),
                fa::solid::CHEVRON_RIGHT.with_size(16.0),
            ))
            .spacing(12.0),
        ))
        .spacing(8.0),
    ))
    .padding()
}

#[hot_reload]
fn main() -> impl View {
    scroll(
        vstack((
            text("WaterUI Icon Examples").size(28.0),
            Divider,
            solid_icons_demo(),
            Divider,
            regular_icons_demo(),
            Divider,
            brand_icons_demo(),
            Divider,
            icon_sizes_demo(),
            Divider,
            colored_icons_demo(),
            Divider,
            navigation_demo(),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
