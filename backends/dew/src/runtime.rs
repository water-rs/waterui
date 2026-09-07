//! The frame pump: connects reactive rebuild requests to banded flushes.
//!
//! One [`DewRuntime`] owns the renderer, painter, scheduler, and a [`Board`]
//! (the platform substrate — display, clock, input). Each
//! [`DewRuntime::pump`] call performs at most one frame. The root view is built
//! once; later signal changes refresh the retained node tree, diff its new
//! display list against the previous one, and only re-rasterize changed regions
//! to the board's display. On an SPI-bound panel the diff is what makes
//! reactivity affordable: a text change re-sends a few bands, not a frame.
//!
//! The runtime is generic over the board, so the engine is identical on the
//! host ([`HostBoard`]) and on-chip.

use kurbo::Rect;
#[cfg(feature = "host")]
use waterui_core::View;
use waterui_core::{AnyView, Environment};

use crate::board::Board;
#[cfg(feature = "host")]
use crate::board::HostBoard;
use crate::compositor::BandScheduler;
use crate::dispatch::DewRenderer;
use crate::display::DisplayFlush;
use crate::display_list::DisplayList;
use crate::painter::Painter;
use crate::render_frame;
use crate::stats::FrameWork;

/// One rendered frame: what changed on screen, and what producing it cost.
///
/// [`Frame::work`] is the machine-independent half — identical on host and
/// target for the same view tree and signal values — and is what performance
/// budgets are written against. See [`crate::stats`] for why wall-clock is
/// not.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// Logical-pixel regions that were re-rasterized and flushed.
    pub dirty: Vec<Rect>,
    /// Work performed producing this frame.
    pub work: FrameWork,
}

/// Drives a view tree onto a [`Board`].
pub struct DewRuntime<B: Board> {
    renderer: DewRenderer,
    painter: Painter,
    scheduler: BandScheduler,
    board: B,
    env: Environment,
    build_root: Option<Box<dyn Fn() -> AnyView>>,
    current: DisplayList,
    rendered_once: bool,
}

impl<B: Board + core::fmt::Debug> core::fmt::Debug for DewRuntime<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DewRuntime")
            .field("renderer", &self.renderer)
            .field("board", &self.board)
            .field("rendered_once", &self.rendered_once)
            .finish_non_exhaustive()
    }
}

impl<B: Board> DewRuntime<B> {
    /// Creates a runtime rendering `build_root()` onto `board`, slicing work
    /// into bands at most `band_height` rows tall.
    ///
    /// `build_root` is invoked exactly once, on the first pump.
    ///
    /// `env` needs no theme: [`DewRenderer::render_tree`] installs dew's
    /// built-in type scale for every font slot it does not already carry.
    pub fn new(
        mut board: B,
        env: Environment,
        band_height: u32,
        build_root: impl Fn() -> AnyView + 'static,
    ) -> Self {
        let render_settings = board.render_settings();
        let fonts = board.fonts();
        let signals = waterui_backend_core::frame_signals::FrameSignals::new(board.now());
        let (width, height) = board.display().size();
        let mut renderer = DewRenderer::new(signals, fonts);
        renderer.set_accessibility_enabled(board.supports_accessibility());
        Self {
            renderer,
            painter: Painter::new(render_settings),
            scheduler: BandScheduler::new(width, height, band_height),
            board,
            env,
            build_root: Some(Box::new(build_root)),
            current: DisplayList::new(),
            rendered_once: false,
        }
    }

