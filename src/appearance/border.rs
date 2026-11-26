//! Border modifier for applying borders to views.
//!
//! # Examples
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::border::Border;
//!
//! fn border_example() -> impl View {
//!     // Simple border
//!     text!("Hello")
//!         .border(Color::red(), 2.0)
//! }
//!
//! fn rounded_border_example() -> impl View {
//!     // Rounded border
//!     text!("Rounded")
//!         .border_with(Border::new(Color::blue(), 1.0).corner_radius(8.0))
//! }
//! ```

pub use waterui_shape::border::Border;
