//! The platform-neutral input vocabulary of a self-drawn interactive view.
//!
//! A view that paints its own interactive content — a GPU surface, a text
//! editor, a game — receives keyboard, IME, pointer and scroll input through
//! one event enum on every Apple platform. The keyboard half is the W3C
//! UI Events model: [`Key`] is the logical value (what the user typed, after
//! layout and modifiers), [`Code`] is the physical key (where it sits on the
//! keyboard), and [`Modifiers`] is the chord state, all re-exported from the
//! `keyboard-types` crate.
//!
//! All positions are logical points, surface-local: the surface's own
//! top-left is `(0, 0)` and one unit is one logical pixel.

pub use keyboard_types::{Code, Key, Location, Modifiers, NamedKey};

use kurbo::Point;

/// A pointer button, in the W3C UI Events button vocabulary.
///
/// Platform buttons with no W3C meaning (extra mouse buttons past forward)
/// are not delivered rather than being reported as a button a view would
/// misinterpret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerButton {
    /// The primary button — the left button on a right-handed mouse, a tap.
    Primary,
    /// The secondary button — the right button on a right-handed mouse.
    Secondary,
    /// The middle button, usually the scroll wheel pressed down.
    Middle,
    /// The "back" side button.
    Back,
    /// The "forward" side button.
    Forward,
}

/// What one unit of a [`SurfaceEvent::Scroll`] delta means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScrollUnit {
    /// Deltas count lines of text — a discrete mouse wheel.
    Line,
    /// Deltas are logical pixels — a trackpad or precise wheel.
    Pixel,
}

/// One input event delivered to a view that asked for input.
#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceEvent {
    /// Keyboard focus entered (`true`) or left (`false`) this surface.
    ///
    /// Key, text and composition events only arrive while focused.
    Focus(bool),
    /// The active modifier chord changed.
    ///
    /// Key events carry their own modifiers; this reports a change that
    /// happens without one, so a view can update hover feedback or a cursor.
    Modifiers(Modifiers),
    /// The pointer moved over the surface, or moved anywhere while this
    /// surface holds the press capture.
    PointerMove {
        /// Where the pointer now is.
        position: Point,
    },
    /// A pointer button went down (`pressed`) or up.
    PointerButton {
        /// `true` for a press, `false` for a release.
        pressed: bool,
        /// Which button changed state.
        button: PointerButton,
        /// Where the pointer was when it changed.
        position: Point,
    },
    /// A scroll gesture over the surface.
    Scroll {
        /// Where the pointer was during the gesture.
        position: Point,
        /// Horizontal delta, positive when the content should move left.
        delta_x: f64,
        /// Vertical delta, positive when the content should move up.
        delta_y: f64,
        /// What one unit of the deltas means.
        unit: ScrollUnit,
        /// `true` on the event that ends a continuous gesture, so a view can
        /// settle momentum or release a scroll-driven state. Discrete wheel
        /// notches carry `true` because each notch is complete on its own.
        finished: bool,
    },
    /// A key went down (`pressed`) or up while this surface had focus.
    ///
    /// This is the raw key, not text. A press that produces text is followed
    /// by *its* [`SurfaceEvent::TextInput`] — pressed key, then text, then
    /// the release — the order the web platform gives `keydown` and
    /// `beforeinput`, and the only order this vocabulary takes.
    Key {
        /// `true` for a key press, `false` for a release.
        pressed: bool,
        /// The logical key — what the layout and modifiers produce.
        key: Key,
        /// The physical key — where it sits on the keyboard.
        code: Code,
        /// The modifier chord held while the key changed state.
        modifiers: Modifiers,
        /// `true` when the platform generated this press by auto-repeat.
        repeat: bool,
    },
    /// Text to insert at the caret, already committed by the platform.
    ///
    /// Following a [`SurfaceEvent::Key`] press it is that press's text — the
    /// committed character a dead key or IME resolved, which wins over the
    /// character the logical key implies. It may also arrive standalone, for
    /// text no key produced (an injected insertion).
    TextInput(String),
    /// An input-method composition session began.
    CompositionStart,
    /// The in-progress (pre-edit) composition text changed.
    ///
    /// This text is *not* committed: it is shown underlined at the caret
    /// until the session ends.
    CompositionUpdate {
        /// The current pre-edit text.
        text: String,
        /// Caret offset within `text`, in bytes, when the platform reports
        /// one.
        caret: Option<usize>,
    },
    /// The composition session ended and its text is to be inserted.
    CompositionCommit(String),
    /// The composition session was abandoned; any pre-edit text is
    /// discarded.
    CompositionCancel,
}

/// One step in a continuous gesture's life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GesturePhase {
    /// The gesture began.
    Began,
    /// The gesture's measurement changed.
    Changed,
    /// The gesture completed.
    Ended,
    /// The gesture was cancelled before completing.
    Cancelled,
}

/// The phase an input event reports for itself, where the platform reports
/// one at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventPhase {
    /// The event carries no phase — a discrete event like a wheel notch.
    None,
    /// The event begins a continuous gesture.
    Began,
    /// The event continues a continuous gesture.
    Changed,
    /// The event ends a continuous gesture.
    Ended,
    /// The event cancels a continuous gesture.
    Cancelled,
}

/// Pointer and gesture input a self-drawn surface wants for its own
/// interaction model — as opposed to [`SurfaceEvent`], which is input the
/// surface's content consumes.
///
/// This is the channel a GPU surface's own interaction (pointer position
/// for hover and hit testing, pinch to zoom, two-finger pan, double-tap to
/// reset) flows through. All positions are logical points, surface-local.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerInteraction {
    /// The hover point moved over the surface (`Some`) or left it (`None`).
    Moved(Option<Point>),
    /// The primary pointer went down.
    PrimaryDown {
        /// Where it went down.
        position: Point,
        /// How many rapid presses this is — 2 for a double-click.
        click_count: i64,
    },
    /// The primary pointer went up.
    PrimaryUp,
    /// A pinch (magnification) gesture.
    Pinch {
        /// Where the gesture is.
        phase: GesturePhase,
        /// The gesture's magnitude: the change since the last event on
        /// `AppKit` (`NSMagnificationGestureRecognizer.magnification`), the
        /// cumulative scale since `Began` on `UIKit`
        /// (`UIPinchGestureRecognizer.scale`).
        magnitude: f64,
        /// The gesture center.
        center: Point,
    },
    /// A pan gesture, reported as a scroll-wheel equivalent on `AppKit` and a
    /// dedicated recognizer on `UIKit`.
    Pan {
        /// Where the gesture is; [`EventPhase::None`] on a discrete wheel
        /// event, which arrives as an immediate began-plus-ended pair.
        phase: EventPhase,
        /// The horizontal offset or delta, in logical points.
        offset_x: f64,
        /// The vertical offset or delta, in logical points.
        offset_y: f64,
    },
    /// A double-tap gesture fired.
    DoubleTap,
}
