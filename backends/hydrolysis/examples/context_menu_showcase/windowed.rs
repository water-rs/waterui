//! The example program, compiled where `hydrolysis::run` exists.

use std::thread;
use std::time::Duration;

use hydrolysis::run;
use waterui::Environment;
use waterui::app::App;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::theme::color::{Background, Surface};
use waterui::window::{Window, WindowState};
use waterui_controls::button::button;
use waterui_layout::frame::Frame;
use waterui_layout::stack::{hstack, vstack};

/// The accessory: a five-button reaction strip the menu anchors above the
/// lifted source.
fn reaction_strip() -> impl View {
    hstack((
        button("+1").action(|| {}),
        button("Ha").action(|| {}),
        button("Wow").action(|| {}),
        button("Sad").action(|| {}),
        button("Angry").action(|| {}),
    ))
    .spacing(8.0)
}

/// The context-menu source: a message card, centered in the window so a click
/// at the window's centre opens the menu over it.
fn message_card() -> impl View {
    Frame::new(
        vstack((
            text("Ava — design review").size(18.0),
            text("The new chip shapes are in. Can you look at the accessory strip?")
                .size(13.0)
                .muted(),
        ))
        .spacing(8.0)
        .padding(),
    )
    .width(420.0)
    .height(180.0)
    .background(Color::new(Surface))
    .context_menu(
        // `HYDROLYSIS_MENU_VARIANT=plain` drops the accessory for the pure
        // popup-window path captures.
        if std::env::var("HYDROLYSIS_MENU_VARIANT").as_deref() == Ok("plain") {
            ContextMenu::new(vec![
                "Reply".action(|| {}),
                "Forward"
                    .command()
                    .action(|| {})
                    .subtitle("Send to another chat"),
                "Delete".command().action(|| {}).destructive(),
            ])
        } else {
            ContextMenu::new(vec![
                "Reply".action(|| {}),
                "Forward"
                    .command()
                    .action(|| {})
                    .subtitle("Send to another chat"),
                "Delete".command().action(|| {}).destructive(),
            ])
            .accessory(reaction_strip())
        },
    )
}

fn main_view() -> impl View {
    // `HYDROLYSIS_CARD_POS` parks the card so each placement band shows:
    // "top" leaves room for the menu's default below-the-preview slot, and
    // "bottom" forces the menu above it.
    let (top, bottom) = match std::env::var("HYDROLYSIS_CARD_POS").as_deref() {
        Ok("top") => (180.0, 340.0),
        Ok("bottom") => (410.0, 110.0),
        _ => (260.0, 260.0),
    };
    vstack((
        ().size(0.0, top),
        hstack((spacer(), message_card(), spacer())),
        ().size(0.0, bottom),
    ))
    .background(Color::new(Background))
}

fn app(env: Environment) -> App {
    App::new_with_windows(
        [Window::new(
            "Hydrolysis Context Menu",
            binding(WindowState::Normal),
            main_view,
        )],
        env,
    )
}

pub fn main() {
    if let Ok(seconds) = std::env::var("HYDROLYSIS_SHOWCASE_SECONDS") {
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(
                seconds.parse::<u64>().expect("a whole number of seconds"),
            ));
            std::process::exit(0);
        });
    }

    // `HYDROLYSIS_DARK=1` pins the dark scheme for the dark-theme captures.
    let style = if std::env::var("HYDROLYSIS_DARK").as_deref() == Ok("1") {
        hydrolysis_m3::Material3::dark()
    } else {
        hydrolysis_m3::Material3::defaults()
    };
    run(app(Environment::new()), style);
}
