//! Top-level windows.
//!
//! # Safety
//!
//! The `unsafe` here creates a window, keeps it from releasing itself, and
//! defines its delegate class. A window created in code releases itself when
//! closed unless told otherwise, which would free it under the `Retained` this
//! wrapper owns; [`Window::new`] turns that off before anything else touches
//! the window. The delegate's methods have the signatures `NSWindowDelegate`
//! declares, and `AppKit` sends them on the main thread.

use std::cell::RefCell;
use std::fmt;
use std::ptr::NonNull;
use std::rc::Rc;

use bitflags::bitflags;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSAppearance, NSAppearanceCustomization,
    NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSBackingStoreType, NSFloatingWindowLevel,
    NSNormalWindowLevel, NSView, NSWindow, NSWindowDelegate, NSWindowLevel, NSWindowStyleMask,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSString};

use crate::appkit::view_controller::ViewController;
use crate::callback::guarded;
use crate::color::Rgba;
use crate::geometry::{Rect, Size};

bitflags! {
    /// The parts of a window's frame. No flags is a borderless window.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct WindowStyle: u16 {
        /// A title bar.
        const TITLED = 1 << 0;
        /// A close button.
        const CLOSABLE = 1 << 1;
        /// A minimize button.
        const MINIATURIZABLE = 1 << 2;
        /// Resizable edges and a zoom button.
        const RESIZABLE = 1 << 3;
        /// The content area extends behind the title bar and toolbars.
        const FULL_SIZE_CONTENT_VIEW = 1 << 4;
        /// The window's live full-screen state — `AppKit` owns the bit; a
        /// style rewrite keeps it so the write does not ask the window to
        /// leave full screen.
        const FULL_SCREEN = 1 << 5;
    }
}

/// Where a window stacks relative to other applications' windows —
/// `NSWindow.level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowLevel {
    /// Stacked with ordinary windows — `NSNormalWindowLevel`.
    Normal,
    /// Above ordinary windows — `NSFloatingWindowLevel`, always on top.
    Floating,
}

impl WindowLevel {
    const fn native(self) -> NSWindowLevel {
        match self {
            Self::Normal => NSNormalWindowLevel,
            Self::Floating => NSFloatingWindowLevel,
        }
    }

    /// The `WindowLevel` `level` names — `None` for a level the enum does
    /// not model.
    fn from_native(level: NSWindowLevel) -> Option<Self> {
        if level == NSNormalWindowLevel {
            Some(Self::Normal)
        } else if level == NSFloatingWindowLevel {
            Some(Self::Floating)
        } else {
            None
        }
    }
}

const STYLE_PARTS: [(WindowStyle, NSWindowStyleMask); 6] = [
    (WindowStyle::TITLED, NSWindowStyleMask::Titled),
    (WindowStyle::CLOSABLE, NSWindowStyleMask::Closable),
    (
        WindowStyle::MINIATURIZABLE,
        NSWindowStyleMask::Miniaturizable,
    ),
    (WindowStyle::RESIZABLE, NSWindowStyleMask::Resizable),
    (
        WindowStyle::FULL_SIZE_CONTENT_VIEW,
        NSWindowStyleMask::FullSizeContentView,
    ),
    (WindowStyle::FULL_SCREEN, NSWindowStyleMask::FullScreen),
];

impl WindowStyle {
    fn native(self) -> NSWindowStyleMask {
        STYLE_PARTS
            .into_iter()
            .filter(|(part, _)| self.contains(*part))
            .fold(NSWindowStyleMask::Borderless, |mask, (_, native)| {
                mask | native
            })
    }

    /// The `WindowStyle` bits a native mask carries — flags the enum does not
    /// model are dropped.
    fn from_native(mask: NSWindowStyleMask) -> Self {
        STYLE_PARTS
            .into_iter()
            .filter(|(_, native)| mask.contains(*native))
            .fold(Self::empty(), |style, (part, _)| style | part)
    }
}

/// A top-level window.
///
/// The window lives as long as this value: dropping it closes the window.
pub struct Window {
    window: Retained<NSWindow>,
    delegate: Retained<Delegate>,
}

impl Window {
    /// A hidden window whose content area is `content_rect`, in screen
    /// coordinates, with the frame parts in `style`.
    ///
    /// The window draws into a buffer and creates it immediately. Show it
    /// with [`Window::make_key_and_order_front`].
    #[must_use]
    pub fn new(mtm: MainThreadMarker, content_rect: Rect, style: WindowStyle) -> Self {
        // SAFETY: see the module safety note.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                content_rect.into(),
                style.native(),
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: see the module safety note.
        unsafe { window.setReleasedWhenClosed(false) };
        let delegate = Delegate::new(mtm);
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        Self { window, delegate }
    }

