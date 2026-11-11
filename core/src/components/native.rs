//! This module provides platform-specific native views that can wrap platform-native UI components.

use core::any::type_name;

use crate::{AnyView, Environment, View, layout::StretchAxis};

/// A wrapper for platform-specific native UI components.
///
/// `Native<T>` allows embedding platform-specific UI elements within the view hierarchy.
/// The generic parameter `T` represents the platform-specific component type.
///
/// # Contract
///
/// Rendering a `Native<T>` view directly breaks the contract of this type, which
/// platform-specific backends handle: a debug build panics, and a release build
/// logs the error and renders the fallback, or nothing without one.
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

    /// Returns a shared reference to the wrapped native component.
    #[must_use]
    pub const fn as_inner(&self) -> &T {
        &self.native
    }

    /// Returns a mutable reference to the wrapped native component.
    pub const fn as_inner_mut(&mut self) -> &mut T {
        &mut self.native
    }
}

impl<T: 'static + NativeView> View for Native<T> {
    fn body(self, _env: &Environment) -> impl View {
        self.fallback.unwrap_or_else(|| {
            crate::contract_violation!(
                "Native view ({}) reached `body` without a fallback: the backend does not realize it, so it renders nothing",
                type_name::<T>()
            );
            AnyView::default()
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::metadata::{Metadata, MetadataKey};

    struct Probe;
    impl NativeView for Probe {}

    struct Marker;
    impl MetadataKey for Marker {}

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "without a fallback"))]
    fn a_native_view_without_a_fallback_renders_nothing() {
        let body = Native::new(Probe).body(&Environment::new());
        assert!(AnyView::new(body).is::<()>());
    }

    #[test]
    fn a_native_view_with_a_fallback_renders_it() {
        let body = Native::new(Probe)
            .with_fallback(Metadata::new((), Marker))
            .body(&Environment::new());
        assert!(AnyView::new(body).is::<Metadata<Marker>>());
    }
}
