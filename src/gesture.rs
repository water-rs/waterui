//! Declarative gesture descriptors used by `WaterUI` components.
//!
//! This module defines lightweight gesture specifications that can be attached to widgets.
//! Each gesture type captures the minimum configuration necessary for a backend to register
//! and recognize the interaction, while remaining portable across platforms.

use waterui_core::handler::{BoxHandler, HandlerFn, into_handler};

/// Describes a tap interaction that must occur a specific number of times.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TapGesture {
    pub count: u32,
}

impl TapGesture {
    /// Creates a tap gesture that fires after `count` consecutive taps.
    #[must_use] 
    pub const fn new(count: u32) -> Self {
        Self { count }
    }
}

/// Describes a long-press interaction that must be held for a minimum duration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LongPressGesture {
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