    /// Wraps a window the platform already created — a host handing its own
    /// `NSWindow` to the window manager — and installs the kit delegate so the
    /// `on_*` notification hooks fire. The window's current delegate is
    /// replaced, matching the `window.delegate = delegate` the bound window
    /// contracts rely on; `releasedWhenClosed` stays whatever the host set,
    /// since the host owns the window's lifetime.
    #[must_use]
    pub fn adopt(mtm: MainThreadMarker, window: Retained<NSWindow>) -> Self {
        let delegate = Delegate::new(mtm);
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        Self { window, delegate }
    }

    /// The underlying `NSWindow`, for callers crossing into platform APIs the
    /// kit does not wrap.
    #[must_use]
    pub fn native(&self) -> &NSWindow {
        &self.window
    }

    /// The window's style mask as `WindowStyle` bits; flags the enum does not
    /// model are dropped.
    #[must_use]
    pub fn style_mask(&self) -> WindowStyle {
        WindowStyle::from_native(self.window.styleMask())
    }

    /// Replaces the window's style mask.
    pub fn set_style_mask(&self, style: WindowStyle) {
        self.window.setStyleMask(style.native());
    }

    /// The content rect of a window whose frame would be `frame` in `style` —
    /// `NSWindow.contentRect(forFrameRect:styleMask:)` — used to size a window
    /// created from a declared window frame.
    #[must_use]
    pub fn content_rect_for_frame(mtm: MainThreadMarker, frame: Rect, style: WindowStyle) -> Rect {
        NSWindow::contentRectForFrameRect_styleMask(frame.into(), style.native(), mtm).into()
    }

    /// The frame a window with content rect `content` in `style` would carry —
    /// `NSWindow.frameRect(forContentRect:styleMask:)` — the inverse of
    /// [`Self::content_rect_for_frame`].
    #[must_use]
    pub fn frame_rect_for_content(
        mtm: MainThreadMarker,
        content: Rect,
        style: WindowStyle,
    ) -> Rect {
        NSWindow::frameRectForContentRect_styleMask(content.into(), style.native(), mtm).into()
    }

    /// Reveals the window's contents: fades `alphaValue` to opaque over
    /// `duration` seconds with ease-out timing, the way a fresh window first
    /// appears.
    pub fn fade_in(&self, duration: f64) {
        let window = self.window.clone();
        let changes = RcBlock::new(move |context: NonNull<NSAnimationContext>| {
            // SAFETY: AppKit hands the block a live `NSAnimationContext`.
            let context = unsafe { context.as_ref() };
            context.setDuration(duration);
            // SAFETY: see the module safety note.
            unsafe {
                context.setTimingFunction(Some(
                    &objc2_quartz_core::CAMediaTimingFunction::functionWithName(
                        objc2_quartz_core::kCAMediaTimingFunctionEaseOut,
                    ),
                ));
                window.animator().setAlphaValue(1.0);
            }
        });
        NSAnimationContext::runAnimationGroup(&changes);
    }

    /// Pins the window's appearance to `scheme`, or hands appearance back to
    /// the system when `scheme` is `None`.
    pub fn set_appearance(&self, scheme: Option<crate::ColorScheme>) {
        // SAFETY: the name statics are AppKit constants; see the module
        // safety note.
        let name = unsafe {
            scheme.map(|scheme| match scheme {
                crate::ColorScheme::Light => NSAppearanceNameAqua,
                crate::ColorScheme::Dark => NSAppearanceNameDarkAqua,
            })
        };
        self.window
            .setAppearance(name.and_then(NSAppearance::appearanceNamed).as_deref());
    }

    /// Sets the text of the title bar.
    pub fn set_title(&self, title: &str) {
        self.window.setTitle(&NSString::from_str(title));
    }

    /// The window's content area, in screen coordinates: its frame without
    /// the title bar and borders.
    #[must_use]
    pub fn content_rect(&self) -> Rect {
        self.window
            .contentRectForFrameRect(self.window.frame())
            .into()
    }

    /// The window's frame, in screen coordinates.
    #[must_use]
    pub fn frame(&self) -> Rect {
        self.window.frame().into()
    }

    /// Moves and resizes the window to `frame`, in screen coordinates,
    /// redrawing it. `animate` asks the system to tween the change.
    pub fn set_frame(&self, frame: Rect, animate: bool) {
        self.window
            .setFrame_display_animate(frame.into(), true, animate);
    }

