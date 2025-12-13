//! This module provides platform-specific native views that can wrap platform-native UI components.

use core::any::type_name;

use crate::{AnyView, Environment, View, layout::StretchAxis};

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
pub struct Native<T: NativeView> {
    native: T,
    fallback: Option<AnyView>,
}

impl<T: NativeView> Native<T> {
    /// Creates a new `Native<T>` view wrapping the given native component.
    ///
    /// # Arguments
    ///
    /// * `native` - The platform-specific native UI component to wrap.
    pub const fn new(native: T) -> Self {
        Self {
            native,
            fallback: None,
        }
    }

    /// Sets a fallback view to be used if the native view cannot be rendered.
    ///
    /// # Arguments
    /// * `fallback` - The fallback view to display.
    #[must_use]
    pub fn with_fallback(mut self, fallback: impl View) -> Self {
        self.fallback = Some(AnyView::new(fallback));
        self
    }

    /// Consumes the wrapper and returns the inner native component.
    #[must_use]
    pub fn into_inner(self) -> T {
        self.native
    }
}

impl<T: 'static + NativeView> View for Native<T> {
    #[allow(unused)]
    #[allow(clippy::needless_return)]
    fn body(self, _env: &Environment) -> impl View {
        self.fallback
            .unwrap_or_else(|| panic!("Native view ({})", type_name::<T>()))
    }

    fn stretch_axis(&self) -> StretchAxis {
        NativeView::stretch_axis(&self.native)
    }
}

/// A trait for all views handled by the native backend.
///
/// This includes:
/// - Configurable views (`TextField`, Slider, Toggle, etc.)
/// - Raw views (Color, Spacer, Divider, Container, etc.)
///
/// The native backend uses this trait to query layout behavior.
pub trait NativeView {
    /// Which axis (or axes) this view stretches to fill available space.
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
}
