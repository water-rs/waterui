//! Keyboard Panel Example - the keyboard safe-area region in action
//!
//! A messenger-style screen: a scrollable conversation above a composer
//! panel pinned to the bottom edge. When the software keyboard slides up:
//! - the composer avoids the keyboard region, so it rides the keyboard's
//!   top edge while staying inside the container (home indicator) region;
//! - the composer's fill paints through the translucent keyboard — a fill
//!   background extends into the safe-area bands its frame touches;
//! - the scroll surface grows its content inset by the keyboard inset and
//!   scrolls the focused field the minimum distance that keeps it clear.
//!
//! `docs/layout-spec.md` §7.1 defines the two-region model; the example
//! uses only default behaviour — no per-view safe-area workarounds.

use waterui::app::App;
use waterui::graphics::color::Srgb;
use waterui::prelude::*;
use waterui::preview;

const SELF_BUBBLE: Srgb = Srgb::from_hex("#0B93F6");
const PEER_BUBBLE: Srgb = Srgb::from_hex("#E9E9EB");
const COMPOSER_FILL: Srgb = Srgb::from_hex("#F7F7F9");

fn self_bubble(message: &'static str) -> impl View {
    hstack((
        spacer(),
        text(message)
            .body()
            .foreground(Srgb::WHITE)
            .padding_with(10.0)
            .background(SELF_BUBBLE),
    ))
}

fn peer_bubble(message: &'static str) -> impl View {
    hstack((
        text(message)
            .body()
            .padding_with(10.0)
            .background(PEER_BUBBLE),
        spacer(),
    ))
}

/// The conversation scroll — a scroll surface touching the bottom edge keeps
/// its content inside the container inset and extends under the keyboard.
fn conversation() -> impl View {
    scroll(
        vstack((
            peer_bubble("Did the keyboard-safe-area change land?"),
            self_bubble("Just pushed — the composer now tracks the IME."),
            peer_bubble("Show me."),
            self_bubble("Focus the field below: the keyboard region is its own safe area."),
            peer_bubble("The panel rides the keyboard top — nice."),
            self_bubble("And the panel fill runs underneath it, so a translucent keyboard shows it through."),
            peer_bubble("Scroll a longer thread and focus again — the field stays clear."),
            self_bubble("It scrolls the minimum distance to clear the field's frame."),
        ))
        .padding_with(12.0),
    )
}

/// The bottom panel: it does not ignore the safe area, so layout keeps it
/// clear of whichever band — container or keyboard — is deepest on its edge.
/// Its fill background is what reaches under the band instead.
fn composer(draft: Binding<Str>) -> impl View {
    let draft_for_field = draft.clone();
    let draft_for_send = draft;
    hstack((
        field("Message", &draft_for_field),
        button("Send").action(move || {
            draft_for_send.set(Str::from(""));
        }),
    ))
    .padding_with(12.0)
    .background(COMPOSER_FILL)
}

#[preview]
fn main() -> impl View {
    let draft = Binding::container(Str::from(""));
    vstack((conversation(), composer(draft)))
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
