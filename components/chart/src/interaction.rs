//! Hit-testing and interaction types for charts.

use nami::Binding;
use waterui_core::{
    gesture::{DragEvent, GesturePhase, MagnificationEvent},
    layout::Point,
};

/// Stable screen-space anchor for chart readouts and overlays.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ChartAnchor {
    /// Horizontal coordinate in the chart view's local coordinate space.
    pub x: f32,
    /// Vertical coordinate in the chart view's local coordinate space.
    pub y: f32,
}

impl ChartAnchor {
    /// Creates a new screen-space anchor.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub const fn as_point(self) -> Point {
        Point::new(self.x, self.y)
    }
}

impl From<Point> for ChartAnchor {
    fn from(value: Point) -> Self {
        Self::new(value.x, value.y)
    }
}

/// Result of a chart hit test.
#[derive(Debug, Clone, PartialEq)]
pub struct HitResult<T> {
    /// Series index for multi-series charts.
    pub series: usize,
    /// Data point index within the series.
    pub index: usize,
    /// Strongly-typed chart payload for the hit datum.
    pub value: T,
    /// Screen-space anchor for tooltip/readout composition.
    pub anchor: ChartAnchor,
}

impl<T> HitResult<T> {
    /// Creates a new hit result.
    #[must_use]
    pub fn new(series: usize, index: usize, value: T, anchor: impl Into<ChartAnchor>) -> Self {
        Self {
            series,
            index,
            value,
            anchor: anchor.into(),
        }
    }
}

/// Selected/focused value for depth charts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepthSide {
    Bid,
    Ask,
}

/// Payload for area chart focus/selection.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaDatum {
    /// Series index.
    pub series: usize,
    /// Shared x value.
    pub x: f32,
    /// Sampled y value for the selected series.
    pub y: f32,
}

impl AreaDatum {
    #[must_use]
    pub const fn new(series: usize, x: f32, y: f32) -> Self {
        Self { series, x, y }
    }
}

/// Payload for depth chart focus/selection.
#[derive(Debug, Clone, PartialEq)]
pub struct DepthDatum {
    /// Bid or ask side.
    pub side: DepthSide,
    /// Price level.
    pub price: f32,
    /// Cumulative volume at this level.
    pub cumulative_volume: f32,
}

impl DepthDatum {
    #[must_use]
    pub const fn new(side: DepthSide, price: f32, cumulative_volume: f32) -> Self {
        Self {
            side,
            price,
            cumulative_volume,
        }
    }
}

/// Payload for pie/gauge focus/selection.
#[derive(Debug, Clone, PartialEq)]
pub struct SliceDatum {
    /// Slice or sector index.
    pub index: usize,
    /// Underlying scalar value.
    pub value: f32,
    /// Start angle in radians.
    pub start_angle: f32,
    /// End angle in radians.
    pub end_angle: f32,
}

impl SliceDatum {
    #[must_use]
    pub const fn new(index: usize, value: f32, start_angle: f32, end_angle: f32) -> Self {
        Self {
            index,
            value,
            start_angle,
            end_angle,
        }
    }
}

/// Payload for radar chart focus/selection.
#[derive(Debug, Clone, PartialEq)]
pub struct RadarDatum {
    /// Axis index.
    pub axis: usize,
    /// Optional axis label.
    pub label: Option<String>,
    /// Value at the selected axis.
    pub value: f32,
}

impl RadarDatum {
    #[must_use]
    pub fn new(axis: usize, label: Option<String>, value: f32) -> Self {
        Self { axis, label, value }
    }
}

/// Payload for heatmap/contour focus/selection.
#[derive(Debug, Clone, PartialEq)]
pub struct GridDatum {
    /// Zero-based row index.
    pub row: usize,
    /// Zero-based column index.
    pub column: usize,
    /// Scalar value at the selected cell.
    pub value: f32,
}

impl GridDatum {
    #[must_use]
    pub const fn new(row: usize, column: usize, value: f32) -> Self {
        Self { row, column, value }
    }
}

