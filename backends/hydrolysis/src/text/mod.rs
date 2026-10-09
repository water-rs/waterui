//! Text shaping, measurement, drawing and editing behind the session's text
//! engine.
//!
//! Everything text in the backend routes through the seam in [`engine`]: its
//! renderer-free value types in [`types`], the resolved input in [`input`],
//! the shared service in [`service`], and exactly one engine binding — [`SessionTextEngine`] — so there is no `dyn` dispatch
//! and no runtime choice. Each backend names its own shaper behind the same
//! traits.

#[cfg(feature = "cherenkov")]
mod engine;
#[cfg(feature = "cherenkov")]
mod input;
#[cfg(feature = "cherenkov")]
mod parley_engine;
#[cfg(feature = "cherenkov")]
mod service;
#[cfg(all(test, feature = "cherenkov"))]
mod tests;
pub mod types;

#[cfg(all(test, feature = "cherenkov"))]
pub use engine::Affinity;
#[cfg(feature = "cherenkov")]
pub use engine::FontFamilyResolution;
#[cfg(feature = "cherenkov")]
pub use engine::{TailMark, TextEngine, TextLayout, TextPosition, TextSelection};
#[cfg(feature = "cherenkov")]
pub use input::{ResolvedTextLayoutInput, resolve_text_layout_input};
#[cfg(feature = "cherenkov")]
pub use service::TextService;

#[cfg(feature = "cherenkov")]
pub use parley_engine::fonts;

#[cfg(feature = "cherenkov")]
pub type SessionTextEngine = parley_engine::ParleyEngine;
#[cfg(feature = "cherenkov")]
pub type SessionTextLayout = <SessionTextEngine as TextEngine>::Layout;
