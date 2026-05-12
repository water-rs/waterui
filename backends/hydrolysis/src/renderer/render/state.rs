use super::*;

/// Shared mutable state carried by the hydrolysis dispatcher.
pub struct HydroState {
    pub font_cx: parley::FontContext,
    pub layout_cx: parley::LayoutContext,
    pub(crate) dynamic_intrinsic_cache: BTreeMap<usize, ViewDimensions>,
    pub(crate) dynamic_dimensions_cache:
        BTreeMap<(usize, Option<u32>, Option<u32>), ViewDimensions>,
    pub(crate) dynamic_measurement_stack: Vec<(usize, ProposalSize)>,
    pub(crate) measurement_cache: Vec<(usize, usize, ProposalSize, ViewDimensions)>,
    pub(crate) frame_device: Option<wgpu::Device>,
    pub(crate) frame_queue: Option<wgpu::Queue>,
}

impl Default for HydroState {
    fn default() -> Self {
        Self {
            font_cx: parley::FontContext::new(),
            layout_cx: parley::LayoutContext::new(),
            dynamic_intrinsic_cache: BTreeMap::new(),
            dynamic_dimensions_cache: BTreeMap::new(),
            dynamic_measurement_stack: Vec::new(),
            measurement_cache: Vec::new(),
            frame_device: None,
            frame_queue: None,
        }
    }
}

impl HydroState {
    pub(crate) fn set_frame_resources(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.frame_device = Some(device.clone());
        self.frame_queue = Some(queue.clone());
    }

    pub(crate) fn clear_frame_resources(&mut self) {
        self.frame_device = None;
        self.frame_queue = None;
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
}

impl core::fmt::Debug for HydroState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydroState").finish_non_exhaustive()
    }
}
