//! Socket handling for the inspector endpoint.
//!
//! Three kinds of thread, each with a single job:
//!
//! - the **acceptor** authenticates new connections and hands them to the
//!   dispatcher;
//! - one **reader** per client turns inbound control messages into dispatches;
//! - the **dispatcher** owns every client and is the only thread that writes to
//!   a socket, so client state needs no lock.

use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use waterui_inspector_protocol::transport::{read_frame_blocking, write_frame_blocking};
use waterui_inspector_protocol::{
    Channel, ChannelSet, InspectorBuildIdentity, InspectorClientMessage, InspectorServerMessage,
    TargetInfo,
};

use super::hub::{ClientId, Dispatch, EventHub};

/// A connected inspector, owned exclusively by the dispatcher thread.
struct Client {
    id: ClientId,
    socket: TcpStream,
    channels: ChannelSet,
}

/// Runs the accept loop until the listener fails.
pub(super) fn accept_loop(
    listener: &TcpListener,
    hub: &Arc<EventHub>,
    token: &str,
    identity: &InspectorBuildIdentity,
    target: &TargetInfo,
    available: ChannelSet,
) {
    for connection in listener.incoming() {
        let socket = match connection {
            Ok(socket) => socket,
            Err(error) => {
                tracing::warn!(
                    target: "waterui::inspector",
                    error = %error,
                    "Failed to accept inspector connection"
                );
                continue;
            }
        };

        let peer = socket.peer_addr().ok();
        match handshake(socket, token, identity, target, available) {
            Ok(Some((socket, channels))) => {
                tracing::debug!(
                    target: "waterui::inspector",
                    peer = ?peer,
                    channels = ?channels,
                    "Inspector connected"
                );
                spawn_client(hub, socket, channels);
            }
            // An `Identify` was answered and the peer closed cleanly —
            // answered, not rejected.
            Ok(None) => {
                tracing::debug!(
                    target: "waterui::inspector",
                    peer = ?peer,
                    "Answered an inspector identity query"
                );
            }
            Err(error) => {
                tracing::warn!(
                    target: "waterui::inspector",
                    peer = ?peer,
                    error = %error,
                    "Rejected inspector connection"
                );
            }
        }
    }
}

/// Authenticates a connection and negotiates its initial subscription.
fn handshake(
    mut socket: TcpStream,
    expected_token: &str,
    identity: &InspectorBuildIdentity,
    target: &TargetInfo,
    available: ChannelSet,
) -> io::Result<Option<(TcpStream, ChannelSet)>> {
    socket.set_nodelay(true)?;
    // A peer that connects and says nothing must not hold a thread forever.
    socket.set_read_timeout(Some(super::HANDSHAKE_TIMEOUT))?;

    let first: InspectorClientMessage = read_frame_blocking(&mut socket)?;
    let (token, peer, channels) = match first {
        InspectorClientMessage::Identify { token } => {
            // Answer the endpoint-first query and close — no client
            // registration, no subscription, the socket is done.
            if token != expected_token {
                return Err(reject(
                    &mut socket,
                    io::ErrorKind::PermissionDenied,
                    "invalid inspector token",
                ));
            }
            write_frame_blocking(
                &mut socket,
                &InspectorServerMessage::Identity {
                    identity: identity.clone(),
                },
            )?;
            return Ok(None);
        }
        InspectorClientMessage::Hello {
            token,
            identity,
            channels,
        } => (token, identity, channels),
        _ => {
            return Err(reject(
                &mut socket,
                io::ErrorKind::InvalidInput,
                "expected a Hello or Identify message",
            ));
        }
    };

    if token != expected_token {
        return Err(reject(
            &mut socket,
            io::ErrorKind::PermissionDenied,
            "invalid inspector token",
        ));
    }

    if peer != *identity {
        // Two builds disagree about more than the wire format — a foreign
        // framework version is a mismatch even when the protocol version
        // coincides. Refusing here produces one clear error instead of
        // arbitrary decode failures later.
        return Err(reject(
            &mut socket,
            io::ErrorKind::InvalidData,
            &format!("inspector build mismatch: inspector is {peer}, target is {identity}"),
        ));
    }

    socket.set_read_timeout(None)?;
    write_frame_blocking(
        &mut socket,
        &InspectorServerMessage::Welcome {
            identity: identity.clone(),
            target: target.clone(),
            available,
        },
    )?;

    Ok(Some((socket, channels & available)))
}

/// Tells the peer why it was rejected, then reports the same reason locally.
fn reject(socket: &mut TcpStream, kind: io::ErrorKind, message: &str) -> io::Error {
    let _ = write_frame_blocking(
        socket,
        &InspectorServerMessage::Error {
            message: message.to_string(),
        },
    );
    io::Error::new(kind, message.to_string())
}

