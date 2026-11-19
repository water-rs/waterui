//! Offscreen view capture and async content settlement for `WaterUI`.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod content_ledger;
pub mod view_renderer;

pub use content_ledger::ContentLedger;
pub use view_renderer::{
    CustomViewRenderer, RenderError, RenderRequest, RenderResult, ViewRenderer,
};
