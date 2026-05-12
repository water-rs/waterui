//! Hit-testing and interaction types for charts.

use core::ops::RangeInclusive;

use nami::{Binding, Computed, SignalExt as _};
use waterui_core::{Environment, IntoSignalF32, layout::Point};

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

    /// Returns this anchor as a `WaterUI` layout point.
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
    /// The bid side of the book.
    Bid,
    /// The ask side of the book.
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
    /// Creates a new area-chart interaction payload.
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
    /// Creates a new depth-chart interaction payload.
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
    /// Creates a new slice interaction payload for pie and gauge charts.
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
    /// Creates a new radar-chart interaction payload.
    #[must_use]
    pub const fn new(axis: usize, label: Option<String>, value: f32) -> Self {
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
    /// Creates a new grid interaction payload for heatmap-like charts.
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
    /// Creates a new choropleth-region interaction payload.
    #[must_use]
    pub const fn new(index: usize, id: u32, value: f32) -> Self {
        Self { index, id, value }
    }
}

/// Viewport for coordinate transformation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
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
    #[allow(
        clippy::missing_const_for_fn,
        reason = "f32::mul_add is not const-stable; keep fused arithmetic semantics"
    )]
    pub fn normalized_to_screen(&self, x: f32, y: f32) -> Point {
        Point::new(
            x.mul_add(self.width, self.x),
            y.mul_add(self.height, self.y),
        )
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
        let data_x = norm_x.mul_add(bounds.width(), bounds.min_x);
        let data_y = (1.0 - norm_y).mul_add(bounds.height(), bounds.min_y);
        Some((data_x, data_y))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
/// Scroll axes enabled for interactive chart viewport movement.
pub struct ChartScrollableAxes {
    horizontal: bool,
    vertical: bool,
}

impl ChartScrollableAxes {
    /// Creates a scroll-axis set.
    #[must_use]
    pub const fn new(horizontal: bool, vertical: bool) -> Self {
        Self {
            horizontal,
            vertical,
        }
    }

    /// No interactive chart scrolling.
    #[must_use]
    pub const fn none() -> Self {
        Self::new(false, false)
    }

    /// Horizontal interactive chart scrolling.
    #[must_use]
    pub const fn horizontal() -> Self {
        Self::new(true, false)
    }

    /// Vertical interactive chart scrolling.
    #[must_use]
    pub const fn vertical() -> Self {
        Self::new(false, true)
    }

    /// Horizontal and vertical interactive chart scrolling.
    #[must_use]
    pub const fn both() -> Self {
        Self::new(true, true)
    }

    /// Returns whether horizontal dragging should move the chart viewport.
    #[must_use]
    pub const fn allows_horizontal(self) -> bool {
        self.horizontal
    }

    /// Returns whether vertical dragging should move the chart viewport.
    #[must_use]
    pub const fn allows_vertical(self) -> bool {
        self.vertical
    }

    /// Returns true when interactive chart scrolling is disabled on both axes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        !self.horizontal && !self.vertical
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CartesianViewportState {
    x_visible_length: Option<f32>,
    y_visible_length: Option<f32>,
    x_position: f32,
    y_position: f32,
}

#[derive(Debug, Clone, Copy, Default)]
struct AxisViewportBindingFlags {
    visible_length: bool,
    external_position: bool,
}

#[derive(Clone)]
pub(crate) struct CartesianViewportBindings {
    scrollable_axes: ChartScrollableAxes,
    x_visible_length: Computed<Option<f32>>,
    y_visible_length: Computed<Option<f32>>,
    x_position: Binding<f32>,
    y_position: Binding<f32>,
    x_flags: AxisViewportBindingFlags,
    y_flags: AxisViewportBindingFlags,
}

impl Default for CartesianViewportBindings {
    fn default() -> Self {
        Self {
            scrollable_axes: ChartScrollableAxes::none(),
            x_visible_length: Computed::constant(None),
            y_visible_length: Computed::constant(None),
            x_position: Binding::f32(0.0),
            y_position: Binding::f32(0.0),
            x_flags: AxisViewportBindingFlags::default(),
            y_flags: AxisViewportBindingFlags::default(),
        }
    }
}

impl CartesianViewportBindings {
    pub fn validate(&self) {
        if self.x_flags.visible_length {
            assert!(
                self.x_flags.external_position,
                "chart_x_visible_domain requires chart_x_scroll_position"
            );
        }
        if self.y_flags.visible_length {
            assert!(
                self.y_flags.external_position,
                "chart_y_visible_domain requires chart_y_scroll_position"
            );
        }
        if self.x_flags.external_position {
            assert!(
                self.x_flags.visible_length,
                "chart_x_scroll_position requires chart_x_visible_domain"
            );
        }
        if self.y_flags.external_position {
            assert!(
                self.y_flags.visible_length,
                "chart_y_scroll_position requires chart_y_visible_domain"
            );
        }
        if self.scrollable_axes.allows_horizontal() {
            assert!(
                self.x_flags.external_position,
                "chart_scrollable_axes(horizontal) requires chart_x_scroll_position"
            );
        }
        if self.scrollable_axes.allows_vertical() {
            assert!(
                self.y_flags.external_position,
                "chart_scrollable_axes(vertical) requires chart_y_scroll_position"
            );
        }
    }

    #[must_use]
    pub const fn with_scrollable_axes(mut self, axes: ChartScrollableAxes) -> Self {
        self.scrollable_axes = axes;
        self
    }

    #[must_use]
    pub fn with_x_visible_length(mut self, length: impl IntoSignalF32) -> Self {
        self.x_visible_length = length.into_signal_f32().map(Some).computed();
        self.x_flags.visible_length = true;
        self
    }

    #[must_use]
    pub fn with_y_visible_length(mut self, length: impl IntoSignalF32) -> Self {
        self.y_visible_length = length.into_signal_f32().map(Some).computed();
        self.y_flags.visible_length = true;
        self
    }

    #[must_use]
    pub fn with_x_position(mut self, binding: &Binding<f32>) -> Self {
        self.x_position = binding.clone();
        self.x_flags.external_position = true;
        self
    }

    #[must_use]
    pub fn with_y_position(mut self, binding: &Binding<f32>) -> Self {
        self.y_position = binding.clone();
        self.y_flags.external_position = true;
        self
    }

    #[must_use]
    pub const fn allows_horizontal_drag(&self) -> bool {
        self.scrollable_axes.allows_horizontal()
    }

    #[must_use]
    pub const fn allows_vertical_drag(&self) -> bool {
        self.scrollable_axes.allows_vertical()
    }

    #[must_use]
    pub fn state_signal(&self) -> Computed<CartesianViewportState> {
        self.x_visible_length
            .zip(&self.y_visible_length)
            .zip(&self.x_position)
            .zip(&self.y_position)
            .map(
                |(((x_visible_length, y_visible_length), x_position), y_position)| {
                    CartesianViewportState {
                        x_visible_length,
                        y_visible_length,
                        x_position,
                        y_position,
                    }
                },
            )
            .computed()
    }

    #[must_use]
    pub fn resolve_bounds(
        &self,
        base_bounds: crate::data::DataBounds,
        state: CartesianViewportState,
    ) -> crate::data::DataBounds {
        let (min_x, max_x) = resolve_axis_window(
            state.x_visible_length,
            state.x_position,
            &self.x_position,
            base_bounds.min_x,
            base_bounds.max_x,
            "chart_x_visible_domain(length) requires finite length > 0",
        );
        let (min_y, max_y) = resolve_axis_window(
            state.y_visible_length,
            state.y_position,
            &self.y_position,
            base_bounds.min_y,
            base_bounds.max_y,
            "chart_y_visible_domain(length) requires finite length > 0",
        );
        crate::data::DataBounds::new(min_x, max_x, min_y, max_y)
    }

    pub fn set_x_position(&self, value: f32) {
        if f32_binding_needs_update(&self.x_position, value) {
            self.x_position.set(value);
        }
    }

    pub fn set_y_position(&self, value: f32) {
        if f32_binding_needs_update(&self.y_position, value) {
            self.y_position.set(value);
        }
    }
}

fn f32_binding_needs_update(binding: &Binding<f32>, value: f32) -> bool {
    binding.get().to_bits() != value.to_bits()
}

fn resolve_axis_window(
    requested_length: Option<f32>,
    requested_position: f32,
    binding: &Binding<f32>,
    base_min: f32,
    base_max: f32,
    invalid_length_message: &'static str,
) -> (f32, f32) {
    let total_length = base_max - base_min;
    if total_length <= 0.0 {
        if f32_binding_needs_update(binding, base_min) {
            binding.set(base_min);
        }
        return (base_min, base_max);
    }

    let Some(raw_length) = requested_length else {
        if f32_binding_needs_update(binding, base_min) {
            binding.set(base_min);
        }
        return (base_min, base_max);
    };
    assert!(
        raw_length.is_finite() && raw_length > 0.0,
        "{invalid_length_message}"
    );

    let visible_length = raw_length.min(total_length);
    let max_start = base_max - visible_length;
    let start = if max_start <= base_min {
        base_min
    } else {
        requested_position.clamp(base_min, max_start)
    };
    if f32_binding_needs_update(binding, start) {
        binding.set(start);
    }
    (start, start + visible_length)
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CartesianSelectionBindings {
    x_value: Option<Binding<Option<f32>>>,
    x_range: Option<Binding<Option<RangeInclusive<f32>>>>,
    y_value: Option<Binding<Option<f32>>>,
    y_range: Option<Binding<Option<RangeInclusive<f32>>>>,
}

impl CartesianSelectionBindings {
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.x_value.is_some()
            || self.x_range.is_some()
            || self.y_value.is_some()
            || self.y_range.is_some()
    }

    pub fn validate(&self) {
        assert!(
            !(self.x_value.is_some() && self.x_range.is_some()),
            "chart_x_selection and chart_x_selection_range cannot be active on the same chart"
        );
        assert!(
            !(self.y_value.is_some() && self.y_range.is_some()),
            "chart_y_selection and chart_y_selection_range cannot be active on the same chart"
        );
    }

    #[must_use]
    pub fn with_x_value(mut self, binding: &Binding<Option<f32>>) -> Self {
        self.x_value = Some(binding.clone());
        self
    }

    #[must_use]
    pub fn with_x_range(mut self, binding: &Binding<Option<RangeInclusive<f32>>>) -> Self {
        self.x_range = Some(binding.clone());
        self
    }

    #[must_use]
    pub fn with_y_value(mut self, binding: &Binding<Option<f32>>) -> Self {
        self.y_value = Some(binding.clone());
        self
    }

    #[must_use]
    pub fn with_y_range(mut self, binding: &Binding<Option<RangeInclusive<f32>>>) -> Self {
        self.y_range = Some(binding.clone());
        self
    }

    pub fn set_x_value(&self, value: Option<f32>) {
        if let Some(binding) = &self.x_value
            && binding.get() != value
        {
            binding.set(value);
        }
    }

    pub fn set_x_range(&self, value: Option<RangeInclusive<f32>>) {
        if let Some(binding) = &self.x_range
            && binding.get() != value
        {
            binding.set(value);
        }
    }

    pub fn set_y_value(&self, value: Option<f32>) {
        if let Some(binding) = &self.y_value
            && binding.get() != value
        {
            binding.set(value);
        }
    }

    pub fn set_y_range(&self, value: Option<RangeInclusive<f32>>) {
        if let Some(binding) = &self.y_range
            && binding.get() != value
        {
            binding.set(value);
        }
    }

    #[must_use]
    pub const fn tracks_x_value(&self) -> bool {
        self.x_value.is_some()
    }

    #[must_use]
    pub const fn tracks_x_range(&self) -> bool {
        self.x_range.is_some()
    }

    #[must_use]
    pub const fn tracks_y_value(&self) -> bool {
        self.y_value.is_some()
    }

    #[must_use]
    pub const fn tracks_y_range(&self) -> bool {
        self.y_range.is_some()
    }
}

macro_rules! chart_x_selection_methods {
    () => {
        /// Tracks the selected x-domain value for this chart.
        #[must_use]
        pub fn chart_x_selection(mut self, selection: &nami::Binding<Option<f32>>) -> Self {
            self.cartesian_selection = self.cartesian_selection.with_x_value(selection);
            self
        }

        /// Tracks the selected x-domain range for this chart.
        #[must_use]
        pub fn chart_x_selection_range(
            mut self,
            selection: &nami::Binding<Option<core::ops::RangeInclusive<f32>>>,
        ) -> Self {
            self.cartesian_selection = self.cartesian_selection.with_x_range(selection);
            self
        }

        /// Tracks the selected y-domain value for this chart.
        #[must_use]
        pub fn chart_y_selection(mut self, selection: &nami::Binding<Option<f32>>) -> Self {
            self.cartesian_selection = self.cartesian_selection.with_y_value(selection);
            self
        }

        /// Tracks the selected y-domain range for this chart.
        #[must_use]
        pub fn chart_y_selection_range(
            mut self,
            selection: &nami::Binding<Option<core::ops::RangeInclusive<f32>>>,
        ) -> Self {
            self.cartesian_selection = self.cartesian_selection.with_y_range(selection);
            self
        }

        /// Enables interactive scrolling along the specified axes.
        #[must_use]
        pub fn chart_scrollable_axes(
            mut self,
            axes: crate::interaction::ChartScrollableAxes,
        ) -> Self {
            self.cartesian_viewport = self.cartesian_viewport.with_scrollable_axes(axes);
            self
        }

        /// Sets the visible x-domain length for this chart.
        #[must_use]
        pub fn chart_x_visible_domain(mut self, length: impl waterui_core::IntoSignalF32) -> Self {
            self.cartesian_viewport = self.cartesian_viewport.with_x_visible_length(length);
            self
        }

        /// Sets the visible y-domain length for this chart.
        #[must_use]
        pub fn chart_y_visible_domain(mut self, length: impl waterui_core::IntoSignalF32) -> Self {
            self.cartesian_viewport = self.cartesian_viewport.with_y_visible_length(length);
            self
        }

        /// Tracks the current visible x-domain start for this chart.
        #[must_use]
        pub fn chart_x_scroll_position(mut self, position: &nami::Binding<f32>) -> Self {
            self.cartesian_viewport = self.cartesian_viewport.with_x_position(position);
            self
        }

        /// Tracks the current visible y-domain start for this chart.
        #[must_use]
        pub fn chart_y_scroll_position(mut self, position: &nami::Binding<f32>) -> Self {
            self.cartesian_viewport = self.cartesian_viewport.with_y_position(position);
            self
        }
    };
}

pub(crate) use chart_x_selection_methods;

#[derive(Debug, Clone, Copy, Default)]
struct SelectionBindingFlags {
    track: bool,
    external: bool,
}

#[derive(Clone)]
pub(crate) struct SelectionBindings<T: Clone + PartialEq + 'static> {
    focused: Binding<Option<HitResult<T>>>,
    selected: Binding<Option<HitResult<T>>>,
    focused_flags: SelectionBindingFlags,
    selected_flags: SelectionBindingFlags,
}

impl<T: Clone + PartialEq + 'static> Default for SelectionBindings<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone + PartialEq + 'static> SelectionBindings<T> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            focused: Binding::container(None),
            selected: Binding::container(None),
            focused_flags: SelectionBindingFlags::default(),
            selected_flags: SelectionBindingFlags::default(),
        }
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.focused_flags.track || self.selected_flags.track
    }

    #[must_use]
    pub const fn activate_proxy(mut self) -> Self {
        self.focused_flags.track = true;
        self.selected_flags.track = true;
        self
    }

    #[must_use]
    pub fn persist_internal(mut self, env: &Environment) -> Self {
        if !self.focused_flags.external {
            self.focused = crate::local_state::local_binding(env, || None::<HitResult<T>>);
        }
        if !self.selected_flags.external {
            self.selected = crate::local_state::local_binding(env, || None::<HitResult<T>>);
        }
        self
    }

    #[must_use]
    pub fn with_focused(mut self, binding: &Binding<Option<HitResult<T>>>) -> Self {
        self.focused = binding.clone();
        self.focused_flags.track = true;
        self.focused_flags.external = true;
        self
    }

    #[must_use]
    pub fn with_selected(mut self, binding: &Binding<Option<HitResult<T>>>) -> Self {
        self.selected = binding.clone();
        self.selected_flags.track = true;
        self.selected_flags.external = true;
        self
    }

    #[must_use]
    pub fn focused_signal(&self) -> nami::Computed<Option<HitResult<T>>> {
        self.focused.computed()
    }

    #[must_use]
    pub fn selected_signal(&self) -> nami::Computed<Option<HitResult<T>>> {
        self.selected.computed()
    }

    pub fn set_focus(&self, value: Option<HitResult<T>>) {
        if self.focused_flags.track && self.focused.get() != value {
            self.focused.set(value);
        }
    }

    pub fn clear_focus(&self) {
        if self.focused_flags.track && self.focused.get().is_some() {
            self.focused.set(None);
        }
    }

    pub fn set_selected(&self, value: Option<HitResult<T>>) {
        if self.selected_flags.track && self.selected.get() != value {
            self.selected.set(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;
    use alloc::rc::Rc;
    use core::any::{Any, TypeId};
    use core::cell::RefCell;

    use super::*;
    use nami::Signal;
    use waterui_core::{LocalStateScope, LocalStateStore};

    #[derive(Clone)]
    struct Slot {
        type_id: TypeId,
        value: Rc<dyn Any>,
    }

    #[test]
    fn persistent_internal_selection_reuses_local_state_slot() {
        let slots = Rc::new(RefCell::new(BTreeMap::<(u64, usize), Slot>::new()));
        let store = LocalStateStore::new({
            let slots = Rc::clone(&slots);
            move |path, index, type_id, _type_name, init| {
                let key = (path, index);
                let mut slots = slots.borrow_mut();
                if let Some(slot) = slots.get(&key) {
                    assert_eq!(slot.type_id, type_id);
                    return Rc::clone(&slot.value);
                }
                let value = init();
                slots.insert(
                    key,
                    Slot {
                        type_id,
                        value: Rc::clone(&value),
                    },
                );
                value
            }
        });
        let scope = LocalStateScope::root();
        let mut env = Environment::new();
        env.insert(scope.clone());
        env.insert(store);

        let first = SelectionBindings::<i32>::new()
            .activate_proxy()
            .persist_internal(&env);
        first.set_selected(Some(HitResult::new(0, 0, 7, ChartAnchor::new(3.0, 4.0))));

        env.insert(scope.reset());
        let second = SelectionBindings::<i32>::new()
            .activate_proxy()
            .persist_internal(&env);

        assert_eq!(
            second.selected_signal().get(),
            Some(HitResult::new(0, 0, 7, ChartAnchor::new(3.0, 4.0)))
        );
    }
}
