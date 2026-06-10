//! Scroll offset/viewport math and per-identity scroll handle registry.
//!
//! [`ScrollController`] owns one slot per scroll view in body order, rebound
//! on every structural rebuild via cursor reuse. Each [`bind`] hands out a
//! [`ScrollHandle`] stamped with a generation; when a rebuild changes the
//! scroll view's layout (axis, viewport, or content extent), the generation
//! advances and input routed through stale handles from earlier frames is
//! silently dropped. All lengths and offsets are f64 logical pixels.
//!
//! [`bind`]: ScrollController::bind

use std::cell::RefCell;
use std::rc::Rc;

use waterui_layout::scroll::Axis;

const SCROLL_EPSILON: f64 = 0.000_01;
const SCROLL_LINE_STEP: f64 = 40.0;

/// Registry of scroll-view state slots, keyed by body order across rebuilds.
///
/// The renderer brackets every structural rebuild with
/// [`begin_rebuild_frame`](Self::begin_rebuild_frame) /
/// [`finish_rebuild_frame`](Self::finish_rebuild_frame) and calls
/// [`bind`](Self::bind) once per scroll view encountered during dispatch, so
/// the n-th scroll view of consecutive frames shares the same persistent
/// offset state.
#[derive(Debug, Default)]
pub struct ScrollController {
    slots: Vec<ScrollSlot>,
    cursor: usize,
}

#[derive(Debug)]
struct ScrollSlot {
    state: Rc<RefCell<ScrollState>>,
}

/// Cloneable reference to one scroll view's offset state, valid for the
/// layout generation it was bound against.
///
/// Input closures (wheel/trackpad handlers) capture a handle at dispatch
/// time; if the scroll view's layout changed in a later rebuild, the stale
/// handle's generation no longer matches and its input is dropped.
#[derive(Clone, Debug)]
pub struct ScrollHandle {
    state: Rc<RefCell<ScrollState>>,
    generation: u64,
}

/// Snapshot of one scroll view's offsets and extents, in f64 logical pixels.
#[derive(Debug, Clone, Copy)]
pub struct ScrollMetrics {
    /// Current horizontal offset, clamped to `0.0..=max_x`.
    pub offset_x: f64,
    /// Current vertical offset, clamped to `0.0..=max_y`.
    pub offset_y: f64,
    /// Maximum horizontal offset: `(content_width - viewport_width).max(0.0)`.
    pub max_x: f64,
    /// Maximum vertical offset: `(content_height - viewport_height).max(0.0)`.
    pub max_y: f64,
    /// Width of the visible viewport.
    pub viewport_width: f64,
    /// Height of the visible viewport.
    pub viewport_height: f64,
    /// Total width of the scrollable content.
    pub content_width: f64,
    /// Total height of the scrollable content.
    pub content_height: f64,
}

#[derive(Debug)]
struct ScrollState {
    generation: u64,
    axis: Axis,
    viewport_width: f64,
    viewport_height: f64,
    content_width: f64,
    content_height: f64,
    offset_x: f64,
    offset_y: f64,
}

impl ScrollController {
    /// Resets the slot cursor; called at the begin of a structural rebuild,
    /// before any scroll view is dispatched.
    pub fn begin_rebuild_frame(&mut self) {
        self.cursor = 0;
    }

    /// Drops slots not rebound during the rebuild that just finished, so
    /// scroll views removed from the tree release their state.
    pub fn finish_rebuild_frame(&mut self) {
        self.slots.truncate(self.cursor);
    }

    /// Binds the next scroll view in body order to its persistent slot and
    /// returns a handle for routing input to it.
    ///
    /// Reuses the slot at the current cursor (creating it on first
    /// encounter), updates it with the freshly measured layout in logical
    /// pixels, and re-clamps the retained offsets. If axis, viewport, content
    /// extent, or the clamped offsets changed, the slot's generation advances
    /// and handles bound in earlier frames become inert.
    pub fn bind(
        &mut self,
        axis: Axis,
        viewport_width: f64,
        viewport_height: f64,
        content_width: f64,
        content_height: f64,
    ) -> ScrollHandle {
        let index = self.cursor;
        self.cursor = self
            .cursor
            .checked_add(1)
            .expect("scroll controller cursor overflow");

        if index == self.slots.len() {
            self.slots.push(ScrollSlot {
                state: Rc::new(RefCell::new(ScrollState::new(
                    axis,
                    viewport_width,
                    viewport_height,
                    content_width,
                    content_height,
                ))),
            });
        }

        let state = Rc::clone(&self.slots[index].state);
        let generation = state.borrow_mut().prepare_generation(
            axis,
            viewport_width,
            viewport_height,
            content_width,
            content_height,
        );
        ScrollHandle { state, generation }
    }
}

