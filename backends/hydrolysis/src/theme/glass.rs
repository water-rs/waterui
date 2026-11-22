//! The glass background look of the theme contract (water-rs/waterui#4495):
//! what a theme returns for a `.background(Glass…)` ([`GlassLook`]).
//!
//! The look states what fills the glass's outline and how it answers
//! interaction; the outline itself is the [`Glass`](waterui::background::Glass)'s
//! shape on the content's frame, which Hydrolysis supplies. Hydrolysis
//! realizes the look as the surface of a chrome — the material, its border
//! and its elevation — recorded behind the content, with every value that
//! follows the inputs a live operand.

use super::{Backdrop, Border, Driven, Elevation, Motion};

/// The look of a `.background(Glass…)`: what fills the glass's outline and
/// how it answers interaction. The outline is the `Glass`'s shape on the
/// content's frame, which Hydrolysis supplies.
///
/// The [`Backdrop`] material, the border and the elevation are the same
/// vocabulary a [`Surface`](super::Surface) is made of, and are recorded the
/// way a surface records them. Their [`Driven`] values follow the live
/// [`Inputs`](super::Inputs) — the frame, the interaction state and the press
/// point; the glass's value type is `()`.
#[derive(Debug, Clone)]
pub struct GlassLook {
    material: Backdrop,
    motion: Motion,
    border: Option<Driven<Border>>,
    elevation: Driven<Elevation>,
}

impl GlassLook {
    /// Fills the outline with `material`; interaction changes run with
    /// `motion`.
    #[must_use]
    pub fn new(material: Backdrop, motion: Motion) -> Self {
        Self {
            material,
            motion,
            border: None,
            elevation: Driven::constant(Elevation::NONE),
        }
    }

    /// Strokes `border` inside the outline's edge.
    #[must_use]
    pub fn border(mut self, border: impl Into<Driven<Border>>) -> Self {
        self.border = Some(border.into());
        self
    }

    /// Casts `elevation`'s shadows.
    #[must_use]
    pub fn elevation(mut self, elevation: impl Into<Driven<Elevation>>) -> Self {
        self.elevation = elevation.into();
        self
    }

    /// The material filling the outline.
    #[must_use]
    pub const fn material(&self) -> &Backdrop {
        &self.material
    }

    /// The motion interaction changes run with.
    #[must_use]
    pub const fn motion(&self) -> &Motion {
        &self.motion
    }

    /// The border; `None` draws none.
    #[must_use]
    pub const fn get_border(&self) -> Option<&Driven<Border>> {
        self.border.as_ref()
    }

    /// The elevation.
    #[must_use]
    pub const fn get_elevation(&self) -> &Driven<Elevation> {
        &self.elevation
    }
}
