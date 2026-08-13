//! The client half of the inspector protocol.
//!
//! One task owns the socket for the life of a connection: it authenticates,
//! subscribes, and folds every inbound event into the [`Model`]. Subscription
//! changes made in the UI arrive over a channel, so the task never has to poll
//! the UI state and the UI never has to know about sockets.

use std::net::SocketAddr;
use std::time::Duration;

use futures_lite::io::BufReader;
use smol::net::TcpStream;
use waterui::prelude::*;
use waterui_inspector_protocol::transport::{read_frame, write_frame};
use waterui_inspector_protocol::{
    ChannelSet, InspectorClientMessage, InspectorEvent, InspectorServerMessage, protocol_info,
};

use crate::model::{Connection, Model};

/// How long to wait before reconnecting after a failure.
const RETRY_DELAY: Duration = Duration::from_secs(1);

/// Where to connect and with what credential.
#[derive(Debug, Clone)]
pub struct Target {
    /// Endpoint published by the target application.
    pub addr: SocketAddr,
    /// One-time session token.
    pub token: String,
}

impl Target {
    /// Reads the target from the environment the CLI launches us with.
    ///
    /// # Errors
    ///
    /// Returns a message naming the missing or malformed variable. The inspector
    /// shows it rather than dying silently behind an already-visible window.
    pub fn from_env() -> Result<Self, String> {
        let addr = std::env::var("WATERUI_INSPECTOR_TARGET_ADDR")
            .map_err(|_| String::from("WATERUI_INSPECTOR_TARGET_ADDR is not set"))?;
        let token = std::env::var("WATERUI_INSPECTOR_TOKEN")
            .map_err(|_| String::from("WATERUI_INSPECTOR_TOKEN is not set"))?;
        let addr = addr
            .parse()
            .map_err(|error| format!("WATERUI_INSPECTOR_TARGET_ADDR `{addr}` is invalid: {error}"))?;
        Ok(Self { addr, token })
    }
}

/// Requests the UI can make of the connection task.
pub type SubscriptionSender = async_channel::Sender<ChannelSet>;
type SubscriptionReceiver = async_channel::Receiver<ChannelSet>;

/// Creates the channel the UI uses to change its subscription.
#[must_use]
pub fn subscription_channel() -> (SubscriptionSender, SubscriptionReceiver) {
    async_channel::unbounded()
}

/// Connects, streams, and reconnects until the app exits.
#[expect(
    clippy::future_not_send,
    reason = "the inspector's model is Rc-based UI state driven by the main-thread executor; this future is never sent"
)]
pub async fn run(model: Model, target: Target, subscriptions: SubscriptionReceiver) {
    model.endpoint.set(Str::from(target.addr.to_string()));

    loop {
        model.connection.set(Connection::Connecting);
        model.reset_stream();

        if let Err(error) = session(&model, &target, &subscriptions).await {
            model.connection.set(Connection::Failed(Str::from(error)));
        }
        smol::Timer::after(RETRY_DELAY).await;
    }
}

/// One connection, from handshake to disconnect.
#[expect(
    clippy::future_not_send,
    reason = "the inspector's model is Rc-based UI state driven by the main-thread executor; this future is never sent"
)]
async fn session(
    model: &Model,
    target: &Target,
    subscriptions: &SubscriptionReceiver,
) -> Result<(), String> {
    let stream = TcpStream::connect(target.addr)
        .await
        .map_err(|error| format!("connect failed: {error}"))?;
    let _ = stream.set_nodelay(true);

    let mut writer = stream.clone();
    write_frame(
        &mut writer,
        &InspectorClientMessage::Hello {
            token: target.token.clone(),
            protocol: protocol_info(),
            channels: model.subscribed.get(),
        },
    )
    .await
    .map_err(|error| format!("handshake failed: {error}"))?;

    let mut reader = BufReader::new(stream);
    match read_frame::<_, InspectorServerMessage>(&mut reader)
        .await
        .map_err(|error| format!("handshake failed: {error}"))?
    {
        InspectorServerMessage::Welcome {
            target: info,
            available,
            ..
        } => {
            model.available.set(available);
            model.target.set(Some(info));
            model.connection.set(Connection::Live);
        }
        InspectorServerMessage::Error { message } => return Err(message),
        other => return Err(format!("unexpected handshake reply: {other:?}")),
    }

    // Forwarding subscription changes and reading events are independent; racing
    // them keeps a quiet target from delaying a subscription change and a busy
    // one from starving it.
    loop {
        let next_event = read_frame::<_, InspectorServerMessage>(&mut reader);
        let next_request = subscriptions.recv();

        match futures_lite::future::or(
            async { Step::Event(next_event.await) },
            async { Step::Subscribe(next_request.await.ok()) },
        )
        .await
        {
            Step::Event(Ok(message)) => {
                if let Some(error) = apply(model, message) {
                    return Err(error);
                }
            }
            Step::Event(Err(error)) => return Err(format!("stream closed: {error}")),
            Step::Subscribe(Some(channels)) => {
                write_frame(&mut writer, &InspectorClientMessage::Subscribe { channels })
                    .await
                    .map_err(|error| format!("failed to update subscription: {error}"))?;
            }
            // The UI is gone, which means the app is shutting down.
            Step::Subscribe(None) => return Ok(()),
        }
    }
}

enum Step {
    Event(std::io::Result<InspectorServerMessage>),
    Subscribe(Option<ChannelSet>),
}

/// Folds one server message into the model, returning a fatal error if any.
fn apply(model: &Model, message: InspectorServerMessage) -> Option<String> {
    match message {
        InspectorServerMessage::Event { envelope } => {
            match envelope.event {
                InspectorEvent::Frame(sample) => model.apply_frame(sample),
                InspectorEvent::Tree(update) => model.apply_tree(update),
                InspectorEvent::Tasks(window) => model.apply_tasks(window),
                InspectorEvent::Stall(stall) => model.apply_stall(stall),
                InspectorEvent::Log(record) => model.apply_log(record),
                InspectorEvent::Signal(event) => model.apply_signal(event),
            }
            None
        }
        InspectorServerMessage::Dropped { channel, events } => {
            tracing::warn!(
                target: "waterui::inspector",
                channel = %channel,
                events,
                "Target dropped events"
            );
            model.apply_drop(events);
            None
        }
        // A pong needs no action, and a second welcome is a target repeating
        // itself; neither is an error.
        InspectorServerMessage::Pong | InspectorServerMessage::Welcome { .. } => None,
        // Someone picked "inspect element" in the application: show it.
        InspectorServerMessage::Select { node } => {
            model.apply_select(node.0);
            None
        }
        InspectorServerMessage::Error { message } => Some(message),
    }
}
