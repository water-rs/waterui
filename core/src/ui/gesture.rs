//! Declarative gesture descriptors used by `WaterUI` components.
//!
//! This module defines lightweight gesture specifications that can be attached to widgets.
//! Each gesture type captures the minimum configuration necessary for a backend to register
//! and recognize the interaction, while remaining portable across platforms.
//! Pointer-hover and cursor appearance are intentionally modeled outside this module,
//! matching SwiftUI-style separation.
//!
//! # Hit-Testing Behavior
//!
//! `WaterUI` uses a **pass-through** hit-testing model where views without gesture handlers
//! are transparent to touch events. This means:
//!
//! - **Non-interactive views** (e.g., `Spacer`, plain `Text`, layout containers) do not
//!   intercept touches. Touches pass through to views behind them in the Z-order.
//!
//! - **Interactive views** (e.g., `Button`, views with [`GestureObserver`] attached) capture
//!   touches within their bounds and prevent them from reaching views below.
//!
//! - In a `ZStack` or overlay, only the topmost *interactive* view at a touch location
//!   receives the event. Non-interactive overlays (like a semi-transparent background or
//!   loading indicator) allow touches to reach interactive content beneath them.
//!
//! ## Example: Video Player with Overlay Controls
//!
//! ```ignore
//! // The controls_overlay uses a Spacer to push buttons to the bottom.
//! // The Spacer is non-interactive, so tapping the video area behind it
//! // still triggers the VideoPlayer's native controls.
//! zstack((
//!     video_player(url).show_controls(true),
//!     vstack((
//!         spacer(),  // Non-interactive: touches pass through to VideoPlayer
//!         button("Play").action(|| { /* ... */ }),  // Interactive: captures touches
//!     )),
//! ))
//! ```
//!
//! ## Backend Implementation Requirements
//!
//! Backend implementors must ensure:
//!
//! 1. Layout containers (`VStack`, `HStack`, `ZStack`) do not consume unhandled touch events.
//! 2. Only views with registered gesture handlers or inherent interactivity (buttons, sliders)
//!    should return `true` from hit-test queries.
//! 3. When multiple views overlap, the hit-test should find the topmost *interactive* view,
//!    not simply the topmost view in the Z-order.

use alloc::boxed::Box;
use core::fmt;

use crate::{
    handler::{BoxedAction, Handler, boxed_action},
    metadata::MetadataKey,
};

/// Represents the phase of a gesture interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GesturePhase {
    /// The gesture has just begun.
    Started,
    /// The gesture is actively updating.
    Updated,
    /// The gesture has completed successfully.
    Ended,
    /// The gesture was cancelled before completion.
    Cancelled,
}

/// A two-dimensional point used to describe gesture locations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GesturePoint {
    /// Horizontal component of the point.
    pub x: f32,
    /// Vertical component of the point.
    pub y: f32,
}