/// Registers an authenticated client and starts reading its control messages.
fn spawn_client(hub: &Arc<EventHub>, socket: TcpStream, channels: ChannelSet) {
    let id = hub.next_client_id();
    let Ok(reader_socket) = socket.try_clone() else {
        tracing::warn!(
            target: "waterui::inspector",
            "Failed to clone inspector socket; dropping connection"
        );
        return;
    };

    hub.send_control(Dispatch::Connected {
        client: id,
        socket,
        channels,
    });

    let reader_hub = Arc::clone(hub);
    let spawned = thread::Builder::new()
        .name(format!("waterui-inspector-client-{}", id.0))
        .spawn(move || {
            read_control_messages(&reader_hub, id, reader_socket);
            reader_hub.send_control(Dispatch::Disconnected { client: id });
        });

    if let Err(error) = spawned {
        tracing::warn!(
            target: "waterui::inspector",
            error = %error,
            "Failed to spawn inspector reader thread"
        );
        hub.send_control(Dispatch::Disconnected { client: id });
    }
}

/// Turns inbound control messages into dispatches until the socket closes.
fn read_control_messages(hub: &EventHub, client: ClientId, mut socket: TcpStream) {
    loop {
        match read_frame_blocking::<_, InspectorClientMessage>(&mut socket) {
            Ok(InspectorClientMessage::Subscribe { channels }) => {
                hub.send_control(Dispatch::Subscribed { client, channels });
            }
            Ok(InspectorClientMessage::Ping) => {
                hub.send_control(Dispatch::Ping { client });
            }
            Ok(InspectorClientMessage::Hello { .. }) => {
                tracing::warn!(
                    target: "waterui::inspector",
                    "Inspector sent a second Hello; ignoring"
                );
            }
            Ok(InspectorClientMessage::Identify { .. }) => {
                // Identity queries live at handshake time only — an
                // established client that asks again is speaking nonsense
                // (its token is deliberately not logged).
                tracing::warn!(
                    target: "waterui::inspector",
                    "Inspector sent Identify on an established connection; closing"
                );
                return;
            }
            Err(error) => {
                tracing::debug!(
                    target: "waterui::inspector",
                    error = %error,
                    "Inspector disconnected"
                );
                return;
            }
        }
    }
}

/// Owns every client and performs all socket writes.
pub(super) fn dispatch_loop(
    hub: &Arc<EventHub>,
    receiver: &async_channel::Receiver<Dispatch>,
    available: ChannelSet,
) {
    let mut clients: Vec<Client> = Vec::new();
    // A node asked for before any inspector was attached, sent to the first one
    // that arrives.
    let mut pending_select: Option<waterui_inspector_protocol::NodeId> = None;

    while let Ok(dispatch) = receiver.recv_blocking() {
        match dispatch {
            Dispatch::Event(envelope) => {
                let channel = envelope.event.channel();
                let message = InspectorServerMessage::Event { envelope };
                clients.retain_mut(|client| {
                    if !client.channels.contains(channel.as_set()) {
                        return true;
                    }
                    write_frame_blocking(&mut client.socket, &message).is_ok()
                });
                report_drops(hub, &mut clients, channel);
            }
            Dispatch::Connected {
                client,
                socket,
                channels,
            } => {
                clients.push(Client {
                    id: client,
                    socket,
                    channels,
                });
                // Inspecting an element launches an inspector that is not
                // connected yet, so the node it was asked to reveal waits here
                // for it rather than being sent to nobody.
                if let Some(node) = pending_select.take()
                    && let Some(entry) = clients.last_mut()
                {
                    let message = InspectorServerMessage::Select { node };
                    let _ = write_frame_blocking(&mut entry.socket, &message);
                }
                update_subscription(hub, &clients, available);
            }
            Dispatch::Subscribed { client, channels } => {
                if let Some(entry) = clients.iter_mut().find(|entry| entry.id == client) {
                    entry.channels = channels & available;
                }
                update_subscription(hub, &clients, available);
            }
            Dispatch::Select { node } => {
                if clients.is_empty() {
                    pending_select = Some(node);
                } else {
                    let message = InspectorServerMessage::Select { node };
                    clients.retain_mut(|entry| {
                        write_frame_blocking(&mut entry.socket, &message).is_ok()
                    });
                }
            }
            Dispatch::Ping { client } => {
                clients.retain_mut(|entry| {
                    entry.id != client
                        || write_frame_blocking(&mut entry.socket, &InspectorServerMessage::Pong)
                            .is_ok()
                });
            }
            Dispatch::Disconnected { client } => {
                clients.retain(|entry| entry.id != client);
                update_subscription(hub, &clients, available);
            }
        }

        if clients.is_empty() {
            update_subscription(hub, &clients, available);
        }
    }
}

/// Tells subscribed clients how many events they missed on `channel`.
fn report_drops(hub: &Arc<EventHub>, clients: &mut Vec<Client>, channel: Channel) {
    let dropped = hub.take_dropped(channel);
    if dropped == 0 {
        return;
    }
    let message = InspectorServerMessage::Dropped {
        channel,
        events: dropped,
    };
    clients.retain_mut(|client| {
        if !client.channels.contains(channel.as_set()) {
            return true;
        }
        write_frame_blocking(&mut client.socket, &message).is_ok()
    });
}

