//! The `UIKit` scene's host window: the platform services behind one view.
//!
//! [`SceneShared`] is the state `UIKit` callbacks and the frame transaction
//! both touch. It is interior-mutable and never borrowed across a `UIKit`
//! call, so a callback that fires while a transaction runs — the keyboard
//! asking for the document, a layout pass — cannot find the session
//! borrowed. Callbacks only stash an event or ask for a frame; the next
//! transaction applies the metrics, insets and input they left.
//!
//! [`UiKitHostWindow`] is the [`PlatformWindow`] the session renders into:
//! window metrics are read off the view at the head of each transaction,
//! input arrives as the events the callbacks queued, and a redraw request
//! resumes the scene's display link.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cocoa_ui::display_link::FrameClock;
use cocoa_ui::main_queue::{MainQueueWake, MainThreadBound, enqueue_after_local};
use cocoa_ui::notification::NotificationObserver;
use cocoa_ui::uikit::input_view::InputView;
use cocoa_ui::uikit::{KeyboardBand, PointerCursor, WindowScene};
use cocoa_ui::{MainThreadMarker, Retained};
use nami::Signal as _;
use waterui::cursor::CursorStyle;
use waterui::window::{Window, WindowState};
use waterui_core::Environment;
use waterui_core::layout::{Size, layout_direction};
use waterui_graphics::gpu::RedrawHandle;
use waterui_layout::padding::EdgeInsets;

use super::super::input::InputTranslator;
use super::super::session::AppleHostWindow;
use super::super::surface::AppleViewSurface;
use super::insets;
use crate::num_cast::{f64_as_f32, f64_as_u32};
use crate::platform::{
    GpuSurfaceWindow, InputEvent, PlatformWindow, PresentationSurface as _, ScrollPlatform,
    TextInputState, TouchScrollConfig, validated_window_frame,
};
use crate::runner::host_executor::{UiThreadPoster, redraw_wake};
use crate::runner::window::{FrameOutcome, FrameTransaction};
use crate::time::Instant;

/// How long past a keyboard animation's reported duration frames keep
/// sampling its band, so the settled position is the last one read.
const KEYBOARD_SETTLE: Duration = Duration::from_millis(100);

/// The state one scene's `UIKit` callbacks and its frame transaction share.
pub(super) struct SceneShared {
    mtm: MainThreadMarker,
    this: Weak<Self>,
    scene: WindowScene,
    pub(super) view: Retained<InputView>,
    keyboard_band: KeyboardBand,
    clock: FrameClock,
    /// The transaction the clock's tick runs, installed once the session
    /// exists.
    tick: RefCell<Option<Rc<dyn Fn()>>>,
    transaction: FrameTransaction,
    /// A redraw the engine asked for. Atomic because the GPU-surface redraw
    /// handle latches it from whichever thread its producer runs on.
    redraw_pending: Arc<AtomicBool>,
    events: RefCell<Vec<InputEvent>>,
    translator: RefCell<InputTranslator>,
    /// The scene is between `willEnterForeground` and `didEnterBackground`:
    /// a backgrounded process must submit no GPU work.
    foreground: Cell<bool>,
    /// The end of the keyboard animation the band is being sampled through.
    keyboard_animation_until: Cell<Option<std::time::Instant>>,
    /// Identifies the one outstanding deadline wake, so a replaced one
    /// finds itself stale.
    deadline_generation: Cell<u64>,
    /// The screen the scene was last seen on, by identity.
    screen: Cell<Option<usize>>,
    /// The fastest rate that screen displays, in frames per second.
    max_frames_per_second: Cell<u32>,
}

