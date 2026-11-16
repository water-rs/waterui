//! This module provides platform-specific native views that can wrap platform-native UI components.

use core::any::type_name;

use crate::backend::Backend;
use crate::{Environment, View, layout::StretchAxis};

/// A wrapper for platform-specific native UI components.
///
/// `Native<T>` allows embedding platform-specific UI elements within the view hierarchy.
/// The generic parameter `T` represents the platform-specific component type.
///
/// # Contract
///
/// Every backend realizes every native component it can receive: a backend
/// built on `waterui-backend-core` implements each one. `body` is therefore
/// unreachable, and reaching it panics.
#[derive(Debug)]
pub struct Native<T: NativeView> {
    native: T,
}

impl<T: NativeView> Native<T> {
    /// Creates a new `Native<T>` view wrapping the given native component.
    ///
    /// # Arguments
    ///
    /// * `native` - The platform-specific native UI component to wrap.
    pub const fn new(native: T) -> Self {
        Self { native }
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
        never_expands::<T>();
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.native, env, cx)
    }
}

/// Panics: a native view never expands, because every backend realizes every
/// native component.
fn never_expands<T>() {
    unreachable!(
        "native view {} reached `body`: every backend realizes every native component",
        type_name::<T>()
    );
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

    use crate::AnyView;

    struct Probe;
    impl NativeView for Probe {}

    #[test]
    #[should_panic(expected = "reached `body`")]
    fn a_native_view_never_expands() {
        let _ = AnyView::new(Native::new(Probe).body(&Environment::new()));
    }
}
