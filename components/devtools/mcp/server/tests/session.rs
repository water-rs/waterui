//! Drives `waterui_mcp::serve` over a `DuplexTransport`: initialize, list the
//! tools, find a button, click it through `act`, and screenshot.

use aither_mcp::protocol::{
    CallToolParams, CallToolResult, Content, InitializeParams, InitializeResult, JsonRpcRequest,
    ListToolsResult,
};
use aither_mcp::transport::{DuplexTransport, Transport};
use futures_lite::future::block_on;
use waterui::Binding;
use waterui::component::{button, toggle, vstack};
use waterui::text;
use waterui_mcp::{ServerInfo, serve};
use waterui_testing::{OffscreenApp, install_default_theme, ui};

fn mount() -> OffscreenApp {
    let enabled = Binding::bool(false);
    let count = Binding::container(0_i32);
    let label = Binding::container(String::from("count: 0"));
    ui().theme(install_default_theme)
        .viewport(200, 100)
        .mount_offscreen(move || {
            let count = count.clone();
            let label = label.clone();
            let label_for_action = label.clone();
            vstack((
                toggle("Enable", &enabled),
                button("Increment").action(move || {
                    let next = count.get() + 1;
                    count.set(next);
                    label_for_action.set(format!("count: {next}"));
                }),
                text!("{label}"),
            ))
        })
}

fn call_tool(
    client: &mut DuplexTransport,
    name: &str,
    arguments: serde_json::Value,
) -> CallToolResult {
    let response = block_on(client.request(JsonRpcRequest::with_params(
        0_i64,
        "tools/call",
        CallToolParams {
            name: name.to_owned(),
            arguments,
        },
    )))
    .expect("tools/call transport error");
    assert!(
        response.error.is_none(),
        "tools/call {name}: {:?}",
        response.error
    );
    serde_json::from_value(response.result.expect("tools/call returned no result"))
        .expect("tools/call result must deserialize")
}

fn tool_text(result: &CallToolResult) -> &str {
    assert_eq!(result.content.len(), 1, "expected exactly one content item");
    let Content::Text(content) = &result.content[0] else {
        panic!("expected text content, got {:?}", result.content[0]);
    };
    &content.text
}

#[test]
fn mcp_session_drives_a_mounted_app() {
    let (mut client, server) = DuplexTransport::pair();
    let info = ServerInfo {
        name: "waterui-mcp-test".to_owned(),
        version: Some("0.0.0".to_owned()),
    };
    let server_thread = std::thread::spawn(move || serve(server, mount, info));

    let response = block_on(client.request(JsonRpcRequest::with_params(
        0_i64,
        "initialize",
        InitializeParams::default(),
    )))
    .expect("initialize transport error");
    let initialized: InitializeResult =
        serde_json::from_value(response.result.expect("initialize returned no result"))
            .expect("initialize result must deserialize");
    assert!(
        initialized
            .instructions
            .is_some_and(|text| !text.is_empty()),
        "server should advertise its instructions"
    );

    let response = block_on(client.request(JsonRpcRequest::new(0_i64, "tools/list")))
        .expect("tools/list transport error");
    let listed: ListToolsResult =
        serde_json::from_value(response.result.expect("tools/list returned no result"))
            .expect("tools/list result must deserialize");
    let names = listed
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    // `Tools` stores registrations in a `BTreeMap`, so `tools/list` is
    // alphabetical.
    assert_eq!(
        names,
        [
            "act",
            "find",
            "key",
            "pointer",
            "restart",
            "screenshot",
            "snapshot",
            "type_text",
            "wait",
        ]
    );

    let snapshot = call_tool(&mut client, "snapshot", serde_json::json!({}));
    let text = tool_text(&snapshot);
    assert!(text.contains("revision="), "snapshot text: {text}");
    assert!(text.contains("\"Enable\""), "snapshot text: {text}");
    assert!(
        text.contains("button \"Increment\""),
        "snapshot text: {text}"
    );
    assert!(text.contains("count: 0"), "snapshot text: {text}");

    let found = call_tool(
        &mut client,
        "find",
        serde_json::json!({"label": "Increment"}),
    );
    let line = tool_text(&found);
    let node: u64 = line
        .split_whitespace()
        .find(|token| token.starts_with('#'))
        .and_then(|token| token[1..].parse().ok())
        .unwrap_or_else(|| panic!("find returned no node id: {line}"));

    let clicked = call_tool(
        &mut client,
        "act",
        serde_json::json!({"node": node, "action": "click"}),
    );
    assert!(!clicked.is_error);
    assert!(
        tool_text(&clicked).contains("count: 1"),
        "act should have incremented the counter"
    );

    let shot = call_tool(&mut client, "screenshot", serde_json::json!({}));
    assert!(!shot.is_error);
    // aither-mcp 0.4.0 renders binary tool results as a text placeholder; the
    // image-content fix is merged as lexoliu/aither#53 but unreleased — this
    // assertion flips to `Content::Image` when aither-mcp >= 0.4.1 is picked
    // up.
    assert!(
        tool_text(&shot).starts_with("[binary tool result: image/png"),
        "screenshot content: {:?}",
        shot.content
    );

    block_on(client.close()).expect("close transport");
    server_thread
        .join()
        .expect("server thread panicked")
        .expect("serve returned an error");
}