impl ScrollHandle {
    /// Returns a key identifying the underlying scroll slot, stable for the
    /// slot's lifetime across rebuilds (the address of the shared state).
    pub fn cache_key(&self) -> usize {
        Rc::as_ptr(&self.state) as usize
    }

    /// Returns the current offsets and extents of the bound scroll view.
    pub fn metrics(&self) -> ScrollMetrics {
        let state = self.state.borrow();
        state.metrics()
    }

    /// Applies a wheel/trackpad delta along the scroll view's axis and
    /// returns whether the clamped offset actually moved.
    ///
    /// Positive deltas scroll content toward its start (offsets decrease, the
    /// platform wheel convention). With `is_line_delta` the values are line
    /// counts scaled by 40 logical pixels per line; otherwise they are
    /// logical pixels. Input from a handle whose generation is stale is
    /// dropped and returns `false`.
    pub fn apply_scroll_delta(&self, dx: f32, dy: f32, is_line_delta: bool) -> bool {
        let mut state = self.state.borrow_mut();
        if state.generation != self.generation {
            return false;
        }
        state.apply_scroll_delta(f64::from(dx), f64::from(dy), is_line_delta)
    }
}

impl ScrollState {
    fn new(
        axis: Axis,
        viewport_width: f64,
        viewport_height: f64,
        content_width: f64,
        content_height: f64,
    ) -> Self {
        let mut state = Self {
            generation: 1,
            axis,
            viewport_width,
            viewport_height,
            content_width,
            content_height,
            offset_x: 0.0,
            offset_y: 0.0,
        };
        state.clamp_offsets();
        state
    }

    fn prepare_generation(
        &mut self,
        axis: Axis,
        viewport_width: f64,
        viewport_height: f64,
        content_width: f64,
        content_height: f64,
    ) -> u64 {
        let layout_changed = self.axis != axis
            || value_changed(self.viewport_width, viewport_width)
            || value_changed(self.viewport_height, viewport_height)
            || value_changed(self.content_width, content_width)
            || value_changed(self.content_height, content_height);
        let old_offset_x = self.offset_x;
        let old_offset_y = self.offset_y;
        self.axis = axis;
        self.viewport_width = viewport_width;
        self.viewport_height = viewport_height;
        self.content_width = content_width;
        self.content_height = content_height;
        self.clamp_offsets();
        let offset_changed = value_changed(old_offset_x, self.offset_x)
            || value_changed(old_offset_y, self.offset_y);
        if layout_changed || offset_changed {
            self.generation = self
                .generation
                .checked_add(1)
                .expect("scroll controller generation overflow");
        }
        self.generation
    }

    fn apply_scroll_delta(&mut self, dx: f64, dy: f64, is_line_delta: bool) -> bool {
        let metrics = self.metrics();
        let old_x = self.offset_x;
        let old_y = self.offset_y;
        let delta_scale = if is_line_delta { SCROLL_LINE_STEP } else { 1.0 };
        let scaled_dx = dx * delta_scale;
        let scaled_dy = dy * delta_scale;

        match self.axis {
            Axis::Horizontal => {
                self.offset_x = clamp_scroll_offset(old_x - scaled_dx, metrics.max_x);
            }
            Axis::Vertical => {
                self.offset_y = clamp_scroll_offset(old_y - scaled_dy, metrics.max_y);
            }
            Axis::All => {
                self.offset_x = clamp_scroll_offset(old_x - scaled_dx, metrics.max_x);
                self.offset_y = clamp_scroll_offset(old_y - scaled_dy, metrics.max_y);
            }
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        }

        (self.offset_x - old_x).abs() > SCROLL_EPSILON
            || (self.offset_y - old_y).abs() > SCROLL_EPSILON
    }

    fn clamp_offsets(&mut self) {
        let metrics = self.metrics();
        self.offset_x = clamp_scroll_offset(self.offset_x, metrics.max_x);
        self.offset_y = clamp_scroll_offset(self.offset_y, metrics.max_y);
    }