impl SceneShared {
    /// The shared state of a scene showing `view`.
    pub(super) fn new(
        mtm: MainThreadMarker,
        scene: WindowScene,
        view: Retained<InputView>,
        keyboard_band: KeyboardBand,
        foreground: bool,
    ) -> Rc<Self> {
        Rc::new_cyclic(|this: &Weak<Self>| Self {
            mtm,
            this: this.clone(),
            max_frames_per_second: Cell::new(scene.max_frames_per_second()),
            scene,
            view,
            keyboard_band,
            clock: FrameClock::new(mtm, {
                let this = this.clone();
                move || {
                    let tick = this
                        .upgrade()
                        .and_then(|shared| shared.tick.borrow().clone());
                    if let Some(tick) = tick {
                        tick();
                    }
                }
            }),
            tick: RefCell::new(None),
            transaction: FrameTransaction::default(),
            redraw_pending: Arc::new(AtomicBool::new(false)),
            events: RefCell::new(Vec::new()),
            translator: RefCell::new(InputTranslator::default()),
            foreground: Cell::new(foreground),
            keyboard_animation_until: Cell::new(None),
            deadline_generation: Cell::new(0),
            screen: Cell::new(None),
        })
    }

    /// The fastest rate the scene's screen displays, in frames per second.
    pub(super) const fn max_frames_per_second(&self) -> u32 {
        self.max_frames_per_second.get()
    }

    /// Follows the scene onto another screen — an external display, a
    /// Stage Manager move — by re-picking the frame clock's rate for it.
    /// Returns whether the scene moved off a screen it was already seen on,
    /// which is when the output must be re-negotiated.
    fn follow_screen(&self) -> bool {
        let Some(screen) = self.view.screen_id() else {
            return false;
        };
        let previous = self.screen.replace(Some(screen));
        if previous == Some(screen) {
            return false;
        }
        if let Some(rate) = self.view.max_frames_per_second() {
            self.max_frames_per_second.set(rate);
        }
        self.clock.reselect(&self.view);
        previous.is_some()
    }

    /// Follows the keyboard's show, hide and resize animations: frames keep
    /// coming for their duration, each sampling the guide's in-flight band.
    pub(super) fn observe_keyboard(&self) -> NotificationObserver {
        let this = self.this.clone();
        self.keyboard_band
            .observe_changes(self.mtm, move |duration| {
                if let Some(shared) = this.upgrade() {
                    shared.keyboard_will_animate(duration);
                }
            })
    }

    /// Installs the transaction the display link ticks, and arms the link
    /// parked: it ticks only once a frame is requested.
    pub(super) fn start(&self, tick: Rc<dyn Fn()>) {
        self.tick.replace(Some(tick));
        self.clock.set_paused(true);
        self.clock.start(&self.view);
    }

    /// Stops the display link for good: the scene is detaching.
    pub(super) fn stop(&self) {
        self.clock.stop();
        self.tick.replace(None);
        self.deadline_generation
            .set(self.deadline_generation.get() + 1);
    }

    /// Runs a transaction at the next display tick, whatever the window's
    /// occlusion: callbacks that may move visibility itself (a layout pass,
    /// a safe-area change) use this, and the transaction re-derives
    /// occlusion before it decides what to draw.
    pub(super) fn request_frame(&self) {
        self.clock.set_paused(false);
    }

    /// The redraw request of [`PlatformWindow::request_redraw`]: latched for
    /// the next render, and a frame only while the gate is open — inside a
    /// transaction the latch joins its continuation, and an occluded window
    /// keeps it armed for the restore frame.
    fn request_redraw(&self) {
        self.redraw_pending.store(true, Ordering::Release);
        if self.transaction.may_post() {
            self.request_frame();
        }
    }

    pub(super) const fn foreground(&self) -> bool {
        self.foreground.get()
    }

    pub(super) fn set_foreground(&self, foreground: bool) {
        self.foreground.set(foreground);
        if !foreground {
            self.clock.set_paused(true);
        }
    }

    /// Queues the event `UIKit` delivered for the next transaction.
    pub(super) fn push_surface_event(&self, event: cocoa_ui::input::SurfaceEvent) {
        let Some(event) = self.translator.borrow_mut().translate(event) else {
            return;
        };
        self.push_event(event);
    }

    /// Queues `event` for the next transaction without waking one: for the
    /// transaction's own head, which dispatches it a moment later.
    fn queue_event(&self, event: InputEvent) {
        self.events.borrow_mut().push(event);
    }

