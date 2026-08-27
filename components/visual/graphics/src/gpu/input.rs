//! Backend-neutral input vocabulary for GPU surfaces.
//!
//! A [`GpuView`](super::gpu_surface::GpuView) that draws its own interactive
//! content — a browser engine, a terminal, a text editor, a game — needs the
//! keyboard, IME, pointer and scroll events that reach its layer, not just the
//! pointer state a [`GpuFrame`](super::gpu_surface::GpuFrame) exposes. Every
//! backend used to invent its own adapter for that, so an engine had to be
//! ported once per backend. This module is the single vocabulary they all
//! speak: a backend translates its platform events into
//! [`SurfaceInputEvent`] once, and every input-hungry GPU view works on every
//! backend that does.
//!
//! The keyboard half is the W3C UI Events model, taken wholesale from the
//! [`keyboard_types`] crate rather than reinvented: [`Key`] is the logical
//! value (what the user typed, after layout and modifiers), [`Code`] is the
//! physical key (where it sits on the keyboard), and [`Modifiers`] is the
//! chord state. Text arriving through an input method is *not* a key event —
//! it is a composition session ([`SurfaceInputEvent::CompositionStart`] …
//! [`SurfaceInputEvent::CompositionCommit`]) or a plain
//! [`SurfaceInputEvent::TextInput`] insertion.
//!
//! All positions are **logical, surface-local** points: the surface's own
//! top-left is `(0, 0)` and one unit is one logical pixel, whatever the
//! display scale. A view never has to know its placement in the window.

pub use keyboard_types::{Code, Key, Location, Modifiers, NamedKey};

use kurbo::Point;
use waterui_core::Str;

/// A pointer button, in the W3C UI Events button vocabulary.
///
/// Platform buttons with no W3C meaning (extra mouse buttons past forward) are
/// not delivered rather than being reported as a button a view would
/// misinterpret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SurfacePointerButton {
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

/// What one unit of a [`SurfaceInputEvent::Scroll`] delta means.
///
/// A wheel notch and a trackpad glide are not the same gesture, and a view
/// that renders its own content has to tell them apart: lines are quantised
/// and want the view's own line height applied, pixels are already the
/// distance the content should move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScrollUnit {
    /// Deltas count lines of text — a discrete mouse wheel.
    Line,
    /// Deltas are logical pixels — a trackpad or precise wheel.
    Pixel,
}

/// One input event delivered to a [`GpuView`](super::gpu_surface::GpuView)
/// that asked for input with
/// [`wants_input_events`](super::gpu_surface::GpuView::wants_input_events).
///
/// Positions are logical and surface-local (see the [module
/// docs](self)).
#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceInputEvent {
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
        button: SurfacePointerButton,
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
    /// This is the raw key, not text: a key that produces text is followed by
    /// a [`SurfaceInputEvent::TextInput`], exactly as the web platform does.
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
    TextInput(Str),
    /// An input-method composition session began.
    CompositionStart,
    /// The in-progress (pre-edit) composition text changed.
    ///
    /// This text is *not* committed: it is shown underlined at the caret until
    /// the session ends.
    CompositionUpdate {
        /// The current pre-edit text.
        text: Str,
        /// Caret offset within `text`, in bytes, when the platform reports one.
        caret: Option<usize>,
    },
    /// The composition session ended and its text is to be inserted.
    CompositionCommit(Str),
    /// The composition session was abandoned; any pre-edit text is discarded.
    CompositionCancel,
}

#[cfg(test)]
mod tests {
    use super::{Code, Key, Modifiers, NamedKey, ScrollUnit, SurfaceInputEvent};

    #[test]
    fn key_events_carry_both_the_logical_and_the_physical_key() {
        let event = SurfaceInputEvent::Key {
            pressed: true,
            key: Key::Character("a".into()),
            code: Code::KeyA,
            modifiers: Modifiers::SHIFT,
            repeat: false,
        };
        let SurfaceInputEvent::Key { key, code, .. } = &event else {
            panic!("constructed a key event");
        };
        assert_eq!(*key, Key::Character("a".into()));
        assert_eq!(*code, Code::KeyA);
    }

    #[test]
    fn named_keys_use_the_w3c_vocabulary() {
        assert_eq!(
            "ArrowLeft".parse::<Key>().expect("W3C named key"),
            Key::Named(NamedKey::ArrowLeft)
        );
        assert_eq!(
            " ".parse::<Key>().expect("space is a character key"),
            Key::Character(" ".into())
        );
    }

    #[test]
    fn scroll_units_are_distinct() {
        assert_ne!(ScrollUnit::Line, ScrollUnit::Pixel);
    }
}