    fn metrics(&self) -> ScrollMetrics {
        let max_x = (self.content_width - self.viewport_width).max(0.0);
        let max_y = (self.content_height - self.viewport_height).max(0.0);
        ScrollMetrics {
            offset_x: self.offset_x,
            offset_y: self.offset_y,
            max_x,
            max_y,
            viewport_width: self.viewport_width,
            viewport_height: self.viewport_height,
            content_width: self.content_width,
            content_height: self.content_height,
        }
    }
}

fn clamp_scroll_offset(value: f64, max: f64) -> f64 {
    value.clamp(0.0, max)
}

fn value_changed(old: f64, new: f64) -> bool {
    (old - new).abs() > SCROLL_EPSILON
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertical_controller() -> (ScrollController, ScrollHandle) {
        let mut controller = ScrollController::default();
        controller.begin_rebuild_frame();
        let handle = controller.bind(Axis::Vertical, 100.0, 100.0, 100.0, 300.0);
        (controller, handle)
    }

    #[test]
    fn scroll_offsets_clamp_to_content_extent() {
        let (_controller, handle) = vertical_controller();
        assert!(handle.apply_scroll_delta(0.0, -50.0, false));
        assert_eq!(handle.metrics().offset_y, 50.0);

        // Overscroll clamps to max_y = content - viewport = 200.
        assert!(handle.apply_scroll_delta(0.0, -10_000.0, false));
        assert_eq!(handle.metrics().offset_y, 200.0);

        // Clamped at the end: a further push changes nothing.
        assert!(!handle.apply_scroll_delta(0.0, -1.0, false));

        // Scrolling back past the top clamps to zero.
        assert!(handle.apply_scroll_delta(0.0, 10_000.0, false));
        assert_eq!(handle.metrics().offset_y, 0.0);
    }

    #[test]
    fn vertical_axis_ignores_horizontal_delta() {
        let (_controller, handle) = vertical_controller();
        assert!(!handle.apply_scroll_delta(-30.0, 0.0, false));
        assert_eq!(handle.metrics().offset_x, 0.0);
    }

    #[test]
    fn line_deltas_scale_to_pixels() {
        let (_controller, handle) = vertical_controller();
        assert!(handle.apply_scroll_delta(0.0, -2.0, true));
        assert_eq!(handle.metrics().offset_y, 80.0);
    }

    #[test]
    fn rebinding_with_same_layout_keeps_offset_and_handle_validity() {
        let (mut controller, handle) = vertical_controller();
        assert!(handle.apply_scroll_delta(0.0, -50.0, false));
        controller.finish_rebuild_frame();

        controller.begin_rebuild_frame();
        let rebound = controller.bind(Axis::Vertical, 100.0, 100.0, 100.0, 300.0);
        assert_eq!(rebound.metrics().offset_y, 50.0);
        // The previous handle still targets the same generation.
        assert!(handle.apply_scroll_delta(0.0, -10.0, false));
    }

    #[test]
    fn layout_change_invalidates_stale_handles() {
        let (mut controller, handle) = vertical_controller();
        controller.finish_rebuild_frame();

        controller.begin_rebuild_frame();
        let rebound = controller.bind(Axis::Vertical, 100.0, 100.0, 100.0, 500.0);
        // The stale handle's generation no longer matches: input is dropped.
        assert!(!handle.apply_scroll_delta(0.0, -10.0, false));
        assert!(
            rebound.apply_scroll_delta(0.0, -10.0, false) || rebound.metrics().offset_y == 10.0
        );
        assert_eq!(rebound.metrics().offset_y, 10.0);
    }

    #[test]
    fn viewport_growth_reclamps_existing_offset() {
        let (mut controller, handle) = vertical_controller();
        assert!(handle.apply_scroll_delta(0.0, -10_000.0, false));
        assert_eq!(handle.metrics().offset_y, 200.0);
        controller.finish_rebuild_frame();

        // The viewport now shows the whole content: the offset clamps home.
        controller.begin_rebuild_frame();
        let rebound = controller.bind(Axis::Vertical, 100.0, 300.0, 100.0, 300.0);
        assert_eq!(rebound.metrics().offset_y, 0.0);
        assert_eq!(rebound.metrics().max_y, 0.0);
    }
}
