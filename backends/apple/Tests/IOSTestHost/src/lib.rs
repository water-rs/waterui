//! Fixture application for the device-hosted WaterUITests cases: a `List`
//! whose rows carry the reported nested-stack shape — `hstack(icon,
//! vstack(hstack(text, spacer, flag), text, text))` behind a `Label` — so a
//! linked test bundle can assert every nested text view lands with a real
//! frame inside its cell.
//!
//! `.github/scripts/run-ios-device-tests.sh ios_test_host` packages this
//! workspace member like a framework example and launches it on a booted
//! iOS simulator.

use waterui::app::App;
use waterui::component::list::{List, ListItem};
use waterui::prelude::theme_color::{Accent, Foreground};
use waterui::prelude::*;
use waterui::shape::Circle;

fn message_row(sender: &'static str, subject: &'static str, preview: &'static str) -> ListItem {
    ListItem::new(Label::new(
        Str::from(format!("{sender}: {subject}")),
        move || {
            hstack((
                hstack((Accent.size(8.0, 8.0).clip(Circle),)).size(8.0, 8.0),
                vstack((
                    hstack((
                        text(sender).sub_headline().foreground(Foreground),
                        spacer(),
                        text("flag").caption().muted(),
                    ))
                    .spacing(6.0),
                    text(subject).body().foreground(Foreground),
                    text(preview).caption().muted(),
                ))
                .leading()
                .spacing(2.0),
            ))
            .top()
            .spacing(6.0)
            .padding_vertical(8.0)
        },
    ))
}

fn inbox() -> impl View {
    List::content((
            || {
                message_row(
                    "Ada Lovelace",
                    "WaterUI render loop",
                    "The nested vstack inside this row must paint its text.",
                )
            },
            || {
                message_row(
                    "Grace Hopper",
                    "List cell layout",
                    "Three lines sit in a vstack nested in the row's hstack.",
                )
            },
            || {
                message_row(
                    "Edsger Dijkstra",
                    "Placement proposals",
                    "Every nested stack receives the width its parent proposes.",
                )
            },
            || {
                message_row(
                    "Barbara Liskov",
                    "Substitution",
                    "Cells measure correctly; their text must render too.",
                )
            },
            || {
                message_row(
                    "Margaret Hamilton",
                    "Priority display",
                    "Zero-width frames are the regression this fixture guards.",
                )
            },
        ))
}

pub fn app(env: Environment) -> App {
    App::new(inbox, env)
}
