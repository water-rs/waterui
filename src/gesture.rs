//! Declarative gesture descriptors used by `WaterUI` components.
//!
//! This module defines lightweight gesture specifications that can be attached to widgets.
//! Each gesture type captures the minimum configuration necessary for a backend to register
//! and recognize the interaction, while remaining portable across platforms.

use waterui_core::handler::{BoxHandler, HandlerFn, into_handler};

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
/// allowing gesture handlers to extract the payload using [`Use<TapEvent>`](waterui_core::extract::Use).
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
    pub fn first(&self) -> &Gesture {
        &self.first
    }

    /// Returns a reference to the gesture that should run after the first one completes.
    #[must_use]
    pub fn then(&self) -> &Gesture {
        &self.then
    }
}

macro_rules! impl_gesture {
    ($(($name:ty, $variant:ident)),*) => {
        $(
            impl $name {
                /// Chains another gesture to run after this one succeeds.
                pub fn then(self, other: Gesture) -> Gesture {
                    Gesture::Then(Box::new(Then {
                        first: Gesture::$variant(self),
                        then: other,
                    }))
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
#[derive(Debug)]
#[non_exhaustive]
pub struct GestureObserver {
    /// The gesture to observe.
    pub gesture: Gesture,
    /// The action to execute when the gesture is recognized.
    pub action: BoxHandler<()>,
}

impl GestureObserver {
    /// Creates a new gesture observer that executes the given action when the gesture is recognized.
    pub fn new<P>(gesture: impl Into<Gesture>, action: impl HandlerFn<P, ()> + 'static) -> Self
    where
        P: 'static,
    {
        Self {
            gesture: gesture.into(),
            action: Box::new(into_handler(action)),
        }
    }
}
