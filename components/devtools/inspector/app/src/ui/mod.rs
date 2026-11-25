//! The Inspector's views.
//!
//! A three-part shell: a sidebar of sections, a detail column per section, and a
//! status strip that is always visible because "am I actually connected?" is the
//! question every other answer depends on.

use waterui::prelude::*;
use waterui::theme::color::{Accent, Background, MutedForeground};
use waterui_icons_lucide as lucide;

use crate::connection::SubscriptionSender;
use crate::model::{Connection, Model, Section};

mod frames;
mod logs;
mod overview;
mod signals;
mod tasks;
mod tree;

pub(crate) mod parts;

/// The Inspector's root view.
pub fn inspector(model: Model, subscriptions: SubscriptionSender) -> impl View {
    let selected = model.section.clone();

    NavigationSplitView::new(&selected, sidebar(model.clone()), move |section| {
        detail(section, model.clone(), subscriptions.clone())
    })
}

/// Builds one section's pane, without the surrounding split view.
///
/// Exposed so each pane can be mounted and reviewed on its own.
pub fn pane(section: Section, model: Model, subscriptions: SubscriptionSender) -> impl View {
    match section {
        Section::Overview => OneOf6::A(overview::view(model, subscriptions)),
        Section::Frames => OneOf6::B(frames::view(model)),
        Section::Tree => OneOf6::C(tree::view(model)),
        Section::Tasks => OneOf6::D(tasks::view(model)),
        Section::Logs => OneOf6::E(logs::view(model)),
        Section::Signals => OneOf6::F(signals::view(model)),
    }
}

/// The detail column for the selected `section`.
fn detail(
    section: Section,
    model: Model,
    subscriptions: SubscriptionSender,
) -> NavigationView<impl View> {
    NavigationView::new(section.title(), pane(section, model, subscriptions))
}

/// The section list, with a live connection indicator pinned above it.
///
/// A split view may rebuild its sidebar, so this is a closure — which is a view,
/// and unlike the layout it produces, is `Clone`.
fn sidebar(model: Model) -> impl View + Clone {
    move || {
        let model = model.clone();
        vstack((
            connection_badge(&model).background(Background),
            Divider,
            List::for_each(StateList::from(Vec::from(Section::ALL)), {
                let model = model.clone();
                move |section| sidebar_row(section, &model)
            })
            .selection(&model.section),
        ))
    }
}

/// One sidebar entry: icon, title, and whatever number matters for that section.
fn sidebar_row(section: Section, model: &Model) -> impl View + use<> {
    let unavailable = section.channel().map(|channel| {
        model
            .available
            .clone()
            .map(move |available| !available.contains(channel.as_set()))
            .computed()
    });

    let row = hstack((
        section_icon(section).size(18.0, 18.0).foreground(Accent),
        text(section.title()),
        spacer(),
        parts::count_badge(section, model),
    ))
    .spacing(10.0)
    .padding_with(EdgeInsets::all(8.0));

    // A channel the target cannot produce is shown, dimmed, rather than hidden:
    // knowing a capability is absent is itself diagnostic information.
    match unavailable {
        Some(unavailable) => OneOf2::A(
            row.opacity(unavailable.clone().select(0.45, 1.0))
                .disabled(unavailable),
        ),
        None => OneOf2::B(row),
    }
}

fn section_icon(section: Section) -> impl View {
    match section {
        Section::Overview => lucide::gauge(),
        Section::Frames => lucide::activity(),
        Section::Tree => lucide::list_tree(),
        Section::Tasks => lucide::cpu(),
        Section::Logs => lucide::scroll_text(),
        Section::Signals => lucide::radio(),
    }
}

/// The always-visible connection state.
fn connection_badge(model: &Model) -> impl View + use<> {
    let connection = model.connection.clone();
    let live = connection.map(|state| state == Connection::Live);
    let detail = connection
        .map(|state| match state {
            Connection::Connecting => Str::from("Connecting…"),
            Connection::Live => Str::from("Live"),
            Connection::Failed(reason) => reason,
        })
        .computed();

    hstack((
        parts::status_dot(live.computed()),
        vstack((
            text!("{detail}").subheadline(),
            text!("{endpoint}", endpoint = model.endpoint)
                .caption()
                .foreground(MutedForeground),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(2.0),
        spacer(),
    ))
    .spacing(10.0)
    .padding_with(EdgeInsets::all(12.0))
}