    /// Renders one frame if the retained tree requested a refresh (or none was
    /// rendered yet); returns what was flushed and what it cost, or [`None`]
    /// when the frame was clean.
    ///
    /// # Panics
    ///
    /// Panics when external code requests a root rebuild after the initial
    /// frame; structural changes must use an explicit local `Dynamic` node.
    pub fn pump(&mut self) -> Option<Frame> {
        let first = !self.rendered_once;
        let signals = self.renderer.signals();
        let mut input_changed = false;
        if !first {
            while let Some(request) = self.board.poll_accessibility_action() {
                input_changed |= self.renderer.handle_accessibility_action(&request);
            }
            // One instant for the whole batch: the board reports positions,
            // not timestamps, and a pump is a single cadence slot — dating the
            // samples apart would be inventing precision the device never had.
            #[cfg(feature = "gestures")]
            let now = self.board.now();
            while let Some(sample) = self.board.poll_pointer() {
                input_changed |= self.renderer.handle_pointer(sample);
                #[cfg(feature = "gestures")]
                {
                    input_changed |= self
                        .renderer
                        .handle_interaction_pointer(sample, now, &self.env);
                }
            }
            // A long press is recognized by time passing rather than by input
            // arriving, so it needs the frame pump to carry the clock to it.
            #[cfg(feature = "gestures")]
            {
                input_changed |= self.renderer.tick_interaction(now, &self.env);
            }
        }
        if input_changed {
            signals.request_refresh();
        }
        let rebuild = signals.take_rebuild_request();
        assert!(
            first || !rebuild,
            "Dew does not rebuild the root view; use Dynamic for an explicit local structural replacement"
        );
        let refresh = signals.take_patch_request() | signals.take_redraw_request();
        if !(first || refresh) {
            return None;
        }
        let (width, height) = self.board.display().size();
        let list = if first {
            let build_root = self
                .build_root
                .take()
                .expect("Dew root builder must exist before the first frame");
            self.renderer
                .render_tree(build_root(), &self.env, f64::from(width), f64::from(height))
        } else {
            self.renderer
                .refresh_tree(f64::from(width), f64::from(height))
        };
        if let Some(accessibility) = self.renderer.take_accessibility_tree_update() {
            self.board.update_accessibility(&accessibility);
        }
        let dirty = if first {
            vec![Rect::new(0.0, 0.0, f64::from(width), f64::from(height))]
        } else {
            diff_dirty(&self.current, &list)
        };
        let mut work = list.work();
        if !dirty.is_empty() {
            render_frame(
                &mut self.painter,
                &list,
                &self.scheduler,
                &dirty,
                self.board.display(),
                &mut work,
            );
        }
        self.current = list;
        self.rendered_once = true;
        Some(Frame { dirty, work })
    }

    /// The board being rendered to.
    pub const fn board(&self) -> &B {
        &self.board
    }

    /// Mutable access to the board, used by host runners to enqueue input.
    pub const fn board_mut(&mut self) -> &mut B {
        &mut self.board
    }

    /// The frame-trigger handle, for callers that need to request a retained
    /// refresh outside the watched-signal path (e.g. size changes).
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
        dirty.extend(old_command.changed_bounds(new_command));
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
#[cfg(feature = "host")]
pub fn render_view_png<V: View>(
    build_root: impl Fn() -> V + 'static,
    env: Environment,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let mut runtime = DewRuntime::new(HostBoard::new(width, height), env, 16, move || {
        AnyView::new(build_root())
    });
    assert!(runtime.pump().is_some(), "initial pump must render a frame");
    runtime.board().framebuffer().to_png()
}

#[cfg(all(test, feature = "host"))]
mod tests {
    use super::*;
    use crate::DrawCommand;
    use crate::display_list::DisplayList;
    use core::cell::Cell;
    use kurbo::Affine;
    use nami::{Binding, binding};
    use peniko::Color;
    use std::rc::Rc;
    use waterui_backend_core::input::TouchPhase;
    use waterui_controls::toggle::Toggle;
    use waterui_text::text;

    struct CountingToggle {
        body_calls: Rc<Cell<usize>>,
        value: Binding<bool>,
    }

    impl View for CountingToggle {
        fn body(self, _env: &Environment) -> impl View {
            self.body_calls.set(self.body_calls.get() + 1);
            Toggle::new("Counting", &self.value).hide_label()
        }
    }

    #[test]
    fn binding_refresh_does_not_rebuild_view_body() {
        let body_calls = Rc::new(Cell::new(0));
        let value = binding(false);
        let mut runtime = DewRuntime::new(HostBoard::new(200, 40), Environment::new(), 16, {
            let body_calls = Rc::clone(&body_calls);
            let value = value.clone();
            move || {
                AnyView::new(CountingToggle {
                    body_calls: Rc::clone(&body_calls),
                    value: value.clone(),
                })
            }
        });

        assert!(runtime.pump().is_some());
        assert_eq!(body_calls.get(), 1);
        assert!(runtime.pump().is_none(), "clean frame must not render");

        value.set(true);
        let frame = runtime
            .pump()
            .expect("binding refresh must render the retained tree");

        assert!(!frame.dirty.is_empty());
        assert_eq!(body_calls.get(), 1, "refresh must not evaluate body again");
    }

