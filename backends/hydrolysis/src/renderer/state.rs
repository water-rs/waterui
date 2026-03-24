use super::*;

/// Shared mutable state carried by the hydrolysis dispatcher.
pub struct HydroState {
    pub font_cx: parley::FontContext,
    pub layout_cx: parley::LayoutContext,
    pub(crate) dynamic_intrinsic_cache: BTreeMap<usize, ViewDimensions>,
    pub(super) frame_device: Option<wgpu::Device>,
    pub(super) frame_queue: Option<wgpu::Queue>,
}

impl Default for HydroState {
    fn default() -> Self {
        Self {
            font_cx: parley::FontContext::new(),
            layout_cx: parley::LayoutContext::new(),
            dynamic_intrinsic_cache: BTreeMap::new(),
            frame_device: None,
            frame_queue: None,
        }
    }
}

impl HydroState {
    pub(super) fn set_frame_resources(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.frame_device = Some(device.clone());
        self.frame_queue = Some(queue.clone());
    }

    pub(super) fn clear_frame_resources(&mut self) {
        self.frame_device = None;
        self.frame_queue = None;
    }

    pub(super) fn frame_resources(&self) -> (&wgpu::Device, &wgpu::Queue) {
        let device = self.frame_device.as_ref().unwrap_or_else(|| {
            panic!("hydrolysis frame device is unavailable during AppliedFilter dispatch")
        });
        let queue = self.frame_queue.as_ref().unwrap_or_else(|| {
            panic!("hydrolysis frame queue is unavailable during AppliedFilter dispatch")
        });
        (device, queue)
    }
}

impl core::fmt::Debug for HydroState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydroState").finish_non_exhaustive()
    }
}