    /// Moves and resizes the window so its content area becomes `content`,
    /// in screen coordinates. `animate` asks the system to tween the change.
    pub fn set_content_rect(&self, content: Rect, animate: bool) {
        let frame = self.window.frameRectForContentRect(content.into());
        self.window.setFrame_display_animate(frame, true, animate);
    }

    /// Makes `view` fill the content area, replacing the view there.
    pub fn set_content_view(&self, view: &NSView) {
        self.window.setContentView(Some(view));
    }

    /// Hands the window's content to `controller`, whose view then fills the
    /// content area and joins the responder chain as a controller.
    ///
    /// A view hierarchy that contains controller-based components — a split
    /// view most importantly — needs this: `NSSplitViewItem` only reaches the
    /// titlebar when the split view lives under the window's view controller.
    pub fn set_content_view_controller(&self, controller: &ViewController) {
        self.window
            .setContentViewController(Some(controller.native()));
    }

    /// Moves the window to the center of its screen, a little above the
    /// middle.
    pub fn center(&self) {
        self.window.center();
    }

    /// Shows the window in front of the application's other windows and gives
    /// it keyboard focus.
    ///
    /// A window whose key-view loop is already populated picks the first
    /// valid key view as its first responder when it becomes key. Callers
    /// that mount content before ordering and want no control focused at
    /// reveal clear that pick with [`Self::clear_first_responder`].
    pub fn make_key_and_order_front(&self) {
        self.window.makeKeyAndOrderFront(None);
    }

    /// Hands first responder back to the window itself, undoing the
    /// automatic pick `AppKit` makes when the window becomes key with a
    /// populated key-view loop.
    pub fn clear_first_responder(&self) {
        self.window.makeFirstResponder(None);
    }

    /// Shows the window without giving it keyboard focus.
    pub fn order_front(&self) {
        self.window.orderFront(None);
    }