    /// Queues `event` for the next transaction and wakes it.
    pub(super) fn push_event(&self, event: InputEvent) {
        self.queue_event(event);
        // A scene in the background draws nothing; the event dispatches on
        // the frame the foreground restores.
        if self.foreground.get() {
            self.request_frame();
        }
    }

    /// Keeps frames coming for as long as the keyboard animates, so each one
    /// samples where the band is now.
    pub(super) fn keyboard_will_animate(&self, duration: f64) {
        let animation = Duration::try_from_secs_f64(duration.max(0.0))
            .expect("the keyboard animation duration is finite");
        self.keyboard_animation_until.set(Some(
            std::time::Instant::now() + animation + KEYBOARD_SETTLE,
        ));
        self.request_frame();
    }

    fn keyboard_animating(&self) -> bool {
        self.keyboard_animation_until
            .get()
            .is_some_and(|until| std::time::Instant::now() < until)
    }

    /// Settles the display link and the deadline wake after a transaction.
    pub(super) fn frame_done(&self, outcome: &FrameOutcome) {
        if !(outcome.wants_next_frame || self.keyboard_animating()) {
            self.clock.set_paused(true);
        }
        self.schedule_deadline(outcome.deadline);
    }

    /// Replaces the one outstanding fallback wake with one at `deadline`.
    fn schedule_deadline(&self, deadline: Option<Instant>) {
        let generation = self.deadline_generation.get() + 1;
        self.deadline_generation.set(generation);
        let Some(deadline) = deadline else {
            return;
        };
        let delay = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_default();
        let this = self.this.clone();
        enqueue_after_local(self.mtm, delay, move |_| {
            if let Some(shared) = this.upgrade()
                && shared.deadline_generation.get() == generation
            {
                shared.request_redraw();
            }
        });
    }
}

/// The window the session renders into.
pub(super) struct UiKitHostWindow {
    shared: Rc<SceneShared>,
    surface: AppleViewSurface,
    /// The pixel size reported to the pump, as the view's bounds at `scale`.
    size_px: (u32, u32),
    scale: f64,
    safe_area: nami::Binding<EdgeInsets>,
    keyboard_area: nami::Binding<EdgeInsets>,
    /// Shared with the surface: raised when the view changes scale, so the
    /// next frame re-negotiates its output.
    display_moved: Rc<Cell<bool>>,
    /// Posts onto the main thread from any thread.
    poster: UiThreadPoster<MainQueueWake>,
    /// The [`TextInputState::activation`] the soft keyboard was last shown
    /// for; `None` while no field holds focus.
    soft_input: Option<u64>,
    /// The title last handed to the scene.
    applied_title: Option<String>,
    /// The size limits last handed to the scene.
    applied_size_limits: Option<(Option<Size>, Option<Size>)>,
}

impl UiKitHostWindow {
    pub(super) fn new(
        shared: Rc<SceneShared>,
        surface: AppleViewSurface,
        display_moved: Rc<Cell<bool>>,
        safe_area: nami::Binding<EdgeInsets>,
        keyboard_area: nami::Binding<EdgeInsets>,
        poster: UiThreadPoster<MainQueueWake>,
    ) -> Self {
        let mut window = Self {
            shared,
            surface,
            size_px: (0, 0),
            scale: 0.0,
            safe_area,
            keyboard_area,
            display_moved,
            poster,
            soft_input: None,
            applied_title: None,
            applied_size_limits: None,
        };
        window.read_metrics();
        window
    }

