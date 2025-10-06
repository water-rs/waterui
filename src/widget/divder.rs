//! Module that provides a simple divider component.
//!
//! This module contains the `Divider` component which is a visual separator
//! that can be used to create a clear distinction between different sections
//! or elements in a user interface.

use waterui_color::Color;
use waterui_core::View;

use crate::ViewExt;

/// A divider component that can be used to separate content.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct Divider;

impl View for Divider {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Color::srgb_f32(0.8, 0.8, 0.8).height(1.0)
    }
}
