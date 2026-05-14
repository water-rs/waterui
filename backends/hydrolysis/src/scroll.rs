use std::cell::RefCell;
use std::rc::Rc;

use waterui_layout::scroll::Axis;

const SCROLL_EPSILON: f64 = 0.000_01;
const SCROLL_LINE_STEP: f64 = 40.0;

#[derive(Debug, Default)]
pub struct ScrollController {
    slots: Vec<ScrollSlot>,
    cursor: usize,
}

#[derive(Debug)]
struct ScrollSlot {
    state: Rc<RefCell<ScrollState>>,
}

#[derive(Clone, Debug)]
pub struct ScrollHandle {
    state: Rc<RefCell<ScrollState>>,
    generation: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct ScrollMetrics {
    pub offset_x: f64,
    pub offset_y: f64,
    pub max_x: f64,
    pub max_y: f64,
    pub viewport_width: f64,
    pub viewport_height: f64,
    pub content_width: f64,
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
    pub fn begin_rebuild_frame(&mut self) {
        self.cursor = 0;
    }

    pub fn finish_rebuild_frame(&mut self) {
        self.slots.truncate(self.cursor);
    }

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
    pub(crate) fn cache_key(&self) -> usize {
        Rc::as_ptr(&self.state) as usize
    }

    pub fn metrics(&self) -> ScrollMetrics {
        let state = self.state.borrow();
        state.metrics()
    }

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
