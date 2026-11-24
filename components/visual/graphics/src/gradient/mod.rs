//! Gradient views.
//!
//! Every gradient is authored in unit space — `[0, 1]` on both axes, `(0, 0)`
//! the top-left of the view — and fills the bounds it is laid out at.
//! A radial gradient's radii are the exception to per-axis scaling: they are
//! fractions of the box's shorter side, so a radial gradient stays circular
//! on a non-square box (see [`Gradient::radial`]).
//!
//! - [`Gradient`] — a linear, radial or angular gradient whose stops resolve
//!   in the environment and follow signals, rendered by the backend as a
//!   native gradient where it has one.
//! - [`MeshGradient`] — a mesh gradient whose colours and control points
//!   resolve in the environment and follow signals; a change updates the
//!   recorded paint in place.
//! - `AnimatedMeshGradient` — a 4×4 mesh whose vertices and colours flow
//!   with the canvas clock, drawn through [`Canvas`](crate::canvas::Canvas).
//! - `FlowingGradient` — drifting noise-driven colour bands painted by a
//!   [`ShaderPaintView`](crate::ShaderPaintView).

mod animated_mesh;
#[cfg(any(feature = "gpu", feature = "cpu"))]
mod flowing;
mod mesh;
mod paint;

pub use animated_mesh::{
    ANIMATED_MESH_PALETTE_LEN, AnimatedMeshGradient, AnimatedMeshGradientConfig,
};
#[cfg(any(feature = "gpu", feature = "cpu"))]
pub use flowing::FlowingGradient;
pub use mesh::MeshGradient;
pub use paint::{Gradient, GradientStops, GradientType};