    /// The cross-thread redraw wake, ending in the frame request on the main
    /// thread; see [`redraw_wake`].
    fn content_redraw_wake(&self) -> impl Fn() + Send + Sync + 'static {
        // The post runs on the main thread, where the scene is reachable;
        // the handle itself crosses threads.
        let shared = MainThreadBound::new(Rc::downgrade(&self.shared), self.shared.mtm);
        redraw_wake(
            self.poster.clone(),
            Arc::clone(&self.shared.redraw_pending),
            self.shared.transaction.ui_thread_wake(move || {
                let mtm = MainThreadMarker::new()
                    .expect("a post queued on the main-thread executor runs on the main thread");
                if let Some(shared) = shared.get(mtm).upgrade() {
                    shared.request_frame();
                }
            }),
        )
    }

    /// Reads the view's size and scale. Returns whether either moved.
    fn read_metrics(&mut self) -> bool {
        let view = &self.shared.view;
        // Off a screen the view has no scale; the window is occluded then,
        // and its metrics wait for the view to arrive.
        let Some(scale) = view.display_scale() else {
            return false;
        };
        let bounds = view.bounds().size;
        let size = (
            f64_as_u32((bounds.width * scale).round()),
            f64_as_u32((bounds.height * scale).round()),
        );
        if size == self.size_px && scale.to_bits() == self.scale.to_bits() {
            return false;
        }
        if scale.to_bits() != self.scale.to_bits() && self.scale != 0.0 {
            self.display_moved.set(true);
        }
        self.size_px = size;
        self.scale = scale;
        self.surface.resize(size.0, size.1);
        true
    }

    /// Writes the two inset regions, each only when its own value moved: the
    /// binding writes are the wake that re-lays out.
    fn sync_insets(&self, env: &Environment) {
        let view = &self.shared.view;
        let height = view.bounds().size.height;
        let safe_area = view.safe_area_insets();
        let direction = layout_direction(env).snapshot();
        let container = insets::container(safe_area, direction);
        if self.safe_area.snapshot() != container {
            self.safe_area.set(container);
        }
        let keyboard = insets::keyboard(height, self.shared.keyboard_band.top(), safe_area.bottom);
        if self.keyboard_area.snapshot() != keyboard {
            self.keyboard_area.set(keyboard);
        }
    }
}

impl PlatformWindow for UiKitHostWindow {
    fn content_size(&self) -> (u32, u32) {
        self.size_px
    }

    fn apply_properties(&mut self, window: &Window) {
        if window.state.snapshot() == WindowState::Closed {
            return;
        }
        let title = window.display_title().snapshot();
        if self.applied_title.as_deref() != Some(title.as_str()) {
            self.shared.scene.set_title(title.as_str());
            self.applied_title = Some(title.as_str().to_owned());
        }
        // The scene sizes the window; the app's `Window::frame` binding is a
        // mount-time request — validated, then driven by the view's size,
        // which is what the frame must fill. The write is value-gated:
        // `window.frame` is subscribed by the render pass itself, so an
        // unconditional `set` re-arms a refresh every frame and the pump
        // never idles.
        if self.scale == 0.0 {
            return;
        }
        let current = validated_window_frame(window.frame.snapshot());
        let target = waterui_core::layout::Rect::new(
            waterui_core::layout::Point::new(0.0, 0.0),
            waterui_core::layout::Size::new(
                f64_as_f32(f64::from(self.size_px.0) / self.scale),
                f64_as_f32(f64::from(self.size_px.1) / self.scale),
            ),
        );
        if current != target {
            window.frame.set(target);
        }
    }

    /// Only a scene the system lets the user resize has limits to apply;
    /// deriving them from the content costs measure passes a fixed-size
    /// iPhone scene never needs.
    fn applies_size_limits(&self) -> bool {
        self.shared.scene.supports_size_restrictions()
    }

    fn set_size_limits(&mut self, min: Option<Size>, max: Option<Size>) {
        if self.applied_size_limits == Some((min, max)) {
            return;
        }
        let (minimum, maximum) = size_restrictions(min, max);
        self.shared.scene.set_size_restrictions(minimum, maximum);
        self.applied_size_limits = Some((min, max));
    }

    fn drain_events(&mut self) -> Vec<InputEvent> {
        std::mem::take(&mut *self.shared.events.borrow_mut())
    }

    /// Nothing drawn reaches the user while the scene is in the background
    /// or its view is off a screen: `iOS` terminates a process that submits
    /// GPU work from the background.
    fn is_occluded(&self) -> bool {
        !self.shared.foreground() || !self.presentation_attached()
    }

    fn set_transparent(&mut self, transparent: bool) {
        self.surface.set_transparent(transparent);
    }

