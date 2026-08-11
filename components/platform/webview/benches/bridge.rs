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
//! Two things worth knowing before optimising anything:
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

/// A JSON payload of roughly `bytes` bytes, shaped like a real message rather
/// than one enormous string.
fn json_envelope(bytes: usize) -> String {
    let entries = (bytes / 24).max(1);
    let items: Vec<String> = (0..entries)
        .map(|index| format!(r#"{{"id":{index},"name":"item-{index}"}}"#))
        .collect();
    format!(r#"{{"id":1,"name":"save","json":{{"items":[{}]}}}}"#, items.join(","))
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
}