/// Recomputes what any client wants, so producers can gate on one atomic.
fn update_subscription(hub: &EventHub, clients: &[Client], available: ChannelSet) {
    let union = clients
        .iter()
        .fold(ChannelSet::empty(), |set, client| set | client.channels);
    hub.set_subscribed(union & available);
    hub.set_client_count(clients.len());
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use waterui_inspector_protocol::build_identity;

    /// A real connected pair of stream sockets: the end the server side of the
    /// fixture hands to `handshake`, and the client's end.
    ///
    /// `connect_timeout` queues the connection without an accept thread; the
    /// listener's backlog answers `accept` once the client is waiting. Both
    /// streams carry I/O timeouts so nothing inside `handshake` or the client
    /// can hang the test.
    fn socket_pair() -> (TcpStream, TcpStream) {
        const IO_TIMEOUT: Duration = Duration::from_secs(5);
        let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
        let addr = listener.local_addr().expect("listener address");
        let client = TcpStream::connect_timeout(&addr, IO_TIMEOUT).expect("client connect");
        let server_end = listener.accept().expect("accepted").0;
        for stream in [&client, &server_end] {
            stream
                .set_read_timeout(Some(IO_TIMEOUT))
                .expect("read timeout");
            stream
                .set_write_timeout(Some(IO_TIMEOUT))
                .expect("write timeout");
        }
        (server_end, client)
    }

    fn fixture_target() -> TargetInfo {
        TargetInfo {
            pid: 0,
            name: String::from("fixture"),
            backend: String::from("test"),
        }
    }

    /// Drives `handshake` on its own thread — it owns the socket and blocks —
    /// then writes one Hello as a client would and returns the reply.
    fn run_handshake(
        identity: InspectorBuildIdentity,
        hello: &InspectorClientMessage,
    ) -> (
        io::Result<Option<(TcpStream, ChannelSet)>>,
        InspectorServerMessage,
    ) {
        let (server_end, mut client) = socket_pair();
        let server = thread::spawn(move || {
            handshake(
                server_end,
                "token",
                &identity,
                &fixture_target(),
                ChannelSet::default_subscription(),
            )
        });
        write_frame_blocking(&mut client, hello).expect("hello frame");
        let reply: InspectorServerMessage = read_frame_blocking(&mut client).expect("server reply");
        (server.join().expect("handshake thread"), reply)
    }

    #[test]
    fn a_foreign_framework_version_is_rejected() {
        let server_identity = build_identity("0.0.1");
        // Same protocol build as the caller, but a foreign framework version —
        // the handshake must reject even though the wire format matches.
        let hello = InspectorClientMessage::Hello {
            token: String::from("token"),
            identity: InspectorBuildIdentity {
                framework_version: String::from("0.0.2"),
                protocol: build_identity("0.0.1").protocol,
            },
            channels: ChannelSet::default_subscription(),
        };
        let (result, reply) = run_handshake(server_identity, &hello);
        assert!(result.is_err(), "a foreign build must not handshake");
        match reply {
            InspectorServerMessage::Error { message } => {
                assert!(message.contains("build mismatch"), "{message}");
            }
            other => panic!("expected an Error reply, got {other:?}"),
        }
    }

    #[test]
    fn a_matching_identity_receives_the_welcome_identity() {
        let server_identity = build_identity("0.0.1");
        let hello = InspectorClientMessage::Hello {
            token: String::from("token"),
            identity: build_identity("0.0.1"),
            channels: ChannelSet::default_subscription(),
        };
        let (result, reply) = run_handshake(server_identity.clone(), &hello);
        assert!(result.expect("a matching identity handshakes").is_some());
        match reply {
            InspectorServerMessage::Welcome { identity, .. } => {
                assert_eq!(identity, server_identity);
            }
            other => panic!("expected a Welcome reply, got {other:?}"),
        }
    }

    /// An `Identify` with the session's token is answered with the server's
    /// own build identity and no session — the socket is done, the client
    /// was never registered.
    #[test]
    fn a_valid_identify_is_answered_without_a_session() {
        let server_identity = build_identity("0.0.1");
        let identify = InspectorClientMessage::Identify {
            token: String::from("token"),
        };
        let (result, reply) = run_handshake(server_identity.clone(), &identify);
        assert!(result.expect("a valid Identify answers").is_none(),);
        match reply {
            InspectorServerMessage::Identity { identity } => {
                assert_eq!(identity, server_identity);
            }
            other => panic!("expected an Identity reply, got {other:?}"),
        }
    }

    /// An `Identify` with a wrong token is refused the same way a wrong
    /// `Hello` token is — an Error, never the identity.
    #[test]
    fn a_wrong_token_identify_is_refused() {
        let server_identity = build_identity("0.0.1");
        let identify = InspectorClientMessage::Identify {
            token: String::from("not-the-token"),
        };
        let (result, reply) = run_handshake(server_identity, &identify);
        let error = result.expect_err("a wrong token is refused");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        match reply {
            InspectorServerMessage::Error { message } => {
                assert_eq!(message, "invalid inspector token");
                assert!(
                    !message.contains("not-the-token"),
                    "the refusal must not echo the credential: {message}"
                );
            }
            other => panic!("expected an Error reply, got {other:?}"),
        }
    }
}
