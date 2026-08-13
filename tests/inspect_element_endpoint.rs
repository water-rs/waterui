//! Asking to inspect an element reaches an inspector.
//!
//! Driven over a real socket against a real endpoint, because the thing worth
//! checking is the handover: a selection made before anyone is attached has to
//! survive until one is, or the window opens on the wrong thing.

#![cfg(feature = "inspector")]

use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::time::Duration;

use waterui::inspector::{InspectorServerConfig, init_with_config};
use waterui::inspector::protocol::transport::{read_frame_blocking, write_frame_blocking};
use waterui::inspector::protocol::{
    ChannelSet, InspectorClientMessage, InspectorServerMessage, NodeId, protocol_info,
};

fn config() -> InspectorServerConfig {
    InspectorServerConfig {
        host: IpAddr::V4(Ipv4Addr::LOCALHOST),
        // An ephemeral port keeps concurrent tests from colliding.
        port: 0,
        token: String::from("test-token"),
        task_window: Duration::from_millis(100),
        stall_ratio: 0.9,
        app_name: String::from("test"),
    }
}

/// Connects and completes the handshake, returning the stream to read from.
fn attach(addr: std::net::SocketAddr) -> TcpStream {
    let mut socket = TcpStream::connect(addr).expect("the endpoint accepts connections");
    write_frame_blocking(
        &mut socket,
        &InspectorClientMessage::Hello {
            token: String::from("test-token"),
            protocol: protocol_info(),
            channels: ChannelSet::empty(),
        },
    )
    .expect("the handshake is written");

    let welcome: InspectorServerMessage =
        read_frame_blocking(&mut socket).expect("the endpoint answers the handshake");
    assert!(
        matches!(welcome, InspectorServerMessage::Welcome { .. }),
        "expected a welcome, got {welcome:?}"
    );
    socket
}

/// Reads until a `Select` arrives, failing rather than hanging if it never does.
fn next_select(socket: &mut TcpStream) -> NodeId {
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("a read timeout can be set");
    loop {
        let message: InspectorServerMessage =
            read_frame_blocking(socket).expect("the endpoint keeps talking until a Select arrives");
        if let InspectorServerMessage::Select { node } = message {
            return node;
        }
    }
}

/// An inspector that is already attached is told to reveal the node at once.
#[test]
fn an_attached_inspector_is_told_which_node_to_reveal() {
    let inspector = init_with_config(config()).expect("the endpoint binds");
    let mut socket = attach(inspector.endpoint().addr);

    inspector.inspect_node(NodeId(42));

    assert_eq!(next_select(&mut socket), NodeId(42));
}

/// Inspecting an element launches an inspector that is not attached yet, so the
/// node has to wait for it. Without this the window opens on nothing.
#[test]
fn a_node_asked_for_before_anyone_attached_is_delivered_on_arrival() {
    let inspector = init_with_config(config()).expect("the endpoint binds");

    inspector.inspect_node(NodeId(7));

    let mut socket = attach(inspector.endpoint().addr);
    assert_eq!(next_select(&mut socket), NodeId(7));
}