    fn request_redraw(&self) {
        self.shared.request_redraw();
    }

    fn frame_wake(&self) -> Rc<dyn Fn()> {
        let shared = Rc::clone(&self.shared);
        self.shared
            .transaction
            .frame_wake(Rc::new(move || shared.request_frame()))
    }

    fn scale_factor(&self) -> f64 {
        self.scale
    }

    fn touch_scroll_config(&self) -> Option<TouchScrollConfig> {
        Some(TouchScrollConfig::for_platform(ScrollPlatform::Apple))
    }

    fn refresh_rate_hz(&self) -> Option<f64> {
        Some(f64::from(self.shared.max_frames_per_second()))
    }

    fn sync_text_input_state(&mut self, state: Option<TextInputState>) {
        let soft_input = state.map(|state| state.activation);
        if soft_input == self.soft_input {
            return;
        }
        self.soft_input = soft_input;
        // The view is the first responder for hardware keys whenever it is
        // on screen; its input view is empty until a document is focused,
        // and the document's own reset reloads it. A press on an
        // already-focused field asks for the keyboard again.
        if soft_input.is_some() {
            let view = &self.shared.view;
            if view.isFirstResponder() {
                view.reloadInputViews();
            } else {
                view.becomeFirstResponder();
            }
        }
    }

    fn set_cursor_style(&mut self, style: CursorStyle) {
        self.shared.view.set_pointer_cursor(pointer_cursor(style));
    }
}

impl GpuSurfaceWindow for UiKitHostWindow {
    type Presentation = AppleViewSurface;

    fn surface(&mut self) -> &mut AppleViewSurface {
        &mut self.surface
    }

    fn gpu_surface_redraw_handle(&self) -> Option<RedrawHandle> {
        Some(RedrawHandle::new(self.content_redraw_wake()))
    }
}

impl AppleHostWindow for UiKitHostWindow {
    fn frame_transaction(&self) -> &FrameTransaction {
        &self.shared.transaction
    }

    fn presentation_attached(&self) -> bool {
        self.size_px.0 > 0 && self.size_px.1 > 0 && self.shared.view.window().is_some()
    }

    fn take_redraw_pending(&self) -> bool {
        self.shared.redraw_pending.swap(false, Ordering::AcqRel)
    }

    fn begin_frame(&mut self, env: &Environment) {
        if self.shared.follow_screen() {
            self.display_moved.set(true);
        }
        if self.read_metrics() {
            let (width, height) = self.size_px;
            self.shared
                .queue_event(InputEvent::Resize { width, height });
        }
        self.sync_insets(env);
    }

    fn close_requested(&mut self) {
        panic!(
            "hydrolysis UIKit: the window's content asked to close, but a UIKit scene cannot be \
             closed by the app"
        );
    }
}

/// The scene size restrictions, in points, for the window's content-size
/// limits: no minimum is a zero size, and a missing or infinite maximum on
/// an axis is the largest finite extent, since `UIKit` takes no unbounded
/// value.
fn size_restrictions(min: Option<Size>, max: Option<Size>) -> (cocoa_ui::Size, cocoa_ui::Size) {
    let extent = |value: f32| {
        if value.is_finite() {
            f64::from(value)
        } else {
            f64::from(f32::MAX)
        }
    };
    let minimum = min.map_or(cocoa_ui::Size::ZERO, |size| {
        cocoa_ui::Size::new(f64::from(size.width), f64::from(size.height))
    });
    let maximum = max.unwrap_or(Size::new(f32::INFINITY, f32::INFINITY));
    let maximum = cocoa_ui::Size::new(extent(maximum.width), extent(maximum.height));
    (minimum, maximum)
}

/// The pointer `iPadOS` can draw for `style`. It has the arrow, the text
/// I-beam and no pointer; a style with no counterpart there is the arrow, as
/// [`CursorStyle`] documents for a platform without it.
const fn pointer_cursor(style: CursorStyle) -> PointerCursor {
    match style {
        CursorStyle::IBeam => PointerCursor::Beam,
        _ => PointerCursor::Arrow,
    }
}
