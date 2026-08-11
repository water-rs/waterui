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
    Channel, ChannelSet, InspectorClientMessage, InspectorServerMessage, TargetInfo, protocol_info,
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
        match handshake(socket, token, target, available) {
            Ok((socket, channels)) => {
                tracing::debug!(
                    target: "waterui::inspector",
                    peer = ?peer,
                    channels = ?channels,
                    "Inspector connected"
                );
                spawn_client(hub, socket, channels);
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
    target: &TargetInfo,
    available: ChannelSet,
) -> io::Result<(TcpStream, ChannelSet)> {
    socket.set_nodelay(true)?;
    // A peer that connects and says nothing must not hold a thread forever.
    socket.set_read_timeout(Some(super::HANDSHAKE_TIMEOUT))?;

    let hello: InspectorClientMessage = read_frame_blocking(&mut socket)?;
    let InspectorClientMessage::Hello {
        token,
        protocol,
        channels,
    } = hello
    else {
        return Err(reject(
            &mut socket,
            io::ErrorKind::InvalidInput,
            "expected a Hello message",
        ));
    };

    if token != expected_token {
        return Err(reject(
            &mut socket,
            io::ErrorKind::PermissionDenied,
            "invalid inspector token",
        ));
    }

    if !protocol.is_compatible() {
        // Two builds of the protocol disagree about the wire format. Refusing
        // here produces one clear error instead of arbitrary decode failures
        // later.
        return Err(reject(
            &mut socket,
            io::ErrorKind::InvalidData,
            &format!(
                "inspector protocol mismatch: inspector is {}, target is {}",
                protocol.build_commit,
                protocol_info().build_commit
            ),
        ));
    }

    socket.set_read_timeout(None)?;
    write_frame_blocking(
        &mut socket,
        &InspectorServerMessage::Welcome {
            protocol: protocol_info(),
            target: target.clone(),
            available,
        },
    )?;

    Ok((socket, channels & available))
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
                update_subscription(hub, &clients, available);
            }
            Dispatch::Subscribed { client, channels } => {
                if let Some(entry) = clients.iter_mut().find(|entry| entry.id == client) {
                    entry.channels = channels & available;
                }
                update_subscription(hub, &clients, available);
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
}
