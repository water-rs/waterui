//! Private, disposable recording boundary between Hydrolysis drawing code and
//! the legacy Vello scene (water-rs/hydrolysis#205, P2).
//!
//! Drawing code records only through [`Recording`]'s fixed API. It exposes no
//! `Recording`, no `Deref`, no `scene_mut`, and no encoding escape hatch;
//! the existing `Scene2D` implementation and `DrawContext` adapter live in the
//! legacy implementation file so current WaterUI and hydrolysis-m3 compile
//! unchanged.
//!
//! At cutover both files under this module and `crate::engine::vello_backend`
//! are deleted: call sites become real Cherenkov recording or retained-layer
//! operations. No `type Recording = cherenkov::Recorder` alias remains.

#[path = "vello.rs"]
mod legacy;

pub use legacy::{Recording, VelloDrawContext};

// Encoding types tests inspect through `legacy_scene()` — vello_encoding
// itself is only named inside `legacy` (the `vello.rs` file).
#[cfg(test)]
pub(crate) use legacy::{PathTag, Resolver, Transform};