impl GesturePoint {
    /// Creates a new [`GesturePoint`].
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Event payload for tap gestures.
///
/// Backends place this structure into the environment when a tap is recognised,
/// allowing gesture handlers to extract the payload using [`Use<TapEvent>`](crate::extract::Use).
#[derive(Debug, Clone, PartialEq)]
pub struct TapEvent {
    /// Location of the tap in the widget's coordinate space.
    pub location: GesturePoint,
    /// Number of taps that occurred in succession.
    pub count: u32,
}

/// Event payload for long-press gestures.
///
/// Backends insert this into the environment alongside [`Gesture::LongPress`]
/// whenever a long-press interaction fires.
#[derive(Debug, Clone, PartialEq)]
pub struct LongPressEvent {
    /// Location of the press in the widget's coordinate space.
    pub location: GesturePoint,
    /// Duration, in platform-defined time units, that the press was held.
    pub duration: f32,
}

/// Event payload for drag gestures.
///
/// Each drag update stores a fresh [`DragEvent`] in the environment so handlers
/// can observe pointer position and motion metrics.
#[derive(Debug, Clone, PartialEq)]
pub struct DragEvent {
    /// Phase of the drag gesture.
    pub phase: GesturePhase,
    /// Current location of the pointer.
    pub location: GesturePoint,
    /// Total translation since the drag started.
    pub translation: GesturePoint,
    /// Velocity of the drag in points per second.
    pub velocity: GesturePoint,
}

/// Event payload for magnification (pinch) gestures.
///
/// This payload accompanies [`Gesture::Magnification`] entries in the
/// environment when zoom gestures are recognised.
#[derive(Debug, Clone, PartialEq)]
pub struct MagnificationEvent {
    /// Phase of the magnification gesture.
    pub phase: GesturePhase,
    /// Focal point of the gesture.
    pub center: GesturePoint,
    /// Current scale factor relative to the gesture start.
    pub scale: f32,
    /// Rate of change of the scale factor.
    pub velocity: f32,
}

/// Describes a tap interaction that must occur a specific number of times.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TapGesture {
    /// The number of consecutive taps required to trigger this gesture.
    pub count: u32,
}

impl TapGesture {
    /// Creates a tap gesture that requires `count` consecutive taps to activate.
    #[must_use]
    pub const fn repeat(count: u32) -> Self {
        Self { count }
    }

    /// Creates a tap gesture that requires a single tap to activate.
    #[must_use]
    pub const fn new() -> Self {
        Self { count: 1 }
    }
}

impl Default for TapGesture {
    fn default() -> Self {
        Self::new()
    }
}

/// Describes a long-press interaction that must be held for a minimum duration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LongPressGesture {
    /// The minimum duration (in time units) the press must be held.
    pub duration: u32,
}

impl LongPressGesture {
    /// Creates a long-press gesture that activates after holding for `duration` time units.
    ///
    /// Backends decide how to interpret the unit (for example milliseconds), allowing
    /// platform-specific gesture systems to provide consistent behaviour.
    #[must_use]
    pub const fn new(duration: u32) -> Self {
        Self { duration }
    }
}

/// Describes a drag interaction that begins after the pointer moves beyond a threshold.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DragGesture {
    /// The minimum distance the pointer must travel to initiate the drag.
    pub min_distance: f32,
}

impl DragGesture {
    /// Creates a drag gesture requiring the pointer to travel at least `min_distance` units.
    #[must_use]
    pub const fn new(min_distance: f32) -> Self {
        Self { min_distance }
    }
}

/// Describes a magnification (pinch/zoom) interaction starting from an initial scale factor.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MagnificationGesture {
    /// The initial scale factor when the gesture begins.
    pub initial_scale: f32,
}

impl MagnificationGesture {
    /// Creates a magnification gesture beginning at `initial_scale`.
    #[must_use]
    pub const fn new(initial_scale: f32) -> Self {
        Self { initial_scale }
    }
}

/// Describes a rotation interaction initialized with a starting angle.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RotationGesture {
    /// The initial angle (in radians) when the gesture begins.
    pub initial_angle: f32,
}

impl RotationGesture {
    /// Creates a rotation gesture beginning at `initial_angle` radians.
    #[must_use]
    pub const fn new(initial_angle: f32) -> Self {
        Self { initial_angle }
    }
}

/// High-level gesture descriptions that can be attached to widgets.
///
/// When a backend recognises a gesture it mirrors the interaction by inserting
/// the corresponding [`Gesture`] variant into the environment so handlers can
/// inspect which gesture fired alongside the variant-specific payload types.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Gesture {
    /// A tap gesture that requires a specific number of consecutive taps.
    Tap(TapGesture),
    /// A long-press gesture that activates after holding for a minimum duration.
    LongPress(LongPressGesture),
    /// A drag gesture that begins after the pointer moves beyond a threshold.
    Drag(DragGesture),
    /// A magnification (pinch/zoom) gesture starting from an initial scale factor.
    Magnification(MagnificationGesture),
    /// A rotation gesture initialized with a starting angle.
    Rotation(RotationGesture),
    /// A sequential composition of two gestures where the second runs after the first completes.
    Then(Box<Then>),
    /// A composition where two gestures can be recognized in parallel.
    Simultaneous(Box<Simultaneous>),
    /// A composition where the first gesture has priority and the second is a fallback.
    Exclusive(Box<Exclusive>),
}

