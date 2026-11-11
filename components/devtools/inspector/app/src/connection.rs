//! The client half of the inspector protocol.
//!
//! One task owns the socket for the life of a connection: it authenticates,
//! subscribes, and folds every inbound event into the [`Model`]. Subscription
//! changes made in the UI arrive over a channel, so the task never has to poll
//! the UI state and the UI never has to know about sockets.

use std::future::Future;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

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

/// How long the target has to accept a connection and answer the handshake.
///
/// A target that never answers used to leave the inspector waiting with no
/// explanation (water-rs/waterui#2230): Android suspends a cached
/// application, so its socket stays accepted and the `Hello` sits unread in
/// the backlog forever. The wait is bounded instead, and the retry loop
/// shows what failed and against which endpoint. The bound covers a cold
/// target that is slow to reply, not one that will never reply.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Where to connect and with what credential.
#[derive(Clone)]
#[non_exhaustive]
pub struct Target {
    /// Endpoint published by the target application.
    addr: SocketAddr,
    /// One-time session token.
    token: String,
}

impl Target {
    /// Endpoint published by the target application.
    #[must_use]
    pub const fn addr(&self) -> &SocketAddr {
        &self.addr
    }

    /// One-time session token.
    #[must_use]
    pub const fn token(&self) -> &str {
        self.token.as_str()
    }
}

// The token is a credential: it travels in the `Hello`, never in
// diagnostics.
impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Target")
            .field("addr", &self.addr)
            .field("token", &"[redacted]")
            .finish()
    }
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

        if let Err(error) = session(&model, &target, &subscriptions, HANDSHAKE_TIMEOUT).await {
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
    handshake_timeout: Duration,
) -> Result<(), String> {
    // One deadline for the whole handshake — connect, Hello and the
    // Welcome share it, so a target that stalls at each step is bounded
    // by `handshake_timeout` once, not three times.
    let deadline = Instant::now() + handshake_timeout;
    let stream = within(
        target.addr,
        "connecting to",
        deadline,
        TcpStream::connect(target.addr),
    )
    .await?
    .map_err(|error| format!("connect to {} failed: {error}", target.addr))?;
    let _ = stream.set_nodelay(true);

    let mut writer = stream.clone();
    let hello = InspectorClientMessage::Hello {
        token: target.token.clone(),
        protocol: protocol_info(),
        channels: model.subscribed.snapshot(),
    };
    within(target.addr, "sending the handshake to", deadline, async {
        write_frame(&mut writer, &hello).await
    })
    .await?
    .map_err(|error| format!("handshake with {} failed: {error}", target.addr))?;

    let mut reader = BufReader::new(stream);
    let reply = within(
        target.addr,
        "waiting for the handshake reply from",
        deadline,
        read_frame::<_, InspectorServerMessage>(&mut reader),
    )
    .await?
    .map_err(|error| format!("handshake with {} failed: {error}", target.addr))?;
    match reply {
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

/// Await `work`, giving up at `deadline` with an error that
/// names what was being attempted and the endpoint it was attempted
/// against — the only two facts a developer needs to tell a wrong address
/// from a target that is not answering.
async fn within<T>(
    addr: SocketAddr,
    what: &str,
    deadline: Instant,
    work: impl Future<Output = T>,
) -> Result<T, String> {
    futures_lite::future::or(async { Ok(work.await) }, async {
        smol::Timer::at(deadline).await;
        Err(format!(
            "{what} {addr} timed out. The application may be suspended — \
             Android freezes an application that is not in the foreground — or nothing is \
             listening there."
        ))
    })
    .await
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
    use std::net::{Ipv4Addr, SocketAddr};
    use std::time::Duration;

    use smol::net::TcpListener;
    use waterui_inspector_protocol::InspectorClientMessage;
    use waterui_inspector_protocol::transport::read_frame;

    use super::{Target, session, subscription_channel};
    use crate::model::Model;

    /// A target that accepts the connection, reads the `Hello` and never
    /// answers — what an Android application looks like once the system
    /// has frozen it. The handshake must end by itself, say where it was
    /// waiting, and take the socket down with it: the server observing an
    /// EOF proves the losing side's stream really dropped.
    #[test]
    fn a_handshake_with_no_reply_fails_naming_the_endpoint() {
        smol::block_on(async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .expect("bind a silent target");
            let addr: SocketAddr = listener.local_addr().expect("the listener's address");

            // The server accepts, reads the `Hello`, then waits on the
            // next frame — the client timing out closes its socket,
            // which the server sees as the stream ending.
            let server = async move {
                let (stream, _peer) = listener.accept().await.expect("accept the connection");
                let mut stream = futures_lite::io::BufReader::new(stream);
                let hello: InspectorClientMessage =
                    read_frame(&mut stream).await.expect("the Hello arrives");
                assert!(
                    matches!(hello, InspectorClientMessage::Hello { .. }),
                    "{hello:?}"
                );
                // Whatever the handshake's deadline does to the client,
                // it ends the socket here.
                read_frame::<_, InspectorClientMessage>(&mut stream).await
            };

            let (_sender, receiver) = subscription_channel();
            let model = Model::new();
            let target = Target {
                addr,
                token: String::from("token"),
            };
            let session = session(&model, &target, &receiver, Duration::from_millis(200));

            let both = futures_lite::future::zip(session, server);
            // If neither side finishes the test is itself wedged — bound
            // the whole arrangement, not just the client.
            let (error, observed) = futures_lite::future::or(both, async {
                smol::Timer::after(Duration::from_secs(2)).await;
                panic!("the silent-handshake test did not settle inside 2s");
            })
            .await;

            let error = error.expect_err("a target that never answers cannot complete a handshake");
            assert!(error.contains(&addr.to_string()), "{error}");
            assert!(error.contains("timed out"), "{error}");
            assert!(
                observed.is_err(),
                "the socket must end, not hang: {observed:?}"
            );
        });
    }

    /// The token is a credential: `Debug` names the endpoint without
    /// spelling the secret.
    #[test]
    fn the_target_debug_never_shows_the_token() {
        let target = Target {
            addr: "127.0.0.1:23106".parse().expect("a valid address"),
            token: String::from("s3cret-target-token"),
        };
        let rendered = format!("{target:?}");
        assert!(!rendered.contains("s3cret-target-token"), "{rendered}");
        assert!(rendered.contains("23106"), "{rendered}");
    }
}
