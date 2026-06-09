use super::*;
use rustc_hash::FxHashMap;

/// Shared mutable state carried by the hydrolysis dispatcher.
pub struct HydroState {
    pub font_cx: parley::FontContext,
    pub layout_cx: parley::LayoutContext,
    pub(crate) text_layout_cache: BTreeMap<TextLayoutCacheKey, parley::Layout<[u8; 4]>>,
    pub(crate) measurement: MeasurementCaches,
    pub(crate) frame_device: Option<wgpu::Device>,
    pub(crate) frame_queue: Option<wgpu::Queue>,
}

impl Default for HydroState {
    fn default() -> Self {
        Self {
            font_cx: parley::FontContext::new(),
            layout_cx: parley::LayoutContext::new(),
            text_layout_cache: BTreeMap::new(),
            measurement: MeasurementCaches::default(),
            frame_device: None,
            frame_queue: None,
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct TextLayoutCacheKey {
    pub(crate) text: String,
    pub(crate) spans: Vec<TextLayoutSpanCacheKey>,
    pub(crate) default_font: TextLayoutFontCacheKey,
    pub(crate) default_brush: [u8; 4],
    pub(crate) locale: String,
    pub(crate) alignment_low: u64,
    pub(crate) alignment_high: u64,
    pub(crate) max_width: Option<u32>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct TextLayoutSpanCacheKey {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) font: TextLayoutFontCacheKey,
    pub(crate) foreground: Option<[u8; 4]>,
    pub(crate) italic: bool,
    pub(crate) underline: bool,
    pub(crate) strikethrough: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct TextLayoutFontCacheKey {
    pub(crate) size: u32,
    pub(crate) weight: u16,
    pub(crate) family: Option<String>,
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
