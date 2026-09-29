//! Private, disposable recording boundary between Hydrolysis drawing code and
//! the Cherenkov engine (water-rs/hydrolysis#205, P2/H1).
//!
//! Drawing code records only through [`Recording`]'s fixed API. It exposes no
//! `Deref`, no `scene_mut`, and no encoding escape hatch; the existing
//! `Scene2D` implementation and `DrawContext` adapter live in the
//! implementation file so current WaterUI and hydrolysis-m3 compile unchanged.
//!
//! At H2 both files under this module are deleted: call sites become real
//! Cherenkov recording or retained-layer operations. No
//! `type Recording = cherenkov::Recorder` alias remains.

#[path = "cherenkov.rs"]
mod engine_impl;

pub(crate) use engine_impl::{SceneResources, transform_paint, working_color};
pub use engine_impl::{Recording, VelloDrawContext};