    /// Whether the window is currently hidden.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.window.isVisible()
    }

    /// The smallest content area the user can resize the window to.
    #[must_use]
    pub fn content_min_size(&self) -> Size {
        self.window.contentMinSize().into()
    }

    /// Sets the smallest content area the user can resize the window to.
    pub fn set_content_min_size(&self, size: Size) {
        self.window.setContentMinSize(size.into());
    }

    /// Sets the largest content area the user can resize the window to.
    pub fn set_content_max_size(&self, size: Size) {
        self.window.setContentMaxSize(size.into());
    }

    /// Calls `handler` every time the window is about to close, replacing any
    /// handler set before.
    pub fn on_close(&self, handler: impl Fn() + 'static) {
        self.delegate.ivars().close.replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` every time the window finishes a resize, live or
    /// otherwise, replacing any handler set before.
    pub fn on_resize(&self, handler: impl Fn() + 'static) {
        self.delegate.ivars().resize.replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` every time the window finishes moving, replacing any
    /// handler set before.
    pub fn on_move(&self, handler: impl Fn() + 'static) {
        self.delegate.ivars().moved.replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` after a live (dragged) resize ends, replacing any
    /// handler set before.
    pub fn on_live_resize_end(&self, handler: impl Fn() + 'static) {
        self.delegate
            .ivars()
            .live_resize_end
            .replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` after the window miniaturizes, replacing any handler
    /// set before.
    pub fn on_miniaturized(&self, handler: impl Fn() + 'static) {
        self.delegate
            .ivars()
            .miniaturized
            .replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` after the window returns from the Dock, replacing any
    /// handler set before.
    pub fn on_deminiaturized(&self, handler: impl Fn() + 'static) {
        self.delegate
            .ivars()
            .deminiaturized
            .replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` after the window enters full screen, replacing any
    /// handler set before.
    pub fn on_entered_fullscreen(&self, handler: impl Fn() + 'static) {
        self.delegate
            .ivars()
            .entered_fullscreen
            .replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` after the window leaves full screen, replacing any
    /// handler set before.
    pub fn on_exited_fullscreen(&self, handler: impl Fn() + 'static) {
        self.delegate
            .ivars()
            .exited_fullscreen
            .replace(Some(Rc::new(handler)));
    }

    /// The window's level — where it stacks relative to other applications'
    /// windows, `NSWindow.level`. `None` when the window sits at a level
    /// [`WindowLevel`] does not model.
    #[must_use]
    pub fn level(&self) -> Option<WindowLevel> {
        WindowLevel::from_native(self.window.level())
    }

    /// Moves the window to `level` — `Floating` keeps it above ordinary
    /// windows, `Normal` stacks it with them.
    pub fn set_level(&self, level: WindowLevel) {
        self.window.setLevel(level.native());
    }

    /// Whether the window fills its screen's visible frame — `NSWindow`'s
    /// user-driven maximize.
    #[must_use]
    pub fn is_zoomed(&self) -> bool {
        self.window.isZoomed()
    }

    /// Toggles the window's zoom: a zoomed window shrinks back, any other
    /// fills the screen's visible frame.
    ///
    /// `AppKit` ignores the call on a miniaturized or full-screen window, so
    /// a caller driving window state unwinds those first.
    pub fn zoom(&self) {
        self.window.zoom(None);
    }

    /// The steps the window's content size moves in while the user resizes
    /// it — `NSWindow.contentResizeIncrements`.
    pub fn set_content_resize_increments(&self, size: Size) {
        self.window.setContentResizeIncrements(size.into());
    }

    /// Calls `handler` after the window becomes key, replacing any handler
    /// set before.
    pub fn on_became_key(&self, handler: impl Fn() + 'static) {
        self.delegate
            .ivars()
            .became_key
            .replace(Some(Rc::new(handler)));
    }

    /// Whether the window is collapsed into the Dock.
    #[must_use]
    pub fn is_miniaturized(&self) -> bool {
        self.window.isMiniaturized()
    }

    /// Collapses the window into the Dock.
    pub fn miniaturize(&self) {
        self.window.miniaturize(None);
    }

    /// Brings the window back from the Dock.
    pub fn deminiaturize(&self) {
        self.window.deminiaturize(None);
    }

    /// Whether the window is drawing full screen.
    #[must_use]
    pub fn is_fullscreen(&self) -> bool {
        self.window
            .styleMask()
            .contains(NSWindowStyleMask::FullScreen)
    }

    /// Moves the window in or out of full screen.
    pub fn toggle_fullscreen(&self) {
        self.window.toggleFullScreen(None);
    }

    /// Closes the window, calling the [`Window::on_close`] handler first.
    pub fn close(&self) {
        self.window.close();
    }

    /// How transparent the window and everything in it is, `0.0` to `1.0`.
    pub fn set_alpha_value(&self, alpha: f64) {
        self.window.setAlphaValue(alpha);
    }

    /// Whether the window sees the mouse move even when no button is held.
    ///
    /// Views that track hover need this on: `AppKit` only delivers
    /// `mouseMoved:` events to windows that asked for them.
    pub fn set_accepts_mouse_moved_events(&self, accepts: bool) {
        self.window.setAcceptsMouseMovedEvents(accepts);
    }

    /// The color drawn behind the window's content, `None` for the default.
    pub fn set_background_color(&self, color: Rgba) {
        let native = objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
            color.red,
            color.green,
            color.blue,
            color.alpha,
        );
        self.window.setBackgroundColor(Some(&native));
    }

    /// Whether the window treats itself as opaque for compositing.
    pub fn set_opaque(&self, opaque: bool) {
        self.window.setOpaque(opaque);
    }

    /// Whether the window draws a shadow.
    pub fn set_has_shadow(&self, has_shadow: bool) {
        self.window.setHasShadow(has_shadow);
    }

    /// Draws every part of the window marked as needing display, now rather
    /// than at the end of this turn of the run loop.
    pub fn display_if_needed(&self) {
        self.window.displayIfNeeded();
    }
}

impl fmt::Debug for Window {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Window")
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // The close handler belongs to the value being dropped, so it does not
        // hear about this close.
        self.window.setDelegate(None);
        self.window.close();
    }
}

type Handler = Rc<dyn Fn()>;

#[derive(Default)]
struct DelegateIvars {
    close: RefCell<Option<Handler>>,
    resize: RefCell<Option<Handler>>,
    moved: RefCell<Option<Handler>>,
    live_resize_end: RefCell<Option<Handler>>,
    became_key: RefCell<Option<Handler>>,
    miniaturized: RefCell<Option<Handler>>,
    deminiaturized: RefCell<Option<Handler>>,
    entered_fullscreen: RefCell<Option<Handler>>,
    exited_fullscreen: RefCell<Option<Handler>>,
}

impl DelegateIvars {
    fn fire(slot: &RefCell<Option<Handler>>) {
        // Cloned out of the cell so the handler may replace itself.
        let handler = slot.borrow().clone();
        if let Some(handler) = handler {
            handler();
        }
    }
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and the class does
    // not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiWindowDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    struct Delegate;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for Delegate {}

    // SAFETY: see the module safety note.
    unsafe impl NSWindowDelegate for Delegate {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            guarded("windowWillClose:", || {
                DelegateIvars::fire(&self.ivars().close);
            });
        }

        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notification: &NSNotification) {
            guarded("windowDidResize:", || {
                DelegateIvars::fire(&self.ivars().resize);
            });
        }

        #[unsafe(method(windowDidMove:))]
        fn window_did_move(&self, _notification: &NSNotification) {
            guarded("windowDidMove:", || {
                DelegateIvars::fire(&self.ivars().moved);
            });
        }

        #[unsafe(method(windowDidEndLiveResize:))]
        fn window_did_end_live_resize(&self, _notification: &NSNotification) {
            guarded("windowDidEndLiveResize:", || {
                DelegateIvars::fire(&self.ivars().live_resize_end);
            });
        }

        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _notification: &NSNotification) {
            guarded("windowDidBecomeKey:", || {
                DelegateIvars::fire(&self.ivars().became_key);
            });
        }

        #[unsafe(method(windowDidMiniaturize:))]
        fn window_did_miniaturize(&self, _notification: &NSNotification) {
            guarded("windowDidMiniaturize:", || {
                DelegateIvars::fire(&self.ivars().miniaturized);
            });
        }

        #[unsafe(method(windowDidDeminiaturize:))]
        fn window_did_deminiaturize(&self, _notification: &NSNotification) {
            guarded("windowDidDeminiaturize:", || {
                DelegateIvars::fire(&self.ivars().deminiaturized);
            });
        }

        #[unsafe(method(windowDidEnterFullScreen:))]
        fn window_did_enter_full_screen(&self, _notification: &NSNotification) {
            guarded("windowDidEnterFullScreen:", || {
                DelegateIvars::fire(&self.ivars().entered_fullscreen);
            });
        }

        #[unsafe(method(windowDidExitFullScreen:))]
        fn window_did_exit_full_screen(&self, _notification: &NSNotification) {
            guarded("windowDidExitFullScreen:", || {
                DelegateIvars::fire(&self.ivars().exited_fullscreen);
            });
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars::default());
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// The main screen's backing scale — the display scale an off-window view
/// rasterizes at, `1.0` when there is no main screen.
///
/// # Panics
///
/// Off the main thread.
#[must_use]
pub fn main_screen_scale() -> f64 {
    objc2_app_kit::NSScreen::mainScreen(objc2::MainThreadMarker::new().expect("main thread"))
        .map_or(1.0, |screen| screen.backingScaleFactor())
}

/// Whether `window` could show a frame it was handed: visible and not
/// occluded.
#[must_use]
pub fn is_visible(window: &NSWindow) -> bool {
    window.isVisible()
        && window
            .occlusionState()
            .contains(objc2_app_kit::NSWindowOcclusionState::Visible)
}

/// Calls `handler` on the main thread every time `window`'s occlusion state
/// changes.
///
/// # Panics
///
/// If called off the main thread.
pub fn watch_occlusion(
    mtm: objc2::MainThreadMarker,
    window: &NSWindow,
    handler: impl Fn() + 'static,
) -> crate::notification::NotificationObserver {
    crate::notification::observe_object(
        mtm,
        &crate::notification::NotificationName::framework(
            // SAFETY: the notification name is a system constant.
            unsafe { objc2_app_kit::NSWindowDidChangeOcclusionStateNotification },
        ),
        window.as_ref(),
        handler,
    )
}

#[cfg(test)]
mod tests {
    use objc2_app_kit::NSWindowStyleMask;

    use super::WindowStyle;

    #[test]
    fn style_maps_to_its_mask() {
        assert_eq!(
            WindowStyle::all().native(),
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable
                | NSWindowStyleMask::FullSizeContentView
                | NSWindowStyleMask::FullScreen
        );
        assert_eq!(
            (WindowStyle::TITLED | WindowStyle::RESIZABLE).native(),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Resizable
        );
        assert_eq!(WindowStyle::empty().native(), NSWindowStyleMask::Borderless);
    }

    #[test]
    fn style_round_trips_through_the_native_mask() {
        for style in [
            WindowStyle::empty(),
            WindowStyle::TITLED | WindowStyle::RESIZABLE,
            WindowStyle::TITLED | WindowStyle::CLOSABLE | WindowStyle::FULL_SIZE_CONTENT_VIEW,
            WindowStyle::all(),
        ] {
            assert_eq!(WindowStyle::from_native(style.native()), style);
        }
        // Flags the enum does not model are dropped on the way back.
        assert_eq!(
            WindowStyle::from_native(
                NSWindowStyleMask::Titled | NSWindowStyleMask::UnifiedTitleAndToolbar
            ),
            WindowStyle::TITLED
        );
    }
}
