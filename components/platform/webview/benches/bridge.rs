//! What the bridge costs on the Rust side of the boundary.
//!
//! The plan for making cross-language calls cheaper starts here rather than with
//! a guess: the obvious candidates — JSON versus a binary format, base64 on the
//! common path, coalescing state pushes — pull in different directions, and only
//! one of them is measurable without an engine attached.
//!
//! What this measures is envelope parsing, reply rendering, and state patching:
//! everything `WaterUI` itself does per call. It deliberately does not measure the
//! engine's `JSON.parse`, which is C++ and not ours to improve, nor the IPC hop
//! inside a multi-process browser, which dwarfs both.
//!
//! Run with `cargo bench -p waterui-webview`. On an M-series Mac the first run
//! gave:
//!
//! ```text
//! parse JSON envelope   (100 B)      1.75 µs      (10 KB)  151 µs   (1 MB)  17.6 ms
//! parse binary envelope (100 B)      0.48 µs      (10 KB)   24 µs   (1 MB)   2.5 ms
//! render reply script   (100 B)      0.46 µs      (10 KB)   14 µs   (1 MB)   1.5 ms
//! ```
//!
//! Absolute numbers move a lot with what else the machine is doing, so read the
//! ratios within one run rather than against the figures above.
//!
//! Three things worth knowing before optimising anything:
//!
//! * Tagging integers a JavaScript number cannot hold costs about 20% on top of
//!   serializing a reply, and it used to cost two to four times: the first
//!   version went through a `serde_json::Value` unconditionally, and building
//!   that tree — not walking it — was the whole expense. Serializing once and
//!   scanning the bytes for a 16-digit run keeps the round trip for the messages
//!   that need it, which are almost none. What remains is the scan itself, and
//!   removing that means a `Serializer` wrapper that tags as it writes, which is
//!   a lot of code to save 20% of a cost that is already small.
//!
//! * Base64 is not the expensive part at scale — structure is. A 1 MB binary
//!   payload, base64 and all, parses seven times faster than a 1 MB JSON one.
//!   Keeping JSON off the base64 path is still right for small messages, where
//!   it saves a third of the bytes and two passes, but it is not where large
//!   payloads lose their time.
//! * A megabyte through the bridge costs about 17 ms, which is a dropped frame
//!   at 60 Hz and three at 120. No wire format fixes that: large assets want a
//!   custom URL scheme, where the engine streams them through its own resource
//!   loader and never builds a Rust-side document at all.

use std::hint::black_box;
use std::time::Instant;

use waterui_webview::bridge::{Reply, Request};
use waterui_webview::{IntoJsReply, Json};

/// A JSON payload of roughly `bytes` bytes, shaped like a real message rather
/// than one enormous string.
fn json_envelope(bytes: usize) -> String {
    let entries = (bytes / 24).max(1);
    let items: Vec<String> = (0..entries)
        .map(|index| format!(r#"{{"id":{index},"name":"item-{index}"}}"#))
        .collect();
    format!(
        r#"{{"id":1,"name":"save","json":{{"items":[{}]}}}}"#,
        items.join(",")
    )
}

/// The JSON body of a message of roughly `bytes` bytes, without an envelope.
fn json_payload(bytes: usize) -> String {
    let entries = (bytes / 24).max(1);
    let items: Vec<String> = (0..entries)
        .map(|index| format!(r#"{{"id":{index},"name":"item-{index}"}}"#))
        .collect();
    format!(r#"{{"items":[{}]}}"#, items.join(","))
}

/// A binary payload of `bytes` bytes, base64-encoded as the wire format requires.
fn binary_envelope(bytes: usize) -> String {
    use base64::Engine as _;
    let payload = base64::engine::general_purpose::STANDARD.encode(vec![7_u8; bytes]);
    format!(r#"{{"id":1,"name":"upload","b64":"{payload}"}}"#)
}

fn measure(label: &str, iterations: u32, mut body: impl FnMut()) {
    // Warm up so the first allocation does not colour the result.
    for _ in 0..iterations.min(64) {
        body();
    }
    let start = Instant::now();
    for _ in 0..iterations {
        body();
    }
    let elapsed = start.elapsed();
    let each = elapsed / iterations;
    println!("{label:<44} {each:>12?} per call");
}

fn main() {
    println!("WaterUI bridge, Rust-side cost per call\n");

    for (label, size) in [("100 B", 100_usize), ("10 KB", 10_240), ("1 MB", 1_048_576)] {
        let envelope = json_envelope(size);
        measure(&format!("parse JSON envelope ({label})"), 2_000, || {
            black_box(Request::parse(black_box(&envelope)).expect("well-formed"));
        });
    }

    for (label, size) in [("100 B", 100_usize), ("10 KB", 10_240), ("1 MB", 1_048_576)] {
        let envelope = binary_envelope(size);
        measure(&format!("parse binary envelope ({label})"), 2_000, || {
            black_box(Request::parse(black_box(&envelope)).expect("well-formed"));
        });
    }

    for (label, size) in [("100 B", 100_usize), ("10 KB", 10_240), ("1 MB", 1_048_576)] {
        let reply = Reply::Bytes(vec![7_u8; size]);
        measure(&format!("render reply script ({label})"), 2_000, || {
            black_box(reply.resolve_script(black_box(1)));
        });
    }

    // A value reply is the common path now. It used to be sent as base64 like
    // any other bytes, which is what made `await waterui.invoke(...)` resolve to
    // a base64 string; it goes out as JSON through `RawValue`, so the handler's
    // own bytes land in the envelope rather than being parsed and printed again.
    for (label, size) in [("100 B", 100_usize), ("10 KB", 10_240), ("1 MB", 1_048_576)] {
        let reply = Reply::Json(json_payload(size).into_bytes());
        measure(&format!("render value reply ({label})"), 2_000, || {
            black_box(reply.resolve_script(black_box(1)));
        });
    }

    // Serializing a handler's answer walks it once to tag integers a JavaScript
    // number cannot hold. Measured against a plain `to_vec` of the same value,
    // because the walk is on every JSON reply: it should be small enough not to
    // matter, and it is worth knowing when that stops being true.
    for (label, size) in [("100 B", 100_usize), ("10 KB", 10_240), ("1 MB", 1_048_576)] {
        let value: serde_json::Value =
            serde_json::from_str(&json_payload(size)).expect("well-formed");

        measure(&format!("serialize reply, plain ({label})"), 2_000, || {
            black_box(serde_json::to_vec(black_box(&value)).expect("serializes"));
        });
        measure(
            &format!("serialize reply, tagging ({label})"),
            2_000,
            || {
                black_box(Json(black_box(&value)).into_js_reply().expect("serializes"));
            },
        );
    }
}
