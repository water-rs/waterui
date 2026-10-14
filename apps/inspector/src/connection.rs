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
    ChannelSet, InspectorClientMessage, InspectorEvent, InspectorServerMessage, build_identity,
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
        let addr = addr.parse().map_err(|error| {
            format!("WATERUI_INSPECTOR_TARGET_ADDR `{addr}` is invalid: {error}")
        })?;
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
            identity: build_identity(waterui::FRAMEWORK_VERSION),
            channels: model.subscribed.snapshot(),
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
            identity,
            target: info,
            available,
        } => {
            let local = build_identity(waterui::FRAMEWORK_VERSION);
            if identity != local {
                return Err(format!(
                    "protocol mismatch: inspector is {local}, target is {identity}"
                ));
            }
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

        match futures_lite::future::or(async { Step::Event(next_event.await) }, async {
            Step::Subscribe(next_request.await.ok())
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use smol::net::TcpListener;
    use waterui_inspector_protocol::{InspectorBuildIdentity, TargetInfo, protocol_info};

    /// How long a handshake fixture may run before it is counted hung.
    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    /// Runs one session against a fixture server with the whole cycle — the
    /// client session and the server's own completion — inside the same bound.
    /// A timeout drops the session future and cancels its owned server task.
    #[expect(
        clippy::future_not_send,
        reason = "the inspector's model is Rc-based UI state driven by the main-thread executor; this future is never sent"
    )]
    async fn bounded_session(
        model: &Model,
        target: &Target,
        receiver: &SubscriptionReceiver,
        server: smol::Task<()>,
    ) -> Result<(), String> {
        futures_lite::future::or(
            async move {
                let result = session(model, target, receiver).await;
                server.await;
                result
            },
            async {
                smol::Timer::after(TEST_TIMEOUT).await;
                Err(String::from("fixture timed out"))
            },
        )
        .await
    }

    /// Answers one connection with a real Welcome frame over TCP. The server
    /// task ends once its frame is delivered — the client sees the frame then
    /// the EOF, so the test owns the server's lifetime rather than racing it.
    async fn fake_target(welcome: InspectorServerMessage) -> (SocketAddr, smol::Task<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fixture listener");
        let addr = listener.local_addr().expect("listener address");
        let server = smol::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accepted");
            let mut reader = BufReader::new(stream.clone());
            let _hello: InspectorClientMessage =
                read_frame(&mut reader).await.expect("client hello");
            let mut writer = stream;
            write_frame(&mut writer, &welcome)
                .await
                .expect("welcome frame");
        });
        (addr, server)
    }

    /// Same protocol build as this test binary, but a foreign framework
    /// version — the exact case `version + commit` alone could not reject.
    fn foreign_framework_identity() -> InspectorBuildIdentity {
        InspectorBuildIdentity {
            framework_version: String::from("0.0.2"),
            protocol: protocol_info(),
        }
    }

    /// A foreign protocol inside this framework's identity.
    fn foreign_protocol_identity() -> InspectorBuildIdentity {
        let mut identity = build_identity(waterui::FRAMEWORK_VERSION);
        identity.protocol.build_commit = if identity.protocol.build_commit.is_some() {
            None
        } else {
            Some(String::from("0000000000000000000000000000000000000000"))
        };
        identity
    }

    fn fixture_target(addr: SocketAddr) -> Target {
        Target {
            addr,
            token: String::from("token"),
        }
    }

    #[test]
    fn a_welcome_with_a_foreign_framework_is_never_live() {
        smol::block_on(async {
            let (addr, server) = fake_target(InspectorServerMessage::Welcome {
                identity: foreign_framework_identity(),
                target: TargetInfo {
                    pid: 0,
                    name: "fixture".into(),
                    backend: "test".into(),
                },
                available: ChannelSet::default_subscription(),
            })
            .await;
            let model = Model::new();
            let (_subscriptions, receiver) = subscription_channel();
            let result = bounded_session(&model, &fixture_target(addr), &receiver, server)
                .await
                .expect_err("a foreign protocol is rejected");
            assert!(
                result.contains("protocol mismatch"),
                "expected a protocol rejection, got {result}"
            );
            assert!(
                model.connection.snapshot() != Connection::Live,
                "a rejected welcome must not mark the connection live"
            );
            assert!(
                model.target.snapshot().is_none(),
                "a rejected welcome must not publish the target"
            );
            assert!(
                model.available.snapshot().is_empty(),
                "a rejected welcome must not publish channels"
            );
        });
    }

    #[test]
    fn a_welcome_with_a_foreign_protocol_is_never_live() {
        smol::block_on(async {
            let (addr, server) = fake_target(InspectorServerMessage::Welcome {
                identity: foreign_protocol_identity(),
                target: TargetInfo {
                    pid: 0,
                    name: "fixture".into(),
                    backend: "test".into(),
                },
                available: ChannelSet::default_subscription(),
            })
            .await;
            let model = Model::new();
            let (_subscriptions, receiver) = subscription_channel();
            let result = bounded_session(&model, &fixture_target(addr), &receiver, server)
                .await
                .expect_err("a foreign protocol is rejected");
            assert!(result.contains("protocol mismatch"), "{result}");
            assert_ne!(model.connection.snapshot(), Connection::Live);
        });
    }

    #[test]
    fn a_compatible_welcome_marks_the_connection_live() {
        smol::block_on(async {
            let (addr, server) = fake_target(InspectorServerMessage::Welcome {
                identity: build_identity(waterui::FRAMEWORK_VERSION),
                target: TargetInfo {
                    pid: 0,
                    name: "fixture".into(),
                    backend: "test".into(),
                },
                available: ChannelSet::default_subscription(),
            })
            .await;
            let model = Model::new();
            let (_subscriptions, receiver) = subscription_channel();
            let result = bounded_session(&model, &fixture_target(addr), &receiver, server).await;
            // The fixture drops its end after the Welcome, so the session ends
            // on the closed stream — a timeout means the harness hung.
            assert!(
                result
                    .as_ref()
                    .expect_err("the session ends when the fixture's stream closes")
                    .contains("stream closed"),
                "expected the stream close, got {result:?}"
            );
            assert_eq!(model.connection.snapshot(), Connection::Live);
        });
    }
}