/// Combines two gestures so the second runs only after the first completes.
#[derive(Debug, Clone, PartialEq)]
pub struct Then {
    first: Gesture,
    then: Gesture,
}

impl Then {
    /// Returns a reference to the first gesture in the sequence.
    #[must_use]
    pub const fn first(&self) -> &Gesture {
        &self.first
    }

    /// Returns a reference to the gesture that should run after the first one completes.
    #[must_use]
    pub const fn then(&self) -> &Gesture {
        &self.then
    }
}

/// Combines two gestures so they can be recognized at the same time.
#[derive(Debug, Clone, PartialEq)]
pub struct Simultaneous {
    first: Gesture,
    second: Gesture,
}

impl Simultaneous {
    /// Returns a reference to the first gesture in this composition.
    #[must_use]
    pub const fn first(&self) -> &Gesture {
        &self.first
    }

    /// Returns a reference to the second gesture in this composition.
    #[must_use]
    pub const fn second(&self) -> &Gesture {
        &self.second
    }
}

/// Combines two gestures where the first has recognition priority over the second.
#[derive(Debug, Clone, PartialEq)]
pub struct Exclusive {
    first: Gesture,
    second: Gesture,
}

impl Exclusive {
    /// Returns a reference to the primary gesture.
    #[must_use]
    pub const fn first(&self) -> &Gesture {
        &self.first
    }

    /// Returns a reference to the fallback gesture.
    #[must_use]
    pub const fn second(&self) -> &Gesture {
        &self.second
    }
}

impl Gesture {
    /// Chains another gesture that runs only after this gesture succeeds.
    #[must_use]
    pub fn then(self, other: impl Into<Self>) -> Self {
        Self::Then(Box::new(Then {
            first: self,
            then: other.into(),
        }))
    }

    /// SwiftUI-style alias for [`Gesture::then`].
    #[must_use]
    pub fn sequenced_before(self, other: impl Into<Self>) -> Self {
        self.then(other)
    }

    /// Combines this gesture with another so they can be recognized simultaneously.
    #[must_use]
    pub fn simultaneously_with(self, other: impl Into<Self>) -> Self {
        Self::Simultaneous(Box::new(Simultaneous {
            first: self,
            second: other.into(),
        }))
    }

    /// Combines this gesture with another where this gesture has priority.
    #[must_use]
    pub fn exclusively_before(self, other: impl Into<Self>) -> Self {
        Self::Exclusive(Box::new(Exclusive {
            first: self,
            second: other.into(),
        }))
    }
}

macro_rules! impl_gesture {
    ($(($name:ty, $variant:ident)),*) => {
        $(
            impl $name {
                /// Chains another gesture to run after this one succeeds.
                #[must_use]
                pub fn then(self, other: impl Into<Gesture>) -> Gesture {
                    Gesture::$variant(self).then(other)
                }

                /// SwiftUI-style alias for [`Self::then`].
                #[must_use]
                pub fn sequenced_before(self, other: impl Into<Gesture>) -> Gesture {
                    self.then(other)
                }

                /// Combines two gestures that can be recognized simultaneously.
                #[must_use]
                pub fn simultaneously_with(self, other: impl Into<Gesture>) -> Gesture {
                    Gesture::$variant(self).simultaneously_with(other)
                }

                /// Combines two gestures where this gesture has priority.
                #[must_use]
                pub fn exclusively_before(self, other: impl Into<Gesture>) -> Gesture {
                    Gesture::$variant(self).exclusively_before(other)
                }
            }

            impl From<$name> for Gesture {
                fn from(gesture: $name) -> Self {
                    Gesture::$variant(gesture)
                }
            }
        )*
    };
}

