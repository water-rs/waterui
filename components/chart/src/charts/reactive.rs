//! Reactive wrapper for chart renderers.
//!
//! Provides automatic data synchronization between Signals and GPU renderers.

extern crate alloc;

use alloc::sync::Arc;
use core::any::Any;
use core::future::Future;

use nami::Signal;
use std::sync::Mutex;
use std::time::Instant;
use waterui_graphics::{GpuContext, GpuFrame, GpuView};

use crate::animation::{AnimationConfig, ChartAnimator};
use crate::renderer::ChartRenderer;

/// A reactive wrapper that synchronizes Signal changes to any ChartRenderer.
///
/// This is a single generic wrapper that works with all chart types,
/// using the `ChartRenderer::Data` associated type.
///
/// # Example
///
/// ```ignore
/// let data: Binding<Vec<DataPoint>> = binding(vec![...]);
/// let renderer = BarChartRenderer::new();
/// let reactive = SignalRenderer::new(renderer, data);
/// ```
pub struct SignalRenderer<R, S>
where
    R: ChartRenderer,
    S: Signal<Output = R::Data> + 'static,
{
    /// The underlying chart renderer.
    inner: R,
    /// The data signal being watched.
    signal: S,
    /// Flag indicating a data update is pending.
    pending_update: Arc<Mutex<bool>>,
    /// Watcher guard to keep the watcher alive.
    _watcher_guard: Option<Box<dyn Any>>,
    /// Animation timeline controller for entry and data transitions.
    animator: ChartAnimator,
    /// Animation configuration for entry and update transitions.
    animation: AnimationConfig,
    /// Monotonic clock origin for animation timestamps.
    clock_origin: Instant,
    /// Whether initial setup is complete.
    setup_complete: bool,
}

impl<R, S> SignalRenderer<R, S>
where
    R: ChartRenderer,
    S: Signal<Output = R::Data> + Clone + 'static,
{
    /// Creates a new reactive chart renderer.
    ///
    /// The signal will be watched for changes, and the renderer will be
    /// updated automatically during each render frame.
    pub fn new(inner: R, signal: S) -> Self {
        let pending_update = Arc::new(Mutex::new(false));
        let pending_clone = Arc::clone(&pending_update);

        // Set up watcher to detect data changes
        let guard = signal.clone().watch(move |_context| {
            if let Ok(mut pending) = pending_clone.lock() {
                *pending = true;
            }
        });

        Self {
            inner,
            signal,
            pending_update,
            _watcher_guard: Some(Box::new(guard)),
            animator: ChartAnimator::new(),
            animation: AnimationConfig::default(),
            clock_origin: Instant::now(),
            setup_complete: false,
        }
    }

    /// Returns a reference to the inner renderer.
    pub fn inner(&self) -> &R {
        &self.inner
    }

    /// Returns a mutable reference to the inner renderer.
    pub fn inner_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// Overrides transition animation configuration.
    #[must_use]
    pub fn animation_config(mut self, animation: AnimationConfig) -> Self {
        self.animation = animation;
        self
    }
}

impl<R, S> GpuView for SignalRenderer<R, S>
where
    R: ChartRenderer,
    S: Signal<Output = R::Data> + Clone + 'static,
{
    fn setup(&mut self, ctx: &GpuContext, env: &mut waterui_core::Environment) -> impl Future<Output = ()> {
        self.clock_origin = Instant::now();

        // Seed initial data before setup so buffers are sized correctly.
        let initial = self.signal.get();
        self.inner.update_data(&initial, ctx.device, ctx.queue);

        // Start entry animation on first appearance when there's visible data.
        if self.inner.data_count() > 0 {
            self.animator.start_entry(
                core::time::Duration::ZERO,
                self.animation.duration,
                self.animation.easing,
            );
            let animation = self.animator.current();
            self.inner.set_animation(&animation);
        } else {
            self.animator.finish();
            let animation = self.animator.current();
            self.inner.set_animation(&animation);
        }

        self.setup_complete = true;

        self.inner.setup(ctx, env)
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        // Check for pending data updates
        let needs_update = self.pending_update.lock().ok().map_or(false, |mut guard| {
            let pending = *guard;
            *guard = false;
            pending
        });

        if needs_update {
            // Read current value from signal and update renderer
            let current_data = self.signal.get();
            self.inner
                .update_data(&current_data, frame.device, frame.queue);
            if self.inner.data_count() > 0 {
                self.animator.start_transition(
                    self.clock_origin.elapsed(),
                    self.animation.duration,
                    self.animation.easing,
                );
            } else {
                self.animator.finish();
            }
        }

        let animation = self.animator.update(self.clock_origin.elapsed());
        self.inner.set_animation(&animation);

        // Render the chart
        self.inner.render(frame);

        // Request continuous redraw while animation is active or updates are pending
        let pending = self.pending_update.lock().ok().is_some_and(|guard| *guard);
        if pending || self.animator.is_animating() || ChartRenderer::needs_redraw(&self.inner) {
            frame.request_redraw();
        }
    }
}

impl<R, S> core::fmt::Debug for SignalRenderer<R, S>
where
    R: ChartRenderer + core::fmt::Debug,
    S: Signal<Output = R::Data> + 'static,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SignalRenderer")
            .field("inner", &self.inner)
            .field("setup_complete", &self.setup_complete)
            .finish_non_exhaustive()
    }
}
