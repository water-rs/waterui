//! Module that provides a simple divider component.
//!
//! This module contains the `Divider` component which is a visual separator
//! that can be used to create a clear distinction between different sections
//! or elements in a user interface.

use waterui_core::raw_view;

/// A divider component that can be used to separate content.
#[derive(Debug)]
#[must_use]
pub struct Divider;

raw_view!(Divider);

pub(crate) mod ffi {
    use super::Divider;
    use waterui_core::ffi_view;

    ffi_view!(Divider, waterui_divider_id);
}
