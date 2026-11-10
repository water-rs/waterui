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

use core::num::NonZeroU32;

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

/// Which part of a scroll gesture a [`SurfaceEvent::Scroll`] delta belongs
/// to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScrollPhase {
    /// A delta complete on its own: a wheel notch, or a device that reports
    /// no gesture around its deltas.
    Discrete,
    /// The user is touching the input device — fingers on a trackpad.
    Contact(GesturePhase),
    /// Inertia the platform generates after the fingers lift.
    Momentum(GesturePhase),
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
        /// Which click of a multi-click run this is — 1 for a single click,
        /// 2 for a double click, counted by the platform with its own
        /// interval and distance. A release carries the count of the press
        /// it ends.
        click_count: NonZeroU32,
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
        /// Which part of a scroll gesture this delta belongs to.
        phase: ScrollPhase,
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
    /// The platform's paste command targeted this focused surface, carrying
    /// the pasteboard's text.
    ///
    /// The shortcut's own key press and release are not also delivered as
    /// [`SurfaceEvent::Key`]. A paste with no text on the pasteboard is not
    /// delivered.
    Paste(String),
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

/// A platform's click count as the neutral, non-zero count.
///
/// `NSEvent.clickCount` and `UITouch.tapCount` count the press itself, so a
/// real press is at least 1. A count of 0 comes from an event synthesized
/// without click state; it is still a press, so it reads as the first
/// click.
#[must_use]
pub fn click_count(platform_count: usize) -> NonZeroU32 {
    NonZeroU32::new(u32::try_from(platform_count).unwrap_or(u32::MAX)).unwrap_or(NonZeroU32::MIN)
}

/// Whether a key press with `modifiers` that types `key` is the platform's
/// paste shortcut: Command with the letter V and no other chord modifier.
///
/// The logical key decides, as the platform's own menu shortcuts do, so a
/// layout that types V elsewhere still pastes on that key. Lock state does
/// not make a chord a different chord.
#[must_use]
pub fn is_paste_shortcut(modifiers: Modifiers, key: &Key) -> bool {
    modifiers.difference(Modifiers::CAPS_LOCK | Modifiers::NUM_LOCK) == Modifiers::META
        && matches!(key, Key::Character(text) if text.eq_ignore_ascii_case("v"))
}

#[cfg(test)]
mod tests {
    use keyboard_types::{Key, Modifiers, NamedKey};

    use super::{click_count, is_paste_shortcut};

    #[test]
    fn only_command_v_is_the_paste_shortcut() {
        let v = Key::Character("v".into());
        assert!(is_paste_shortcut(Modifiers::META, &v));
        assert!(is_paste_shortcut(
            Modifiers::META | Modifiers::CAPS_LOCK,
            &Key::Character("V".into())
        ));
        assert!(!is_paste_shortcut(Modifiers::empty(), &v));
        assert!(!is_paste_shortcut(Modifiers::META | Modifiers::SHIFT, &v));
        assert!(!is_paste_shortcut(Modifiers::META | Modifiers::ALT, &v));
        assert!(!is_paste_shortcut(Modifiers::CONTROL, &v));
        assert!(!is_paste_shortcut(
            Modifiers::META,
            &Key::Character("c".into())
        ));
        assert!(!is_paste_shortcut(
            Modifiers::META,
            &Key::Named(NamedKey::Enter)
        ));
    }

    #[test]
    fn a_platform_click_count_is_at_least_the_first_click() {
        assert_eq!(click_count(0).get(), 1);
        assert_eq!(click_count(1).get(), 1);
        assert_eq!(click_count(2).get(), 2);
        assert_eq!(click_count(3).get(), 3);
        assert_eq!(click_count(40).get(), 40, "a run is not capped");
        assert_eq!(click_count(usize::MAX).get(), u32::MAX);
    }
}
