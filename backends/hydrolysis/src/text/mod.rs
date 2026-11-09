//! Text shaping, measurement, drawing and editing behind the session's text
//! engine.
//!
//! Everything text in the backend routes through the seam in [`engine`]: the
//! resolved input in [`input`], the shared service in [`service`], and the
//! generic engine `E` each renderer carries — every painter entry point
//! constructs its own concrete engine ([`ParleyEngine`] today), so there is
//! no `dyn` dispatch and no runtime choice.

mod declared;
mod engine;
mod input;
#[cfg(hydrolysis_gpu_painter)]
mod parley_engine;
mod service;
#[cfg(test)]
mod tests;
pub mod types;

pub use declared::DeclaredFonts;
#[cfg(test)]
pub use engine::Affinity;
pub use engine::FontFamilyResolution;
pub use engine::{TailMark, TextEngine, TextLayout, TextPosition, TextSelection};
pub use input::{ResolvedTextLayoutInput, resolve_text_layout_input};
pub use service::TextScenes;
pub use service::TextService;

#[cfg(any(test, hydrolysis_hwui))]
mod hwui_engine;

#[cfg(any(test, hydrolysis_hwui))]
pub use hwui_engine::HwuiTextEngine;
#[cfg(hydrolysis_gpu_painter)]
pub use parley_engine::ParleyEngine;
#[cfg(hydrolysis_gpu_painter)]
pub use parley_engine::fonts;
