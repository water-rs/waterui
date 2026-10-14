//! The endpoint's published advertisement carries the handshake identity.
//!
//! One test in its own binary: the advertisement file is named by pid, so the
//! assertion owns its process and never collides with another endpoint test.

#![cfg(feature = "inspector")]

use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::time::Duration;

use waterui::inspector::protocol::transport::{read_frame_blocking, write_frame_blocking};
use waterui::inspector::protocol::{
    ChannelSet, InspectorClientMessage, InspectorServerMessage, build_identity,
};
use waterui::inspector::{InspectorServerConfig, init_with_config};

#[test]
fn the_advertised_identity_matches_the_handshake() {
    let inspector = init_with_config(
        InspectorServerConfig {
            host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            token: String::from("identity-test-token"),
            task_window: Duration::from_millis(100),
            stall_ratio: 0.9,
            app_name: String::from("identity-test"),
            framework_version: waterui::FRAMEWORK_VERSION.to_string(),
        },
        "test",
    )
    .expect("the endpoint binds");
    let addr = inspector.endpoint().expect("the endpoint listens").addr;

    let mut socket =
        TcpStream::connect_timeout(&addr, Duration::from_secs(5)).expect("the endpoint accepts");
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("a read timeout can be set");
    socket
        .set_write_timeout(Some(Duration::from_secs(5)))
        .expect("a write timeout can be set");
    write_frame_blocking(
        &mut socket,
        &InspectorClientMessage::Hello {
            token: String::from("identity-test-token"),
            identity: build_identity(waterui::FRAMEWORK_VERSION),
            channels: ChannelSet::empty(),
        },
    )
    .expect("the handshake is written");

    let welcome: InspectorServerMessage =
        read_frame_blocking(&mut socket).expect("the endpoint answers the handshake");
    let InspectorServerMessage::Welcome { identity, .. } = welcome else {
        panic!("expected a welcome, got {welcome:?}");
    };
    assert_eq!(identity, build_identity(waterui::FRAMEWORK_VERSION));

    // A reader of the discovery file sees exactly the identity a real
    // connection checks against.
    let advertised = waterui::inspector::protocol::discovery::list()
        .expect("the advertisement is readable")
        .into_iter()
        .find(|entry| entry.pid == std::process::id() && entry.addr == addr)
        .expect("this endpoint is advertised");
    assert_eq!(advertised.identity, identity);
}
