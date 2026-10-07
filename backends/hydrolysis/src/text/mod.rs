//! Text shaping, measurement, drawing and editing behind the session's text
//! engine.
//!
//! Everything text in the backend routes through the seam in [`engine`]: the
//! resolved input in [`input`], the shared service in [`service`], and exactly
//! one engine binding — [`SessionTextEngine`] — so there is no `dyn` dispatch
//! and no runtime choice. Each backend names its own shaper behind the same
//! traits.

mod declared;
mod engine;
mod input;
mod parley_engine;
mod service;
#[cfg(test)]
mod tests;

pub use declared::DeclaredFonts;
#[cfg(test)]
pub use engine::Affinity;
pub use engine::FontFamilyResolution;
pub use engine::{TailMark, TextEngine, TextLayout, TextPosition, TextSelection};
pub use input::{ResolvedTextLayoutInput, resolve_text_layout_input};
pub use service::TextService;

pub use parley_engine::fonts;

pub type SessionTextEngine = parley_engine::ParleyEngine;
pub type SessionTextLayout = <SessionTextEngine as TextEngine>::Layout;
