//! Markdown family layer of the layered `waterui` dylib chain.
//!
//! Carries the pure-Rust payload of `flow-markdown` (`pulldown-cmark`) and,
//! behind `math`, `waterui-math`. The tree-sitter grammar crates embed in the
//! top image instead — see the comment in `Cargo.toml`. `waterui-text` itself
//! stays in the graphics layer; its `markdown` feature is enabled by
//! `waterui-internal`'s `flow-markdown` feature.

pub use waterui_dylib_graphics;

pub use pulldown_cmark;
#[cfg(feature = "math")]
pub use waterui_math;