/// Payload for choropleth focus/selection.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionDatum {
    /// Polygon index within the data set.
    pub index: usize,
    /// Stable region id from the source data.
    pub id: u32,
    /// Data value for the region.
    pub value: f32,
}

impl RegionDatum {
    #[must_use]
    pub const fn new(index: usize, id: u32, value: f32) -> Self {
        Self { index, id, value }
    }
}

/// Viewport for coordinate transformation.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChartViewport {
    /// X position of chart area.
    pub x: f32,
    /// Y position of chart area.
    pub y: f32,
    /// Width of chart area.
    pub width: f32,
    /// Height of chart area.
    pub height: f32,
}

impl ChartViewport {
    /// Creates a new viewport.
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Checks if a point is within the viewport.
    #[must_use]
    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.x
            && point.x <= self.x + self.width
            && point.y >= self.y
            && point.y <= self.y + self.height
    }

    /// Converts screen coordinates to normalized chart coordinates (0.0 to 1.0).
    #[must_use]
    pub fn screen_to_normalized(&self, point: Point) -> Option<(f32, f32)> {
        if !self.contains(point) {
            return None;
        }

        let x = (point.x - self.x) / self.width;
        let y = (point.y - self.y) / self.height;
        Some((x, y))
    }

    /// Converts normalized coordinates to screen coordinates.
    #[must_use]
    pub fn normalized_to_screen(&self, x: f32, y: f32) -> Point {
        Point::new(self.x + x * self.width, self.y + y * self.height)
    }

    /// Converts data coordinates to screen coordinates.
    #[must_use]
    pub fn data_to_screen(
        &self,
        data_x: f32,
        data_y: f32,
        bounds: &crate::data::DataBounds,
    ) -> Point {
        let norm_x = (data_x - bounds.min_x) / bounds.width();
        let norm_y = 1.0 - (data_y - bounds.min_y) / bounds.height();
        self.normalized_to_screen(norm_x, norm_y)
    }

    /// Converts screen coordinates to data coordinates.
    #[must_use]
    pub fn screen_to_data(
        &self,
        point: Point,
        bounds: &crate::data::DataBounds,
    ) -> Option<(f32, f32)> {
        let (norm_x, norm_y) = self.screen_to_normalized(point)?;
        let data_x = bounds.min_x + norm_x * bounds.width();
        let data_y = bounds.min_y + (1.0 - norm_y) * bounds.height();
        Some((data_x, data_y))
    }
}

/// Legacy internal zoom/pan state kept only for backend interaction smoke coverage.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ZoomPanState {
    pub scale: f32,
    pub offset: Point,
    gesture_active: bool,
    gesture_start_scale: f32,
    gesture_start_offset: Point,
}

impl Default for ZoomPanState {
    fn default() -> Self {
        Self {
            scale: 1.0,
            offset: Point::new(0.0, 0.0),
            gesture_active: false,
            gesture_start_scale: 1.0,
            gesture_start_offset: Point::new(0.0, 0.0),
        }
    }
}

