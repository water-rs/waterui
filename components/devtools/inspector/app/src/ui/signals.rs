//! The reactive graph.
//!
//! `WaterUI`'s claim is that state updates are fine-grained. This pane is where
//! that claim is checked: every state-owning node, where it was created, how
//! many watchers it has, and how often it has fired. A node near the top of this
//! list that you did not expect to be busy is a bug you can now see.

use waterui::prelude::theme_color::MutedForeground;
use waterui::prelude::*;
use waterui::reactive::collection::SignalCollection;
use waterui::widget::condition::when;
use waterui_inspector_protocol::ChannelSet;

use crate::model::{Model, SignalRow};

use super::parts;

/// The signals pane.
#[expect(
    clippy::needless_pass_by_value,
    reason = "view builders take owned reactive handles: the views they return capture them for 'static, so a borrow would not do"
)]
pub fn view(model: Model) -> impl View {
    let unavailable = model
        .available
        .map(|available| !available.contains(ChannelSet::SIGNALS))
        .computed();
    let unsubscribed = model
        .subscribed
        .map(|subscribed| !subscribed.contains(ChannelSet::SIGNALS))
        .computed();
    let empty = model.signals.map(|rows| rows.is_empty()).computed();

    vstack((
        vstack((
            parts::section_header(
                "Reactive graph",
                "State-owning nodes, busiest first. Combinators are edges and do not appear.",
            ),
            totals(&model),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(8.0)
        .padding_with(EdgeInsets::all(16.0)),
        Divider,
        when(unavailable, || {
            parts::empty_state(
                "This build of the target has no reactive-graph tracing. \
                 Rebuild it with the `inspector-signals` feature.",
            )
        })
        .or(
            empty
                .clone()
                .zip(&unsubscribed)
                .map(|(empty, off)| empty && off)
                .computed(),
            || parts::empty_state("Open the signals channel in Overview to start tracing."),
        )
        .or(empty, || {
            parts::empty_state("No reactive state observed yet.")
        })
        .otherwise({
            let signals = model.signals.clone();
            move || List::for_each(SignalCollection::new(signals.clone()), row_view)
        }),
    ))
    .alignment(HorizontalAlignment::Leading)
}

/// Graph-wide figures, which are what you look at before any single node.
fn totals(model: &Model) -> impl View {
    let signals = model.signals.clone();

    hstack((
        parts::metric(
            "Nodes",
            signals
                .clone()
                .map(|rows| Str::from(rows.len().to_string()))
                .computed(),
        ),
        parts::metric(
            "Notifications",
            signals
                .clone()
                .map(|rows| {
                    let total: u64 = rows.iter().map(|row| row.notifications).sum();
                    Str::from(total.to_string())
                })
                .computed(),
        ),
        parts::metric(
            "Watchers",
            signals
                .map(|rows| {
                    let total: u64 = rows.iter().map(|row| u64::from(row.subscribers)).sum();
                    Str::from(total.to_string())
                })
                .computed(),
        ),
        spacer(),
    ))
    .spacing(32.0)
}

/// One node: what it holds, where it came from, and how hard it is working.
fn row_view(row: SignalRow) -> ListItem {
    ListItem::new(
        hstack((
            vstack((
                text(row.type_name).sub_headline(),
                // The creation site is the whole point: it turns "something is
                // firing 4000 times a second" into a file and a line.
                text(row.origin).caption().foreground(MutedForeground),
            ))
            .alignment(HorizontalAlignment::Leading)
            .spacing(2.0),
            spacer(),
            parts::metric(
                "watchers",
                Computed::constant(Str::from(row.subscribers.to_string())),
            ),
            parts::metric(
                "fired",
                Computed::constant(Str::from(row.notifications.to_string())),
            ),
        ))
        .spacing(16.0)
        .padding_with(EdgeInsets::new(8.0, 8.0, 12.0, 12.0))
        // A dropped node stays visible so its history can still be read, but it is
        // clearly no longer live.
        .opacity(if row.dropped { 0.45 } else { 1.0 }),
    )
}
