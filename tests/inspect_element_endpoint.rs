//! Asking to inspect an element reaches an inspector.
//!
//! Driven over a real socket against a real endpoint, because the thing worth
//! checking is the handover: a selection made before anyone is attached has to
//! survive until one is, or the window opens on the wrong thing.
//!
//! The endpoint's launcher only records what it was asked to open, so the
//! tests can check the launch request itself and never start an inspector.

#![cfg(feature = "inspector")]

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

use waterui::inspector::protocol::transport::{read_frame_blocking, write_frame_blocking};
use waterui::inspector::protocol::{
    ChannelSet, InspectorClientMessage, InspectorServerMessage, NodeId, protocol_info,
};
use waterui::inspector::{
    InspectorEndpointInfo, InspectorLauncher, InspectorServerConfig, init_with_config,
};

/// Records the address of every endpoint it is asked to open an inspector on.
struct RecordingLauncher(Sender<SocketAddr>);

impl InspectorLauncher for RecordingLauncher {
    fn launch(&self, endpoint: &InspectorEndpointInfo, _project: Option<&Path>) {
        self.0
            .send(endpoint.addr())
            .expect("the test holds the launch receiver");
    }
}

/// A configuration whose launches are reported on the returned receiver.
fn config() -> (InspectorServerConfig, Receiver<SocketAddr>) {
    let mut config = InspectorServerConfig::new(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        // An ephemeral port keeps concurrent tests from colliding.
        0,
        String::from("test-token"),
        Duration::from_millis(100),
        0.9,
        String::from("test"),
    );
    let (sender, launches) = mpsc::channel();
    config.set_launcher(RecordingLauncher(sender));
    (config, launches)
}

/// Connects and completes the handshake, returning the stream to read from.
fn attach(addr: SocketAddr) -> TcpStream {
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

/// An inspector that is already attached is told to reveal the node at once,
/// and no other inspector is launched.
#[test]
fn an_attached_inspector_is_told_which_node_to_reveal() {
    let (config, launches) = config();
    let inspector = init_with_config(config, "test").expect("the endpoint binds");
    let mut socket = attach(inspector.endpoint().expect("the endpoint listens").addr());

    inspector.inspect_node(NodeId(42));

    assert_eq!(next_select(&mut socket), NodeId(42));
    assert_eq!(launches.try_recv(), Err(TryRecvError::Empty));
}

/// Inspecting an element launches an inspector that is not attached yet, so the
/// node has to wait for it. Without this the window opens on nothing.
#[test]
fn a_node_asked_for_before_anyone_attached_is_delivered_on_arrival() {
    let (config, launches) = config();
    let inspector = init_with_config(config, "test").expect("the endpoint binds");
    let addr = inspector.endpoint().expect("the endpoint listens").addr();

    inspector.inspect_node(NodeId(7));

    assert_eq!(launches.try_recv(), Ok(addr));
    assert_eq!(launches.try_recv(), Err(TryRecvError::Empty));
    let mut socket = attach(addr);
    assert_eq!(next_select(&mut socket), NodeId(7));
}
