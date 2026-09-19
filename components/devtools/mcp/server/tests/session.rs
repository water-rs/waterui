//! Drives `waterui_mcp::serve` over a `DuplexTransport`: initialize, list the
//! tools, find a button, click it through `act`, and screenshot.

use aither_mcp::protocol::{
    CallToolParams, CallToolResult, Content, InitializeParams, InitializeResult, JsonRpcRequest,
    ListToolsResult,
};
use aither_mcp::transport::{DuplexTransport, Transport};
use futures_lite::future::block_on;
use waterui::component::{button, field, toggle, vstack};
use waterui::text;
use waterui::{Binding, Str};
use waterui_mcp::{ServerInfo, serve};
use waterui_testing::{OffscreenApp, theme_with, ui};

fn mount() -> OffscreenApp {
    let enabled = Binding::bool(false);
    let count = Binding::container(0_i32);
    let label = Binding::container(String::from("count: 0"));
    let name = Binding::container(Str::from_static(""));
    ui().theme(theme_with(hydrolysis_m3::install))
        .viewport(200, 100)
        .mount_offscreen(move || {
            let count = count.clone();
            let label = label.clone();
            let label_for_action = label.clone();
            let name = name.clone();
            vstack((
                toggle("Enable", &enabled),
                field("Name", &name),
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

/// Extracts the first `#<id>` token from a `find`/`snapshot` text payload.
fn first_node_id(text: &str) -> u64 {
    text.split_whitespace()
        .find(|token| token.starts_with('#'))
        .and_then(|token| token[1..].parse().ok())
        .unwrap_or_else(|| panic!("expected a node id in: {text}"))
}

#[test]
fn mcp_session_drives_a_mounted_app() {
    let (mut client, server) = DuplexTransport::pair();
    let info = ServerInfo {
        name: "waterui-mcp-test".to_owned(),
        version: Some("0.0.0".to_owned()),
    };
    let server_thread = std::thread::spawn(move || serve(server, mount, info));

    initialize(&mut client);

    let snapshot = call_tool(&mut client, "snapshot", serde_json::json!({}));
    let text = tool_text(&snapshot);
    assert!(text.contains("revision="), "snapshot text: {text}");
    assert!(text.contains("\"Enable\""), "snapshot text: {text}");
    assert!(
        text.contains("button \"Increment\""),
        "snapshot text: {text}"
    );
    assert!(text.contains("count: 0"), "snapshot text: {text}");
    let root = first_node_id(text);

    let found = call_tool(
        &mut client,
        "find",
        serde_json::json!({"label": "Increment"}),
    );
    let node = first_node_id(tool_text(&found));

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

    screenshot_and_advance(&mut client, node);
    state_filtered_and_scoped_find(&mut client, root);
    anchored_pointer(&mut client, node);
    act_error_paths(&mut client, node);
    focus_cycle(&mut client);
    inverted_wait(&mut client);

    block_on(client.close()).expect("close transport");
    server_thread
        .join()
        .expect("server thread panicked")
        .expect("serve returned an error");
}

fn initialize(client: &mut DuplexTransport) {
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
    let mut expected = waterui_mcp::SESSION_TOOL_NAMES;
    expected.sort_unstable();
    assert_eq!(names, expected);
}

/// `screenshot` returns the settled frame; `settle: false` + `advance` splits
/// input dispatch from clock advancement.
fn screenshot_and_advance(client: &mut DuplexTransport, node: u64) {
    let shot = call_tool(client, "screenshot", serde_json::json!({}));
    assert!(!shot.is_error);
    let [Content::Image(image)] = shot.content.as_slice() else {
        panic!("expected image content, got {:?}", shot.content);
    };
    assert_eq!(image.mime_type, "image/png");

    // `settle: false` queues the action without running the runtime to
    // quiescence; `advance` then steps the virtual clock until it lands.
    let queued = call_tool(
        client,
        "act",
        serde_json::json!({"node": node, "action": "click", "settle": false}),
    );
    assert!(!queued.is_error);
    assert!(
        tool_text(&queued).contains("queued"),
        "unsettled act response: {}",
        tool_text(&queued)
    );

    let advanced = call_tool(client, "advance", serde_json::json!({"duration_ms": 1000}));
    assert!(!advanced.is_error);
    let text = tool_text(&advanced);
    assert!(
        text.contains("advanced 1000ms; settled"),
        "advance text: {text}"
    );
    assert!(
        text.contains("count: 2"),
        "advance should land the queued click: {text}"
    );

    let frame = call_tool(
        client,
        "advance",
        serde_json::json!({"duration_ms": 16, "screenshot": true}),
    );
    assert!(!frame.is_error);
    let [Content::Image(image)] = frame.content.as_slice() else {
        panic!("expected image content, got {:?}", frame.content);
    };
    assert_eq!(image.mime_type, "image/png");
}

/// State filters (`checked`, `enabled`) and scope anchors (`within`,
/// `children_of`) narrow `find` to specific nodes.
fn state_filtered_and_scoped_find(client: &mut DuplexTransport, root: u64) {
    let unchecked = call_tool(
        client,
        "find",
        serde_json::json!({"label": "Enable", "checked": false, "enabled": true}),
    );
    assert!(
        !unchecked.is_error,
        "state-filtered find: {}",
        tool_text(&unchecked)
    );
    let checked = call_tool(
        client,
        "find",
        serde_json::json!({"label": "Enable", "checked": true}),
    );
    assert!(checked.is_error, "checked toggle should not match");

    let scoped = call_tool(
        client,
        "find",
        serde_json::json!({"within": root, "label": "Increment"}),
    );
    assert!(!scoped.is_error, "scoped find: {}", tool_text(&scoped));
    let conflict = call_tool(
        client,
        "find",
        serde_json::json!({"within": root, "children_of": root}),
    );
    assert!(conflict.is_error, "within + children_of must conflict");
}

/// Node-anchored pointer input: normalized coordinates resolve against the
/// node's bounds, `magnify` requires `factor`, `scroll` accepts `unit`.
fn anchored_pointer(client: &mut DuplexTransport, node: u64) {
    let tapped = call_tool(
        client,
        "pointer",
        serde_json::json!({"kind": "tap", "node": node, "x": 0.5, "y": 0.5}),
    );
    assert!(
        tool_text(&tapped).contains("count: 3"),
        "anchored tap should increment: {}",
        tool_text(&tapped)
    );

    let magnify = call_tool(
        client,
        "pointer",
        serde_json::json!({"kind": "magnify", "node": node, "x": 0.5, "y": 0.5}),
    );
    assert!(magnify.is_error, "magnify without factor must fail");
    let scroll = call_tool(
        client,
        "pointer",
        serde_json::json!({"kind": "scroll", "node": node, "x": 0.5, "y": 0.5, "dy": -3.0, "unit": "line"}),
    );
    assert!(!scroll.is_error, "line scroll: {}", tool_text(&scroll));
}

/// `act` rejects missing nodes, unsupported actions, and drives the
/// app-level `clear_focus`.
fn act_error_paths(client: &mut DuplexTransport, node: u64) {
    let no_node = call_tool(client, "act", serde_json::json!({"action": "click"}));
    assert!(
        tool_text(&no_node).contains("requires `node`"),
        "missing node: {}",
        tool_text(&no_node)
    );
    let unsupported = call_tool(
        client,
        "act",
        serde_json::json!({"node": node, "action": "scroll_into_view"}),
    );
    assert!(
        tool_text(&unsupported).contains("does not support"),
        "unsupported action: {}",
        tool_text(&unsupported)
    );
}

/// Tapping the text field lands UI focus; `wait.focus` observes it and
/// `act clear_focus` releases it.
fn focus_cycle(client: &mut DuplexTransport) {
    let field_found = call_tool(
        client,
        "find",
        serde_json::json!({"role": "text_input", "label": "Name"}),
    );
    let field_node = first_node_id(tool_text(&field_found));
    let focus_tap = call_tool(
        client,
        "pointer",
        serde_json::json!({"kind": "tap", "node": field_node, "x": 0.5, "y": 0.5}),
    );
    assert!(!focus_tap.is_error);
    let focused = call_tool(
        client,
        "wait",
        serde_json::json!({"focus": {"role": "text_input", "label": "Name"}, "timeout_ms": 1000}),
    );
    assert!(
        tool_text(&focused).starts_with("fulfilled"),
        "focus wait: {}",
        tool_text(&focused)
    );

    let cleared = call_tool(client, "act", serde_json::json!({"action": "clear_focus"}));
    assert!(!cleared.is_error, "clear_focus: {}", tool_text(&cleared));
    let refocused = call_tool(
        client,
        "wait",
        serde_json::json!({"focus": {"role": "text_input", "label": "Name"}, "timeout_ms": 200}),
    );
    assert!(
        tool_text(&refocused).starts_with("timed out"),
        "cleared focus should not re-match: {}",
        tool_text(&refocused)
    );
}

/// An inverted expectation that never holds runs its full timeout and then
/// reports `fulfilled`.
fn inverted_wait(client: &mut DuplexTransport) {
    let forbidden = call_tool(
        client,
        "wait",
        serde_json::json!({"exists": {"label": "Crash", "inverted": true}, "timeout_ms": 100}),
    );
    assert!(
        tool_text(&forbidden).starts_with("fulfilled"),
        "inverted wait: {}",
        tool_text(&forbidden)
    );
}
