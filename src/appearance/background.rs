//! Background styling for views.
//!
//! Backgrounds fill the bounds behind a view: any [`View`](waterui_core::View) — a solid
//! [`Color`](waterui_graphics::color::Color), a [`gradient`](crate::gradient) view, an image — and
//! the platform's blur [`Material`]s and Liquid [`Glass`].
//!
//! # Rendering Model
//!
//! A [`View`](waterui_core::View) passed to [`background`](crate::ViewExt::background) is composed
//! by the framework in Rust via `BackgroundView`.
//!
//! `Material` and `Glass` are different: they become `MaterialBackground` and
//! `GlassBackground` metadata that each backend realizes, because they treat
//! the content behind the view rather than drawing a view of their own. Every
//! backend realizes `Material` (see its contract); `Glass` is an asymmetric
//! primitive.
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::layout::UnitPoint;
//!
//! text!("Hello").background(Color::srgb(20, 40, 80));
//! text!("Hello").background(Gradient::linear(
//!     [Color::srgb(0, 0, 0), Color::srgb(255, 255, 255)],
//!     UnitPoint::TOP,
//!     UnitPoint::BOTTOM,
//! ));
//! ```

pub use waterui_shape::background::{
    ColorSignalBackground, Glass, GlassBackground, GlassStyle, IntoBackground, Material,
    MaterialBackground, MaterialGroup, ViewBackground,
};
