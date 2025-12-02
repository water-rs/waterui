//! This module provides platform-specific native views that can wrap platform-native UI components.

use core::any::type_name;

use crate::{Environment, View, layout::StretchAxis};

/// A wrapper for platform-specific native UI components.
///
/// `Native<T>` allows embedding platform-specific UI elements within the view hierarchy.
/// The generic parameter `T` represents the platform-specific component type.
///
/// # Panics
///
/// Attempting to render a `Native<T>` view directly will panic. This type is intended
/// to be handled by platform-specific rendering backends.
#[derive(Debug)]
pub struct Native<T: NativeView>(pub T);

impl<T: 'static + NativeView> View for Native<T> {
    #[allow(unused)]
    #[allow(clippy::needless_return)]
    fn body(self, _env: &Environment) -> impl View {
        panic!("Native view ({})", type_name::<T>());
        return;
    }

    fn stretch_axis(&self) -> StretchAxis {
        NativeView::stretch_axis(&self.0)
    }
}

/// A trait for all views handled by the native backend.
///
/// This includes:
/// - Configurable views (TextField, Slider, Toggle, etc.)
/// - Raw views (Color, Spacer, Divider, Container, etc.)
///
/// The native backend uses this trait to query layout behavior.
pub trait NativeView {
    /// Which axis (or axes) this view stretches to fill available space.
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
}
