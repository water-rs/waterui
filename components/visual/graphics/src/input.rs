//! Backend-neutral input vocabulary for self-drawn views.
//!
//! A view that draws its own interactive content — a browser engine, a
//! terminal, a text editor, a game — needs the keyboard, IME, pointer and
//! scroll events that reach it, not just the pointer state a frame exposes.
//!
//! Two kinds of view receive them: a `GpuContentView` (with the `gpu`
//! feature), whose engine-hosted GPU content gets them through its
//! `on_input` handler, and a
//! [`SceneContent`](crate::backend::SceneContent), which records into a
//! `waterui_graphics::backend::draw::Recorder` and lets the backend decide how the scene reaches
//! the screen.
//! Every backend used to invent its own adapter for this, so an engine had to
//! be ported once per backend. This module is the single vocabulary they all
//! speak: a backend translates its platform events into
//! [`SurfaceInputEvent`] once, and every input-hungry view works on every
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

/// The logical, surface-local geometry the events carry: a pointer's
/// position and a surface's size.
pub use crate::draw::kurbo::{Point, Size};

use core::num::NonZeroU32;

use waterui_core::Str;

/// A pointer button, in the W3C UI Events button vocabulary.
///
/// Platform buttons with no W3C meaning (extra mouse buttons past forward) are
/// not delivered rather than being reported as a button a view would
/// misinterpret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every embedded surface host maps each W3C button variant by variant; a new button must fail their build instead of falling into a wildcard arm"
)]
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
#[expect(
    clippy::exhaustive_enums,
    reason = "every embedded surface host converts each scroll unit variant by variant; a new unit must fail their build instead of falling into a wildcard arm"
)]
pub enum ScrollUnit {
    /// Deltas count lines of text — a discrete mouse wheel.
    Line,
    /// Deltas are logical pixels — a trackpad or precise wheel.
    Pixel,
}

/// Where a continuous gesture is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[expect(
    clippy::exhaustive_enums,
    reason = "a closed gesture lifecycle every embedded surface host maps variant by variant; a new phase must fail their build instead of falling into a wildcard arm"
)]
pub enum GesturePhase {
    /// The gesture began. Its delta may be zero.
    Started,
    /// The gesture continued.
    Moved,
    /// The gesture ended normally.
    Ended,
    /// The platform abandoned the gesture; a view rolls back whatever the
    /// gesture had not committed.
    Cancelled,
}

/// Which part of a scroll gesture a [`SurfaceInputEvent::Scroll`] delta
/// belongs to.
///
/// A view that runs its own kinetic scrolling or overscroll needs to tell a
/// finger on the trackpad from the inertia the platform generates after
/// lift-off, and both from a delta that is complete on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every embedded surface host maps each scroll phase variant by variant; a new phase must fail their build instead of falling into a wildcard arm"
)]
pub enum ScrollPhase {
    /// A delta complete on its own: a wheel notch, or a platform that reports
    /// no gesture around its scroll deltas.
    Discrete,
    /// The user is touching the input device — fingers on a trackpad.
    Contact(GesturePhase),
    /// Inertia the platform generates after the fingers lift. A momentum
    /// sequence follows a [`ScrollPhase::Contact`] sequence that ended with
    /// [`GesturePhase::Ended`]; platforms that do not generate momentum never
    /// deliver it, and the view supplies its own.
    Momentum(GesturePhase),
}

/// One input event delivered to a view that asked for input.
///
/// The receiver is a `GpuContentView`'s `on_input` handler or a
/// [`SceneContent`](crate::backend::SceneContent) returning `true` from its
/// `wants_input_events`.
///
/// Positions are logical and surface-local (see the [module
/// docs](self)).
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every embedded surface host forwards each input event variant by variant; a new event must fail their build instead of falling into a wildcard arm"
)]
pub enum SurfaceInputEvent {
    /// The surface took on a new size or display scale.
    ///
    /// Delivered once before the surface's first frame and again whenever
    /// its logical size or scale changes, always before the first frame
    /// rendered at the new size, so a view whose model depends on its size —
    /// a terminal's columns and rows — recomputes it once instead of
    /// comparing every frame against the last.
    Resize {
        /// The surface's size in logical points.
        size: Size,
        /// Pixels per logical point.
        scale: f32,
    },
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
        /// Which click of a multi-click run this is: 1 for a single click, 2
        /// for a double click, 3 for a triple click, and so on, counted with
        /// the platform's double-click interval and distance. A release
        /// carries the count of the press it ends, as the W3C
        /// [`UIEvent.detail`](https://w3c.github.io/uievents/#dom-uievent-detail)
        /// of `mousedown` and `mouseup` does.
        click_count: NonZeroU32,
    },
    /// A scroll gesture over the surface.
    Scroll {
        /// Where the pointer was during the gesture.
        position: Point,
        /// Horizontal delta, positive when the content should move left — a
        /// wheel turn to the right. This is the sign of the W3C
        /// [`WheelEvent.deltaX`](https://w3c.github.io/uievents/#dom-wheelevent-deltax).
        delta_x: f64,
        /// Vertical delta, positive when the content should move up — a wheel
        /// turned toward the user, scrolling down. This is the sign of the W3C
        /// [`WheelEvent.deltaY`](https://w3c.github.io/uievents/#dom-wheelevent-deltay).
        delta_y: f64,
        /// What one unit of the deltas means.
        unit: ScrollUnit,
        /// Which part of the gesture this delta belongs to.
        phase: ScrollPhase,
    },
    /// A key went down (`pressed`) or up while this surface had focus.
    ///
    /// This is the raw key, not text. A press that produces text is followed
    /// by *its* [`SurfaceInputEvent::TextInput`] — pressed key, then text,
    /// then the release — the order the web platform gives `keydown` and
    /// `beforeinput`, and the only order this vocabulary takes. Text that
    /// follows a press belongs to that press, so "this press produced no
    /// text" needs no timeout: the next event of any kind is the answer,
    /// because every backend delivers its events down one ordered queue.
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
    /// Following a [`SurfaceInputEvent::Key`] press it is that press's text —
    /// the committed character a dead key or IME resolved, which wins over
    /// the character the logical key implies. It may also arrive standalone,
    /// for text no key produced (an injected insertion).
    TextInput(Str),
    /// The platform's paste command targeted this focused surface, carrying
    /// the clipboard's text.
    ///
    /// The paste command is the Edit menu's and the context menu's Paste and
    /// the platform's paste shortcut (Command-V on Apple platforms, Control-V
    /// elsewhere). The shortcut's key press is the command: it is delivered
    /// as this event and never also as a [`SurfaceInputEvent::Key`]. Paste
    /// is distinct from [`SurfaceInputEvent::TextInput`] so a view can treat
    /// pasted text differently from typed text — a terminal's bracketed
    /// paste. A paste with no text on the clipboard is not delivered.
    Paste(Str),
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