impl ZoomPanState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            scale: 1.0,
            offset: Point::new(0.0, 0.0),
            gesture_active: false,
            gesture_start_scale: 1.0,
            gesture_start_offset: Point::new(0.0, 0.0),
        }
    }

    pub fn apply_drag_event(&mut self, event: &DragEvent, viewport: ChartViewport) {
        if viewport.width <= 0.0 || viewport.height <= 0.0 {
            return;
        }

        match event.phase {
            GesturePhase::Started => {
                self.gesture_active = true;
                self.gesture_start_offset = self.offset;
            }
            GesturePhase::Updated => {}
            GesturePhase::Ended | GesturePhase::Cancelled => {
                self.gesture_active = false;
            }
        }

        if !matches!(event.phase, GesturePhase::Started | GesturePhase::Updated) {
            return;
        }

        let pan_x = event.translation.x / viewport.width;
        let pan_y = event.translation.y / viewport.height;
        self.offset.x = self.gesture_start_offset.x + pan_x / self.scale;
        self.offset.y = self.gesture_start_offset.y + pan_y / self.scale;
        self.clamp_offset();
    }

    pub fn apply_magnification_event(
        &mut self,
        event: &MagnificationEvent,
        viewport: ChartViewport,
    ) {
        if viewport.width <= 0.0 || viewport.height <= 0.0 {
            return;
        }

        match event.phase {
            GesturePhase::Started => {
                self.gesture_active = true;
                self.gesture_start_scale = self.scale;
                self.gesture_start_offset = self.offset;
            }
            GesturePhase::Updated => {}
            GesturePhase::Ended | GesturePhase::Cancelled => {
                self.gesture_active = false;
            }
        }

        if !matches!(event.phase, GesturePhase::Started | GesturePhase::Updated) {
            return;
        }

        let new_scale = (self.gesture_start_scale * event.scale).clamp(0.5, 10.0);
        let center = Point::new(event.center.x, event.center.y);
        if let Some((norm_x, norm_y)) = viewport.screen_to_normalized(center) {
            let scale_delta = new_scale / self.scale;
            self.offset.x = norm_x - (norm_x - self.offset.x) * scale_delta;
            self.offset.y = norm_y - (norm_y - self.offset.y) * scale_delta;
        }
        self.scale = new_scale;
        self.clamp_offset();
    }

    pub fn apply_double_tap(&mut self) {
        self.reset();
    }

    pub fn reset(&mut self) {
        self.scale = 1.0;
        self.offset = Point::new(0.0, 0.0);
    }

    #[must_use]
    pub fn transform_bounds(&self, bounds: &crate::data::DataBounds) -> crate::data::DataBounds {
        let data_width = bounds.width();
        let data_height = bounds.height();
        let visible_width = data_width / self.scale;
        let visible_height = data_height / self.scale;
        let center_x = bounds.min_x + data_width * (0.5 - self.offset.x);
        let center_y = bounds.min_y + data_height * (0.5 - self.offset.y);

        crate::data::DataBounds {
            min_x: center_x - visible_width / 2.0,
            max_x: center_x + visible_width / 2.0,
            min_y: center_y - visible_height / 2.0,
            max_y: center_y + visible_height / 2.0,
        }
    }

    #[must_use]
    pub fn is_transformed(&self) -> bool {
        (self.scale - 1.0).abs() > 0.001
            || self.offset.x.abs() > 0.001
            || self.offset.y.abs() > 0.001
    }

    fn clamp_offset(&mut self) {
        let max_offset = (0.5 - 0.5 / self.scale).max(0.0);
        self.offset.x = self.offset.x.clamp(-max_offset, max_offset);
        self.offset.y = self.offset.y.clamp(-max_offset, max_offset);
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SelectionBindings<T: Clone + PartialEq + 'static> {
    focused: Option<Binding<Option<HitResult<T>>>>,
    selected: Option<Binding<Option<HitResult<T>>>>,
}

impl<T: Clone + PartialEq + 'static> Default for SelectionBindings<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone + PartialEq + 'static> SelectionBindings<T> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            focused: None,
            selected: None,
        }
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.focused.is_some() || self.selected.is_some()
    }

    #[must_use]
    pub fn with_focused(mut self, binding: &Binding<Option<HitResult<T>>>) -> Self {
        self.focused = Some(binding.clone());
        self
    }

    #[must_use]
    pub fn with_selected(mut self, binding: &Binding<Option<HitResult<T>>>) -> Self {
        self.selected = Some(binding.clone());
        self
    }

    pub fn set_focus(&self, value: Option<HitResult<T>>) {
        if let Some(binding) = &self.focused
            && binding.get() != value
        {
            binding.set(value);
        }
    }

    pub fn clear_focus(&self) {
        if let Some(binding) = &self.focused
            && binding.get().is_some()
        {
            binding.set(None);
        }
    }

    pub fn set_selected(&self, value: Option<HitResult<T>>) {
        if let Some(binding) = &self.selected
            && binding.get() != value
        {
            binding.set(value);
        }
    }
}
