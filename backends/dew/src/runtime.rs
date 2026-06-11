//! The frame pump: connects reactive rebuild requests to banded flushes.
//!
//! One [`DewRuntime`] owns the renderer, painter, scheduler, and a concrete
//! display. Each [`DewRuntime::pump`] call performs at most one frame:
//! when the tree's watched signals requested a rebuild (or nothing was
//! rendered yet), the view tree is re-dispatched, the new display list is
//! diffed against the previous one, and only the changed regions are
//! re-rasterized and flushed. On an SPI-bound panel the diff is what makes
//! reactivity affordable: a text change re-sends a few bands, not a frame.

use kurbo::Rect;
use waterui_core::{AnyView, Environment, View};

use crate::compositor::BandScheduler;
use crate::dispatch::DewRenderer;
use crate::display::{BufferDisplay, DisplayFlush};
use crate::display_list::DisplayList;
use crate::painter::Painter;
use crate::render_frame;

/// Drives a view tree onto a [`DisplayFlush`] target.
pub struct DewRuntime<D: DisplayFlush> {
    renderer: DewRenderer,
    painter: Painter,
    scheduler: BandScheduler,
    display: D,
    env: Environment,
    build_root: Box<dyn Fn() -> AnyView>,
    current: DisplayList,
    rendered_once: bool,
}

impl<D: DisplayFlush + core::fmt::Debug> core::fmt::Debug for DewRuntime<D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DewRuntime")
            .field("renderer", &self.renderer)
            .field("display", &self.display)
            .field("rendered_once", &self.rendered_once)
            .finish_non_exhaustive()
    }
}

impl<D: DisplayFlush> DewRuntime<D> {
    /// Creates a runtime rendering `build_root()` onto `display`, slicing
    /// work into bands at most `band_height` rows tall.
    ///
    /// `build_root` is invoked once per structural rebuild; it must return
    /// an equivalent fresh view tree each time (the standard `WaterUI`
    /// root-builder pattern).
    pub fn new(
        display: D,
        env: Environment,
        band_height: u32,
        build_root: impl Fn() -> AnyView + 'static,
    ) -> Self {
        let (width, height) = display.size();
        Self {
            renderer: DewRenderer::default(),
            painter: Painter::new(),
            scheduler: BandScheduler::new(width, height, band_height),
            display,
            env,
            build_root: Box::new(build_root),
            current: DisplayList::new(),
            rendered_once: false,
        }
    }

    /// Renders one frame if the tree requested a rebuild (or none was
    /// rendered yet); returns the logical dirty rects that were flushed, or
    /// [`None`] when the frame was clean.
    pub fn pump(&mut self) -> Option<Vec<Rect>> {
        let first = !self.rendered_once;
        if !(first || self.renderer.signals().take_rebuild_request()) {
            return None;
        }
        let (width, height) = self.display.size();
        let root = (self.build_root)();
        let list = self
            .renderer
            .render_tree(root, &self.env, f64::from(width), f64::from(height));
        let dirty = if first {
            vec![Rect::new(0.0, 0.0, f64::from(width), f64::from(height))]
        } else {
            diff_dirty(&self.current, &list)
        };
        if !dirty.is_empty() {
            render_frame(
                &mut self.painter,
                &list,
                &self.scheduler,
                &dirty,
                &mut self.display,
            );
        }
        self.current = list;
        self.rendered_once = true;
        Some(dirty)
    }

    /// The display being rendered to.
    pub const fn display(&self) -> &D {
        &self.display
    }

    /// The frame-trigger handle, for callers that need to request rebuilds
    /// outside the watched-signal path (e.g. size changes, benchmarks).
    #[must_use]
    pub fn signals(&self) -> waterui_backend_core::frame_signals::FrameSignals {
        self.renderer.signals()
    }
}

/// Window-coordinate regions where `new` draws differently from `old`.
///
/// Commands are compared pairwise in draw order; a changed pair dirties the
/// union of its old and new bounds, and length differences dirty every
/// unpaired command. This is conservative (never misses a changed pixel)
/// and exact for the common case of an in-place value change.
fn diff_dirty(old: &DisplayList, new: &DisplayList) -> Vec<Rect> {
    let old_commands = old.commands();
    let new_commands = new.commands();
    let common = old_commands.len().min(new_commands.len());
    let mut dirty = Vec::new();
    for (old_command, new_command) in old_commands.iter().zip(new_commands) {
        if old_command != new_command {
            dirty.push(old_command.bounds().union(new_command.bounds()));
        }
    }
    for command in &old_commands[common..] {
        dirty.push(command.bounds());
    }
    for command in &new_commands[common..] {
        dirty.push(command.bounds());
    }
    dirty
}

/// Renders one view tree to a PNG at `width` × `height` — the offscreen
/// simulator entry used by snapshot tests and visual review.
///
/// # Panics
///
/// Panics when the initial frame fails to render or the framebuffer cannot
/// be encoded as PNG.
pub fn render_view_png<V: View>(
    build_root: impl Fn() -> V + 'static,
    env: Environment,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let display = BufferDisplay::new(width, height);
    let mut runtime = DewRuntime::new(display, env, 16, move || AnyView::new(build_root()));
    assert!(runtime.pump().is_some(), "initial pump must render a frame");
    runtime.display().to_png()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_list::DisplayList;
    use kurbo::Affine;
    use peniko::Color;

    #[test]
    fn diff_marks_only_changed_commands() {
        let mut old = DisplayList::new();
        old.fill(
            &Rect::new(0.0, 0.0, 100.0, 100.0),
            Affine::IDENTITY,
            Color::WHITE,
        );
        old.fill(
            &Rect::new(10.0, 10.0, 20.0, 20.0),
            Affine::IDENTITY,
            Color::BLACK,
        );
        let mut new = DisplayList::new();
        new.fill(
            &Rect::new(0.0, 0.0, 100.0, 100.0),
            Affine::IDENTITY,
            Color::WHITE,
        );
        new.fill(
            &Rect::new(10.0, 10.0, 20.0, 20.0),
            Affine::IDENTITY,
            Color::from_rgb8(200, 0, 0),
        );
        assert_eq!(
            diff_dirty(&old, &new),
            vec![Rect::new(10.0, 10.0, 20.0, 20.0)]
        );
        assert!(diff_dirty(&old, &old.clone()).is_empty());
    }

    #[test]
    fn diff_dirties_unpaired_tail_commands() {
        let mut old = DisplayList::new();
        old.fill(
            &Rect::new(0.0, 0.0, 50.0, 50.0),
            Affine::IDENTITY,
            Color::WHITE,
        );
        let mut new = old.clone();
        new.fill(
            &Rect::new(60.0, 60.0, 90.0, 90.0),
            Affine::IDENTITY,
            Color::BLACK,
        );
        assert_eq!(
            diff_dirty(&old, &new),
            vec![Rect::new(60.0, 60.0, 90.0, 90.0)]
        );
    }
}
