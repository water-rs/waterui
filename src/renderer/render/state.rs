use super::*;
use std::sync::Arc;
use waterui_graphics::DeviceLoss;

/// Shared mutable state carried by the hydrolysis dispatcher.
pub struct HydroState {
    /// Thread-safe text shaping/measurement, shared with worker-thread layout
    /// measurement via `Arc`. See [`TextMeasureService`].
    pub(crate) text: Arc<TextMeasureService>,
    pub(crate) measurement: MeasurementCaches,
    /// Per-frame Cherenkov-migration counters; see
    /// [`crate::renderer::MigrationCounters`].
    pub(crate) counters: MigrationCounters,
    pub(crate) frame_adapter: Option<wgpu::Adapter>,
    pub(crate) frame_device: Option<wgpu::Device>,
    pub(crate) frame_queue: Option<wgpu::Queue>,
    /// Reports this frame's device lost; taken when the device was opened.
    pub(crate) frame_device_loss: Option<DeviceLoss>,
}

impl Default for HydroState {
    fn default() -> Self {
        Self {
            text: Arc::new(TextMeasureService::new()),
            measurement: MeasurementCaches::default(),
            counters: MigrationCounters::default(),
            frame_adapter: None,
            frame_device: None,
            frame_queue: None,
            frame_device_loss: None,
        }
    }
}

impl HydroState {
    /// Mutable access to the registered fonts for startup font registration.
    ///
    /// Requires that no worker has cloned the [`TextMeasureService`] yet, which
    /// holds during single-threaded setup before the first render/measure.
    pub(crate) fn text_fonts_mut(&mut self) -> &mut parley::FontContext {
        Arc::get_mut(&mut self.text)
            .expect(
                "hydrolysis font registration requires unique TextMeasureService ownership \
                 before rendering",
            )
            .fonts_mut()
    }

    pub(crate) fn set_frame_resources(
        &mut self,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        device_loss: &DeviceLoss,
    ) {
        self.frame_adapter = Some(adapter.clone());
        self.frame_device = Some(device.clone());
        self.frame_queue = Some(queue.clone());
        self.frame_device_loss = Some(device_loss.clone());
    }

    pub(crate) fn clear_frame_resources(&mut self) {
        self.frame_adapter = None;
        self.frame_device = None;
        self.frame_queue = None;
        self.frame_device_loss = None;
    }

    pub(crate) fn frame_resources(&self) -> (&wgpu::Device, &wgpu::Queue) {
        let device = self.frame_device.as_ref().unwrap_or_else(|| {
            panic!("hydrolysis frame device is unavailable during AppliedFilter dispatch")
        });
        let queue = self.frame_queue.as_ref().unwrap_or_else(|| {
            panic!("hydrolysis frame queue is unavailable during AppliedFilter dispatch")
        });
        (device, queue)
    }

    pub(crate) fn frame_adapter(&self) -> &wgpu::Adapter {
        self.frame_adapter.as_ref().unwrap_or_else(|| {
            panic!("hydrolysis frame adapter is unavailable during GPU subtree capture")
        })
    }

    pub(crate) fn frame_device_loss(&self) -> &DeviceLoss {
        self.frame_device_loss.as_ref().unwrap_or_else(|| {
            panic!("hydrolysis frame device loss handle is unavailable during GPU subtree capture")
        })
    }
}

impl core::fmt::Debug for HydroState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydroState").finish_non_exhaustive()
    }
}
