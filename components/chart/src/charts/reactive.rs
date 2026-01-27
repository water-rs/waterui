//! Reactive wrapper for chart renderers.
//!
//! Provides automatic data synchronization between Signals and GPU renderers.

extern crate alloc;

use alloc::sync::Arc;
use core::any::Any;
use core::future::Future;

use nami::Signal;
use std::sync::Mutex;
use waterui_graphics::{GpuContext, GpuFrame, GpuRenderer};

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
}

impl<R, S> GpuRenderer for SignalRenderer<R, S>
where
    R: ChartRenderer,
    S: Signal<Output = R::Data> + Clone + 'static,
{
    fn setup(&mut self, ctx: &GpuContext) -> impl Future<Output = ()> {
        // Seed initial data before setup so buffers are sized correctly.
        let initial = self.signal.get();
        self.inner.update_data(&initial, ctx.queue);

        // Set up the inner renderer
        let setup_future = self.inner.setup(ctx);

        // Mark that we need to load initial data (in case it changed)
        if let Ok(mut pending) = self.pending_update.lock() {
            *pending = true;
        }
        self.setup_complete = true;

        setup_future
    }

    fn render(&mut self, frame: &GpuFrame) {
        // Check for pending data updates
        let needs_update = self.pending_update.lock().ok().map_or(false, |mut guard| {
            let pending = *guard;
            *guard = false;
            pending
        });

        if needs_update {
            // Read current value from signal and update renderer
            let current_data = self.signal.get();
            self.inner.update_data(&current_data, frame.queue);
        }

        // Render the chart
        self.inner.render(frame);
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.inner.resize(width, height);
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
