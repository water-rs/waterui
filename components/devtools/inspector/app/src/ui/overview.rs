//! Connection summary and channel subscription.
//!
//! This is where a developer decides what the target should spend effort
//! producing, so each channel says what it costs as well as what it shows.

use waterui::prelude::theme_color::MutedForeground;
use waterui::prelude::*;
use waterui_controls::LabelDisplayMode;
use waterui::widget::condition::when;
use waterui_inspector_protocol::{Channel, ChannelSet};

use crate::connection::SubscriptionSender;
use crate::model::Model;

use super::parts;

/// The overview pane.
#[expect(
    clippy::needless_pass_by_value,
    reason = "view builders take owned reactive handles: the views they return capture them for 'static, so a borrow would not do"
)]
pub fn view(model: Model, subscriptions: SubscriptionSender) -> impl View {
    let target = model.target.clone();

    scroll(
        vstack((
            parts::section_header(
                "Target",
                "The application this inspector is attached to.",
            ),
            hstack((
                parts::metric(
                    "Process",
                    target
                        .clone()
                        .map(|info| {
                            info.map_or_else(
                                || Str::from("—"),
                                |info| Str::from(info.pid.to_string()),
                            )
                        })
                        .computed(),
                ),
                parts::metric(
                    "Backend",
                    target
                        .clone()
                        .map(|info| {
                            info.map_or_else(|| Str::from("—"), |info| Str::from(info.backend))
                        })
                        .computed(),
                ),
                parts::metric(
                    "Name",
                    target
                        .map(|info| info.map_or_else(|| Str::from("—"), |info| Str::from(info.name)))
                        .computed(),
                ),
            ))
            .spacing(32.0),
            dropped_notice(&model),
            Divider,
            parts::section_header(
                "Channels",
                "Nothing is measured for a channel you have not opened.",
            ),
            channel_rows(&model, &subscriptions).spacing(4.0),
            spacer(),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(20.0)
        .padding_with(EdgeInsets::all(20.0)),
    )
}

/// Reports events the target produced but could not deliver.
///
/// Hidden when nothing was lost; loud when something was, because every other
/// number in the app is wrong by that amount.
fn dropped_notice(model: &Model) -> impl View {
    let dropped = model.dropped.clone();
    let any = dropped.map(|dropped| dropped > 0).computed();

    when(any, move || {
        text!("{dropped} events dropped — the target outran this connection")
            .caption()
            .foreground(Orange)
    })
    .otherwise(|| ())
}

fn channel_rows(model: &Model, subscriptions: &SubscriptionSender) -> VStack<(Vec<AnyView>,)> {
    Channel::ALL
        .iter()
        .map(|channel| channel_row(*channel, model, subscriptions).anyview())
        .collect()
}

/// One channel: a toggle, a description, and whether the target has it at all.
fn channel_row(
    channel: Channel,
    model: &Model,
    subscriptions: &SubscriptionSender,
) -> impl View {
    let subscribed = model.subscribed.clone();
    let available = model.available.clone();
    let unavailable = available
        
        .map(move |available| !available.contains(channel.as_set()))
        .computed();

    // A two-way binding over one bit of the subscription set: reading maps the
    // set to a bool, writing folds the bool back into the set and tells the
    // connection task.
    let enabled = channel_enabled(channel, &subscribed, subscriptions.clone());

    hstack((
        hstack((text(channel_title(channel)), spacer())).width(110.0),
        when(unavailable.clone(), || {
            text("unsupported").caption().foreground(Orange)
        })
        .otherwise(move || text(channel_cost(channel)).caption().foreground(MutedForeground)),
        spacer(),
        toggle(channel_title(channel), &enabled)
            .disabled(unavailable)
            .install(LabelDisplayMode::Hidden),
    ))
    .spacing(12.0)
    .padding_with(EdgeInsets::all(8.0))
}

/// A two-way binding over one bit of the subscription set.
///
/// Reading maps the set to a bool; writing folds the bool back into the set and
/// asks the connection task to tell the target.
fn channel_enabled(
    channel: Channel,
    subscribed: &Binding<ChannelSet>,
    subscriptions: SubscriptionSender,
) -> Binding<bool> {
    Binding::mapping(
        subscribed,
        move |set: ChannelSet| set.contains(channel.as_set()),
        move |set: &Binding<ChannelSet>, on: bool| {
            let mut next = set.get();
            next.set(channel.as_set(), on);
            set.set(next);
            // The connection task owns the socket, so the change is requested
            // rather than written here.
            let _ = subscriptions.try_send(next);
        },
    )
}

const fn channel_title(channel: Channel) -> &'static str {
    match channel {
        Channel::Frames => "Frames",
        Channel::Tree => "View tree",
        Channel::Tasks => "Tasks",
        Channel::Logs => "Logs",
        Channel::Signals => "Signals",
    }
}

/// What subscribing to a channel actually costs the target.
const fn channel_cost(channel: Channel) -> &'static str {
    match channel {
        Channel::Frames => "one event per frame",
        Channel::Tree => "one event per structural change",
        Channel::Tasks => "one aggregate per 100ms",
        Channel::Logs => "one event per record",
        Channel::Signals => "one event per notification — expensive",
    }
}

#[cfg(test)]
mod tests {
    use super::channel_enabled;
    use waterui::prelude::*;
    use waterui_inspector_protocol::{Channel, ChannelSet};

    /// The toggle must start in the state the subscription is actually in,
    /// otherwise the Overview lies about what the target is producing.
    ///
    /// Exercised against the real preview model, which is what the visual
    /// review renders.
    #[core::prelude::v1::test]
    fn the_preview_model_starts_subscribed_to_the_default_channels() {
        let model = crate::preview_model();
        let (sender, _receiver) = async_channel::unbounded();
        let frames = channel_enabled(Channel::Frames, &model.subscribed, sender);
        assert!(
            frames.get(),
            "preview subscription was {:?}",
            model.subscribed.get()
        );
    }

    #[core::prelude::v1::test]
    fn a_channel_toggle_reflects_the_current_subscription() {
        let subscribed = Binding::container(ChannelSet::default_subscription());
        let (sender, _receiver) = async_channel::unbounded();

        let frames = channel_enabled(Channel::Frames, &subscribed, sender.clone());
        let signals = channel_enabled(Channel::Signals, &subscribed, sender);

        assert!(frames.get(), "frames is in the default subscription");
        assert!(!signals.get(), "signals is not");

        signals.set(true);
        assert!(
            subscribed.get().contains(ChannelSet::SIGNALS),
            "writing the toggle must fold the bit back into the set"
        );
    }
}