impl_gesture! {
    (TapGesture, Tap),
    (LongPressGesture, LongPress),
    (DragGesture, Drag),
    (MagnificationGesture, Magnification),
    (RotationGesture, Rotation)
}

/// Observes a gesture and executes an action when the gesture is recognized.
#[non_exhaustive]
pub struct GestureObserver {
    /// The gesture to observe.
    pub gesture: Gesture,
    /// The action to execute when the gesture is recognized.
    pub action: BoxedAction<()>,
}

impl fmt::Debug for GestureObserver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GestureObserver")
            .field("gesture", &self.gesture)
            .finish_non_exhaustive()
    }
}

impl MetadataKey for GestureObserver {}

impl GestureObserver {
    /// Creates a gesture observer for the given gesture and action.
    ///
    /// Local state can be injected by wrapping the owning view with
    /// `.state(...)`.
    ///
    /// # Examples
    ///
    /// Simple action:
    /// ```rust,ignore
    /// GestureObserver::new(TapGesture::new(), || {})
    /// ```
    ///
    /// With injected state:
    /// ```rust,ignore
    /// GestureObserver::new(
    ///     TapGesture::repeat(2),
    ///     |State(counter): State<Binding<i32>>| counter.set(counter.get() + 1),
    /// )
    /// ```
    #[must_use]
    pub fn new<Args>(gesture: impl Into<Gesture>, action: impl Handler<Args, ()>) -> Self {
        Self {
            gesture: gesture.into(),
            action: boxed_action(action),
        }
    }

    /// Creates a gesture observer builder for the given gesture.
    #[must_use]
    pub fn builder(gesture: impl Into<Gesture>) -> GestureObserverBuilder {
        GestureObserverBuilder::new(gesture)
    }
}

// ============================================================================
// GestureObserver Builder
// ============================================================================

/// Builder for creating gesture observers with extracted actions.
#[derive(Debug)]
pub struct GestureObserverBuilder {
    gesture: Gesture,
}

impl GestureObserverBuilder {
    /// Creates a gesture observer builder for the given gesture.
    #[must_use]
    pub fn new(gesture: impl Into<Gesture>) -> Self {
        Self {
            gesture: gesture.into(),
        }
    }

    /// Sets the action handler (no state).
    #[must_use]
    pub fn action<Args>(self, action: impl Handler<Args, ()>) -> GestureObserver {
        GestureObserver::new(self.gesture, action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequenced_before_aliases_then() {
        let gesture = TapGesture::new().sequenced_before(LongPressGesture::new(300));
        match gesture {
            Gesture::Then(pair) => {
                assert!(matches!(pair.first(), Gesture::Tap(_)));
                assert!(matches!(pair.then(), Gesture::LongPress(_)));
            }
            other => panic!("expected Gesture::Then, got {other:?}"),
        }
    }

    #[test]
    fn simultaneous_composition_contains_both_gestures() {
        let gesture = TapGesture::new().simultaneously_with(DragGesture::new(8.0));
        match gesture {
            Gesture::Simultaneous(pair) => {
                assert!(matches!(pair.first(), Gesture::Tap(_)));
                assert!(matches!(pair.second(), Gesture::Drag(_)));
            }
            other => panic!("expected Gesture::Simultaneous, got {other:?}"),
        }
    }

    #[test]
    fn exclusive_composition_contains_primary_and_fallback() {
        let gesture = TapGesture::new().exclusively_before(LongPressGesture::new(500));
        match gesture {
            Gesture::Exclusive(pair) => {
                assert!(matches!(pair.first(), Gesture::Tap(_)));
                assert!(matches!(pair.second(), Gesture::LongPress(_)));
            }
            other => panic!("expected Gesture::Exclusive, got {other:?}"),
        }
    }
}
