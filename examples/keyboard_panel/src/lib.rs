//! Keyboard Panel Example - the keyboard safe-area region in action
//!
//! A messenger-style screen: a scrollable conversation above a composer
//! panel pinned to the bottom edge. When the software keyboard slides up:
//! - the composer avoids the keyboard region, so it rides the keyboard's
//!   top edge while staying inside the container (home indicator) region;
//! - the composer's fill paints through the translucent keyboard — a fill
//!   background extends into the safe-area bands its frame touches;
//! - the conversation's scroll surface extends under the keyboard band,
//!   so its content stays visible through a translucent keyboard instead
//!   of being clipped at the keyboard's top edge.
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
            peer_bubble("Are we still on for Saturday?"),
            self_bubble("Yes — trailhead at nine, right?"),
            peer_bubble("Nine works. The weather is supposed to be clear."),
            self_bubble("I'll bring coffee and the topo map."),
            peer_bubble("Can you grab me an extra water bottle?"),
            self_bubble("Done. Two litres in the side pocket."),
            peer_bubble("Perfect. See you there."),
            self_bubble("See you. Don't forget the permits this time."),
        ))
        .padding_with(12.0),
    )
}

/// The bottom panel: it does not ignore the safe area, so layout keeps it
/// clear of whichever band — container or keyboard — is deepest on its edge.
/// Its fill background is what reaches under the band instead.
fn composer(draft: Binding<Str>) -> impl View {
    hstack((
        field("Message", &draft),
        button("Send").action(move || draft.set(Str::from(""))),
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