    /// Every shaped glyph size in the frame the runtime last flushed.
    fn shaped_font_sizes(list: &DisplayList) -> Vec<u32> {
        list.commands()
            .iter()
            .filter_map(|placed| match placed.command() {
                DrawCommand::GlyphRun { font_size, .. } => Some(font_size.to_bits()),
                _ => None,
            })
            .collect()
    }

    /// A reactive body font must re-shape the retained text: the change has to
    /// request a frame of its own, and the layout cached at the old size must
    /// not be replayed at the new one.
    #[test]
    fn body_font_change_reshapes_retained_text() {
        use waterui::Plugin as _;
        use waterui::theme::{FontSettings, Theme};
        use waterui_text::font::{FontWeight, ResolvedFont};

        let font = binding(ResolvedFont::new(16.0, FontWeight::Normal));
        let mut env = Environment::new();
        Theme::new()
            .fonts(FontSettings::new().body(font.clone()))
            .install(&mut env);

        let mut runtime = DewRuntime::new(HostBoard::new(200, 60), env, 16, || {
            AnyView::new(text("Dew"))
        });

        runtime.pump().expect("initial frame must render");
        assert_eq!(
            shaped_font_sizes(&runtime.current),
            vec![16.0_f32.to_bits()],
            "the installed body font shapes the first frame"
        );

        font.set(ResolvedFont::new(28.0, FontWeight::Normal));
        let frame = runtime
            .pump()
            .expect("a body font change must request a frame of its own");

        assert_eq!(
            shaped_font_sizes(&runtime.current),
            vec![28.0_f32.to_bits()],
            "the new body font must re-shape the retained text"
        );
        assert!(
            !frame.dirty.is_empty(),
            "re-shaped text must flush the region it changed"
        );
    }

    /// A span's own slot must be watched too, not only the body font a bare
    /// `text("…")` shapes at: `.font(Title)` reads the Title slot, and a
    /// theme that drives that slot has to re-shape the span it styles.
    #[test]
    fn span_font_change_reshapes_retained_text() {
        use waterui::Plugin as _;
        use waterui::theme::{FontSettings, Theme};
        use waterui_text::font::{FontWeight, ResolvedFont, Title};

        let title = binding(ResolvedFont::new(22.0, FontWeight::Normal));
        let mut env = Environment::new();
        Theme::new()
            .fonts(FontSettings::new().title(title.clone()))
            .install(&mut env);

        let mut runtime = DewRuntime::new(HostBoard::new(240, 80), env, 16, || {
            AnyView::new(text("Dew").font(Title))
        });

        runtime.pump().expect("initial frame must render");
        assert_eq!(
            shaped_font_sizes(&runtime.current),
            vec![22.0_f32.to_bits()],
            "the installed title font shapes the span on the first frame"
        );

        title.set(ResolvedFont::new(34.0, FontWeight::Normal));
        let frame = runtime
            .pump()
            .expect("a title font change must request a frame of its own");

        assert_eq!(
            shaped_font_sizes(&runtime.current),
            vec![34.0_f32.to_bits()],
            "the new title font must re-shape the span"
        );
        assert!(
            !frame.dirty.is_empty(),
            "re-shaped text must flush the region it changed"
        );
    }

    /// Dew supplies its own type scale, so an application that installs no
    /// theme still renders text: the body font slot is resolved inside
    /// `waterui-text`, which panics when the environment carries no token.
    #[test]
    fn text_renders_without_an_installed_theme() {
        let mut runtime = DewRuntime::new(HostBoard::new(200, 40), Environment::new(), 16, || {
            AnyView::new(text("Dew"))
        });

        let frame = runtime.pump().expect("initial frame must render");
        assert!(!frame.dirty.is_empty(), "text must paint something");
    }

    #[test]
    fn input_queued_before_initial_render_is_applied_after_targets_exist() {
        let value = binding(false);
        let root_value = value.clone();
        let mut board = HostBoard::new(200, 40);
        board.push_pointer(crate::PointerSample {
            x: 180.0,
            y: 20.0,
            phase: TouchPhase::Started,
        });
        board.push_pointer(crate::PointerSample {
            x: 180.0,
            y: 20.0,
            phase: TouchPhase::Ended,
        });
        let mut runtime = DewRuntime::new(board, Environment::new(), 16, move || {
            AnyView::new(Toggle::new("Queued input", &root_value).hide_label())
        });

        runtime.pump().expect("initial frame must render");
        assert!(!value.get(), "initial rendering must not discard input");
        runtime
            .pump()
            .expect("queued input must render after hit targets exist");
        assert!(value.get());
    }

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
