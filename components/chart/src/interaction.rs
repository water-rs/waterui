//! Hit-testing and interaction system for charts.

use waterui_core::{
    gesture::{DragEvent, GesturePhase, MagnificationEvent},
    layout::Point,
};

/// Result of a hit test on chart data.
#[derive(Debug, Clone)]
pub struct HitResult<T> {
    /// Series index (for multi-series charts).
    pub series: usize,
    /// Data point index within the series.
    pub index: usize,
    /// The data value that was hit.
    pub value: T,
    /// Screen position of the hit.
    pub screen_position: Point,
}

impl<T> HitResult<T> {
    /// Creates a new hit result.
    #[must_use]
    pub const fn new(series: usize, index: usize, value: T, screen_position: Point) -> Self {
        Self {
            series,
            index,
            value,
            screen_position,
        }
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
        let norm_y = 1.0 - (data_y - bounds.min_y) / bounds.height(); // Flip Y
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
        let data_y = bounds.min_y + (1.0 - norm_y) * bounds.height(); // Flip Y
        Some((data_x, data_y))
    }
}

/// Selection state for charts.
#[derive(Debug, Clone, Default)]
pub struct SelectionState {
    /// Currently selected series index.
    pub series: Option<usize>,
    /// Currently selected point index.
    pub index: Option<usize>,
    /// Hover position (for crosshair).
    pub hover: Option<Point>,
}

impl SelectionState {
    /// Clears the selection.
    pub fn clear(&mut self) {
        self.series = None;
        self.index = None;
    }

    /// Sets the selection.
    pub fn select(&mut self, series: usize, index: usize) {
        self.series = Some(series);
        self.index = Some(index);
    }

    /// Returns true if anything is selected.
    #[must_use]
    pub fn has_selection(&self) -> bool {
        self.series.is_some() && self.index.is_some()
    }
}

/// Zoom and pan state for interactive chart navigation.
///
/// Tracks cumulative zoom scale and pan offset to transform data bounds.
/// Supports pinch-to-zoom, drag-to-pan, and double-tap-to-reset gestures.
///
/// # Coordinate System
///
/// - Zoom scale > 1.0 means zoomed in (seeing less data)
/// - Pan offset is in normalized coordinates (0.0-1.0 relative to data bounds)
/// - Both zoom and pan are centered around the gesture focal point
#[derive(Debug, Clone, Copy)]
pub struct ZoomPanState {
    /// Cumulative zoom scale (1.0 = no zoom, 2.0 = 2x zoom in).
    pub scale: f32,
    /// Pan offset in normalized data coordinates.
    pub offset: Point,
    /// Whether a gesture is currently active.
    gesture_active: bool,
    /// Scale when gesture started (for delta calculation).
    gesture_start_scale: f32,
    /// Offset when gesture started.
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
    /// Creates a new zoom/pan state with no transformation.
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

    /// Updates zoom/pan state from a drag gesture event.
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

    /// Updates zoom/pan state from a magnification gesture event.
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

    /// Resets zoom/pan in response to a double tap.
    pub fn apply_double_tap(&mut self) {
        self.reset();
    }

    /// Resets zoom and pan to default state.
    pub fn reset(&mut self) {
        self.scale = 1.0;
        self.offset = Point::new(0.0, 0.0);
    }

    /// Transforms data bounds according to current zoom/pan state.
    ///
    /// Returns the visible portion of the data bounds.
    #[must_use]
    pub fn transform_bounds(&self, bounds: &crate::data::DataBounds) -> crate::data::DataBounds {
        let data_width = bounds.width();
        let data_height = bounds.height();

        // Visible width/height is inversely proportional to scale
        let visible_width = data_width / self.scale;
        let visible_height = data_height / self.scale;

        // Center of visible region in data coordinates
        let center_x = bounds.min_x + data_width * (0.5 - self.offset.x);
        let center_y = bounds.min_y + data_height * (0.5 - self.offset.y);

        crate::data::DataBounds {
            min_x: center_x - visible_width / 2.0,
            max_x: center_x + visible_width / 2.0,
            min_y: center_y - visible_height / 2.0,
            max_y: center_y + visible_height / 2.0,
        }
    }

    /// Returns true if there's any zoom or pan applied.
    #[must_use]
    pub fn is_transformed(&self) -> bool {
        (self.scale - 1.0).abs() > 0.001
            || self.offset.x.abs() > 0.001
            || self.offset.y.abs() > 0.001
    }

    /// Clamps offset to keep data visible.
    fn clamp_offset(&mut self) {
        // At higher zoom levels, allow more offset
        let max_offset = (0.5 - 0.5 / self.scale).max(0.0);
        self.offset.x = self.offset.x.clamp(-max_offset, max_offset);
        self.offset.y = self.offset.y.clamp(-max_offset, max_offset);
    }
}
