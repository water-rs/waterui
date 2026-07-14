//! Reactive color resolution for GPU renderers.

use core::fmt;

use waterui_core::{
    Computed, Environment, Signal, SignalExt, flatten_signal, reactive::watcher::BoxWatcherGuard,
};

use crate::{
    color::{Color, ResolvedColor},
    gpu_surface::RedrawHandle,
};

/// A GPU renderer-owned color signal resolved through its view environment.
///
/// Both the outer color signal and the currently selected color's environment
/// resolution are observed. Changing the outer color switches the inner
/// subscription without rebuilding the view tree.
#[doc(hidden)]
pub struct ReactiveColor {
    resolved: Computed<ResolvedColor>,
    redraw_guard: Option<BoxWatcherGuard>,
}

impl fmt::Debug for ReactiveColor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReactiveColor")
            .field("resolved", &self.resolved)
            .finish_non_exhaustive()
    }
}

impl ReactiveColor {
    /// Creates a reactive color whose subscriptions will be installed by
    /// [`Self::install`].
    #[must_use]
    pub fn new(source: &Computed<Color>, environment: &Environment) -> Self {
        let environment = environment.clone();
        Self {
            resolved: flatten_signal(source.map(move |color| color.resolve(&environment))),
            redraw_guard: None,
        }
    }

    /// Installs subscriptions for this renderer instance.
    pub fn install(&mut self, redraw: &RedrawHandle) {
        let redraw = redraw.clone();
        self.redraw_guard = Some(self.resolved.watch(move |_| redraw.request_redraw()));
    }

    /// Returns the current environment-resolved color.
    #[must_use]
    pub fn get(&self) -> ResolvedColor {
        self.resolved.get()
    }
}

#[cfg(test)]
mod tests {
    use waterui_core::{Binding, Computed, Environment, Signal, resolve::Resolvable};

    use super::*;

    #[derive(Clone, Debug)]
    struct BoundColor(Binding<ResolvedColor>);

    impl Resolvable for BoundColor {
        type Resolved = ResolvedColor;

        fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
            self.0.clone()
        }
    }

    #[test]
    fn inner_resolved_color_change_requests_redraw() {
        let resolved = Binding::container(ResolvedColor::default());
        let source = Computed::constant(Color::new(BoundColor(resolved.clone())));
        let redraw = RedrawHandle::new();
        let mut color = ReactiveColor::new(&source, &Environment::new());
        color.install(&redraw);

        resolved.set(ResolvedColor {
            red: 1.0,
            ..ResolvedColor::default()
        });

        assert!(redraw.take_dirty());
        assert!((color.get().red - 1.0).abs() < f32::EPSILON);
    }
}
