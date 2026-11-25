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
//! ```text
//! // Shown as text: the authoring layer lives in crates that depend on this
//! // one, so it cannot be compiled from here.
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
use core::{fmt, num::NonZeroU32, time::Duration};

use waterui_core::{
    Environment, Error,
    extract::{ExtractionState, Extractor, State},
    handler::{
        BoxedAction, EventHandler, ExtractionError, Handler, boxed_action, boxed_event_handler,
    },
    layout::Point,
    metadata::MetadataKey,
};

/// A pointer button that can press a gesture.
///
/// A touch or a pen contact presses as [`Self::Primary`]; the other buttons
/// exist on mice and some pens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PointerButton {
    /// The main button: a left mouse click, a touch, a pen contact.
    Primary,
    /// The secondary button, usually a right click.
    Secondary,
    /// The middle button, usually a wheel click.
    Middle,
    /// The "back" side button.
    Back,
    /// The "forward" side button.
    Forward,
}

bitflags::bitflags! {
    /// The set of pointer buttons a gesture responds to.
    ///
    /// Tap, long-press and drag gestures respond to [`Self::PRIMARY`] unless
    /// told otherwise. A gesture that accepts [`Self::SECONDARY`] competes with
    /// a `.context_menu` at the same point: the context menu wins where one is
    /// declared, the way platforms reserve the secondary click for it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct PointerButtons: u8 {
        /// [`PointerButton::Primary`].
        const PRIMARY = 1 << 0;
        /// [`PointerButton::Secondary`].
        const SECONDARY = 1 << 1;
        /// [`PointerButton::Middle`].
        const MIDDLE = 1 << 2;
        /// [`PointerButton::Back`].
        const BACK = 1 << 3;
        /// [`PointerButton::Forward`].
        const FORWARD = 1 << 4;
    }
}

impl From<PointerButton> for PointerButtons {
    fn from(button: PointerButton) -> Self {
        match button {
            PointerButton::Primary => Self::PRIMARY,
            PointerButton::Secondary => Self::SECONDARY,
            PointerButton::Middle => Self::MIDDLE,
            PointerButton::Back => Self::BACK,
            PointerButton::Forward => Self::FORWARD,
        }
    }
}

impl PointerButtons {
    /// Whether `button` is in this set.
    #[must_use]
    pub fn accepts(self, button: PointerButton) -> bool {
        self.contains(button.into())
    }
}

impl Default for PointerButtons {
    fn default() -> Self {
        Self::PRIMARY
    }
}

/// Represents the phase of a gesture interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed gesture lifecycle (started, updated, ended, cancelled) that every backend maps its native recognizer onto"
)]
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

/// Event payload delivered as the first argument to a [`TapGesture`] handler.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TapEvent {
    /// Location of the tap in the widget's coordinate space.
    location: Point,
    /// Number of taps that occurred in succession.
    count: u32,
    /// The button that tapped.
    button: PointerButton,
}

impl TapEvent {
    /// Location of the tap in the widget's coordinate space.
    #[must_use]
    pub const fn location(&self) -> Point {
        self.location
    }

    /// Sets the `location`.
    pub const fn set_location(&mut self, location: Point) {
        self.location = location;
    }

    /// Number of taps that occurred in succession.
    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }

    /// The button that tapped.
    #[must_use]
    pub const fn button(&self) -> PointerButton {
        self.button
    }
}

impl TapEvent {
    /// A tap of `count` successive taps of `button` at `location`.
    #[must_use]
    pub const fn new(location: Point, count: u32, button: PointerButton) -> Self {
        Self {
            location,
            count,
            button,
        }
    }
}

/// Event payload delivered as the first argument to a [`LongPressGesture`] handler.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LongPressEvent {
    /// Location of the press in the widget's coordinate space.
    location: Point,
    /// Duration that the press was held.
    duration: Duration,
    /// The button that is held.
    button: PointerButton,
}

impl LongPressEvent {
    /// Location of the press in the widget's coordinate space.
    #[must_use]
    pub const fn location(&self) -> Point {
        self.location
    }

    /// Sets the `location`.
    pub const fn set_location(&mut self, location: Point) {
        self.location = location;
    }

    /// Duration that the press was held.
    #[must_use]
    pub const fn duration(&self) -> Duration {
        self.duration
    }

    /// The button that is held.
    #[must_use]
    pub const fn button(&self) -> PointerButton {
        self.button
    }
}

impl LongPressEvent {
    /// A press of `button` at `location`, held for `duration`.
    #[must_use]
    pub const fn new(location: Point, duration: Duration, button: PointerButton) -> Self {
        Self {
            location,
            duration,
            button,
        }
    }
}

/// Event payload delivered as the first argument to a [`DragGesture`] handler.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DragEvent {
    /// Phase of the drag gesture.
    phase: GesturePhase,
    /// Current location of the pointer.
    location: Point,
    /// Total translation since the drag started.
    translation: Point,
    /// Velocity of the drag in points per second.
    velocity: Point,
    /// The button that drags.
    button: PointerButton,
}

impl DragEvent {
    /// Phase of the drag gesture.
    #[must_use]
    pub const fn phase(&self) -> GesturePhase {
        self.phase
    }

    /// Current location of the pointer.
    #[must_use]
    pub const fn location(&self) -> Point {
        self.location
    }

    /// Sets the `location`.
    pub const fn set_location(&mut self, location: Point) {
        self.location = location;
    }

    /// Total translation since the drag started.
    #[must_use]
    pub const fn translation(&self) -> Point {
        self.translation
    }

    /// Sets the `translation`.
    pub const fn set_translation(&mut self, translation: Point) {
        self.translation = translation;
    }

    /// Velocity of the drag in points per second.
    #[must_use]
    pub const fn velocity(&self) -> Point {
        self.velocity
    }

    /// Sets the `velocity`.
    pub const fn set_velocity(&mut self, velocity: Point) {
        self.velocity = velocity;
    }

    /// The button that drags.
    #[must_use]
    pub const fn button(&self) -> PointerButton {
        self.button
    }
}

impl DragEvent {
    /// A drag of `button` in `phase`, now at `location`, moved by
    /// `translation` since it started, moving at `velocity`.
    #[must_use]
    pub const fn new(
        phase: GesturePhase,
        location: Point,
        translation: Point,
        velocity: Point,
        button: PointerButton,
    ) -> Self {
        Self {
            phase,
            location,
            translation,
            velocity,
            button,
        }
    }
}

/// Event payload delivered as the first argument to a [`MagnificationGesture`] handler.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MagnificationEvent {
    /// Phase of the magnification gesture.
    phase: GesturePhase,
    /// Focal point of the gesture.
    center: Point,
    /// Current scale factor relative to the gesture start.
    scale: f32,
    /// Rate of change of the scale factor.
    velocity: f32,
}

impl MagnificationEvent {
    /// Phase of the magnification gesture.
    #[must_use]
    pub const fn phase(&self) -> GesturePhase {
        self.phase
    }

    /// Focal point of the gesture.
    #[must_use]
    pub const fn center(&self) -> Point {
        self.center
    }

    /// Sets the `center`.
    pub const fn set_center(&mut self, center: Point) {
        self.center = center;
    }

    /// Current scale factor relative to the gesture start.
    #[must_use]
    pub const fn scale(&self) -> f32 {
        self.scale
    }

    /// Rate of change of the scale factor.
    #[must_use]
    pub const fn velocity(&self) -> f32 {
        self.velocity
    }
}

impl MagnificationEvent {
    /// A magnification in `phase` about `center`, at `scale` relative to the
    /// gesture start, changing at `velocity`.
    #[must_use]
    pub const fn new(phase: GesturePhase, center: Point, scale: f32, velocity: f32) -> Self {
        Self {
            phase,
            center,
            scale,
            velocity,
        }
    }
}

/// Payload describing an in-flight rotation gesture.
///
/// This payload accompanies [`Gesture::Rotation`] entries in the environment
/// when rotation gestures are recognised, mirroring [`MagnificationEvent`] for
/// the other two-finger transform gesture.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RotationEvent {
    /// Phase of the rotation gesture.
    phase: GesturePhase,
    /// Pivot point the rotation is measured around.
    center: Point,
    /// Current angle in radians relative to the gesture start.
    angle: f32,
    /// Rate of change of the angle, in radians per second.
    velocity: f32,
}

impl RotationEvent {
    /// Phase of the rotation gesture.
    #[must_use]
    pub const fn phase(&self) -> GesturePhase {
        self.phase
    }

    /// Pivot point the rotation is measured around.
    #[must_use]
    pub const fn center(&self) -> Point {
        self.center
    }

    /// Sets the `center`.
    pub const fn set_center(&mut self, center: Point) {
        self.center = center;
    }

    /// Current angle in radians relative to the gesture start.
    #[must_use]
    pub const fn angle(&self) -> f32 {
        self.angle
    }

    /// Rate of change of the angle, in radians per second.
    #[must_use]
    pub const fn velocity(&self) -> f32 {
        self.velocity
    }
}

impl RotationEvent {
    /// A rotation in `phase` about `center`, at `angle` radians from the
    /// gesture start, changing at `velocity` radians per second.
    #[must_use]
    pub const fn new(phase: GesturePhase, center: Point, angle: f32, velocity: f32) -> Self {
        Self {
            phase,
            center,
            angle,
            velocity,
        }
    }
}

/// Describes a tap interaction that must occur a specific number of times.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TapGesture {
    /// The number of consecutive taps required to trigger this gesture.
    count: NonZeroU32,
    /// The buttons that can tap; [`PointerButtons::PRIMARY`] by default.
    buttons: PointerButtons,
}

impl TapGesture {
    /// The number of consecutive taps required to trigger this gesture.
    #[must_use]
    pub const fn count(&self) -> NonZeroU32 {
        self.count
    }

    /// The buttons that can tap; [`PointerButtons::PRIMARY`] by default.
    #[must_use]
    pub const fn get_buttons(&self) -> PointerButtons {
        self.buttons
    }
}

impl TapGesture {
    /// Creates a tap gesture that requires `count` consecutive taps to activate.
    #[must_use]
    pub const fn repeat(count: NonZeroU32) -> Self {
        Self {
            count,
            buttons: PointerButtons::PRIMARY,
        }
    }

    /// Creates a tap gesture that requires a single tap to activate.
    #[must_use]
    pub const fn new() -> Self {
        Self::repeat(NonZeroU32::MIN)
    }

    /// Responds to `buttons` instead of the primary button alone — a middle
    /// click that closes a tab is `TapGesture::new().buttons(PointerButtons::MIDDLE)`.
    #[must_use]
    pub const fn buttons(mut self, buttons: PointerButtons) -> Self {
        self.buttons = buttons;
        self
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
    /// Explicit minimum duration before recognition; `None` uses the platform long-press timeout.
    minimum_duration: Option<Duration>,
    /// The buttons that can press; [`PointerButtons::PRIMARY`] by default.
    buttons: PointerButtons,
}

impl LongPressGesture {
    /// Explicit minimum duration before recognition, or `None` to follow the platform timeout.
    #[must_use]
    pub const fn get_minimum_duration(&self) -> Option<Duration> {
        self.minimum_duration
    }

    /// The buttons that can press; [`PointerButtons::PRIMARY`] by default.
    #[must_use]
    pub const fn get_buttons(&self) -> PointerButtons {
        self.buttons
    }
}

impl LongPressGesture {
    /// Creates a long-press gesture that uses the platform's default long-press timeout.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            minimum_duration: None,
            buttons: PointerButtons::PRIMARY,
        }
    }

    /// Sets an explicit minimum hold `duration` before the gesture activates.
    #[must_use]
    pub const fn minimum_duration(mut self, duration: Duration) -> Self {
        self.minimum_duration = Some(duration);
        self
    }

    /// Responds to `buttons` instead of the primary button alone.
    #[must_use]
    pub const fn buttons(mut self, buttons: PointerButtons) -> Self {
        self.buttons = buttons;
        self
    }
}

impl Default for LongPressGesture {
    fn default() -> Self {
        Self::new()
    }
}

/// Describes a drag interaction that begins after the pointer moves beyond a threshold.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DragGesture {
    /// The minimum distance the pointer must travel to initiate the drag.
    min_distance: f32,
    /// The buttons that can drag; [`PointerButtons::PRIMARY`] by default.
    buttons: PointerButtons,
}

impl DragGesture {
    /// The minimum distance the pointer must travel to initiate the drag.
    #[must_use]
    pub const fn min_distance(&self) -> f32 {
        self.min_distance
    }

    /// The buttons that can drag; [`PointerButtons::PRIMARY`] by default.
    #[must_use]
    pub const fn get_buttons(&self) -> PointerButtons {
        self.buttons
    }
}

impl DragGesture {
    /// Creates a drag gesture requiring the pointer to travel at least `min_distance` units.
    #[must_use]
    pub const fn new(min_distance: f32) -> Self {
        Self {
            min_distance,
            buttons: PointerButtons::PRIMARY,
        }
    }

    /// Responds to `buttons` instead of the primary button alone — a
    /// middle-button pan is `DragGesture::new(0.0).buttons(PointerButtons::MIDDLE)`.
    #[must_use]
    pub const fn buttons(mut self, buttons: PointerButtons) -> Self {
        self.buttons = buttons;
        self
    }
}

/// Describes a magnification (pinch/zoom) interaction starting from an initial scale factor.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MagnificationGesture {
    /// The initial scale factor when the gesture begins.
    initial_scale: f32,
}

impl MagnificationGesture {
    /// The initial scale factor when the gesture begins.
    #[must_use]
    pub const fn initial_scale(&self) -> f32 {
        self.initial_scale
    }
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
    initial_angle: f32,
}

impl RotationGesture {
    /// The initial angle (in radians) when the gesture begins.
    #[must_use]
    pub const fn initial_angle(&self) -> f32 {
        self.initial_angle
    }
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
    gesture: Gesture,
    /// The action to execute when the gesture is recognized.
    action: BoxedAction<()>,
}

impl GestureObserver {
    /// Takes the value apart into its fields, in declaration order.
    pub fn into_parts(self) -> (Gesture, BoxedAction<()>) {
        (self.gesture, self.action)
    }

    /// The gesture to observe.
    #[must_use]
    pub const fn gesture(&self) -> &Gesture {
        &self.gesture
    }

    /// The action to execute when the gesture is recognized.
    #[must_use]
    pub const fn action(&self) -> &BoxedAction<()> {
        &self.action
    }
}

impl fmt::Debug for GestureObserver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GestureObserver")
            .field("gesture", &self.gesture)
            .finish_non_exhaustive()
    }
}

impl MetadataKey for GestureObserver {}

/// Event payload emitted by a dynamic [`Gesture`] observer.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum GestureEvent {
    /// Tap gesture recognized.
    Tap(TapEvent),
    /// Long-press gesture recognized.
    LongPress(LongPressEvent),
    /// Drag gesture recognized.
    Drag(DragEvent),
    /// Magnification gesture recognized.
    Magnification(MagnificationEvent),
    /// Rotation gesture recognized.
    Rotation(RotationEvent),
}

/// Associates a gesture descriptor with the typed event payload delivered to
/// its handler.
pub trait GestureType: Into<Gesture> {
    /// Event payload passed as the first argument to handlers for this gesture.
    type Event: 'static;

    /// Extracts the backend-installed [`Self::Event`] from `env`.
    #[doc(hidden)]
    fn extract_event(env: &Environment) -> Result<Self::Event, ExtractionError>;
}

macro_rules! impl_gesture_type {
    ($gesture:ty => $event:ty) => {
        impl GestureType for $gesture {
            type Event = $event;

            fn extract_event(env: &Environment) -> Result<Self::Event, ExtractionError> {
                Handler::call(&mut |s: State<$event>| s, env).map(|State(event)| event)
            }
        }
    };
}

impl_gesture_type!(TapGesture => TapEvent);
impl_gesture_type!(LongPressGesture => LongPressEvent);
impl_gesture_type!(DragGesture => DragEvent);
impl_gesture_type!(MagnificationGesture => MagnificationEvent);
impl_gesture_type!(RotationGesture => RotationEvent);

impl GestureType for Gesture {
    type Event = GestureEvent;

    fn extract_event(env: &Environment) -> Result<Self::Event, ExtractionError> {
        if let Ok(State(event)) = Handler::call(&mut |s: State<TapEvent>| s, env) {
            return Ok(GestureEvent::Tap(event));
        }
        if let Ok(State(event)) = Handler::call(&mut |s: State<LongPressEvent>| s, env) {
            return Ok(GestureEvent::LongPress(event));
        }
        if let Ok(State(event)) = Handler::call(&mut |s: State<DragEvent>| s, env) {
            return Ok(GestureEvent::Drag(event));
        }
        if let Ok(State(event)) = Handler::call(&mut |s: State<MagnificationEvent>| s, env) {
            return Ok(GestureEvent::Magnification(event));
        }
        Handler::call(&mut |s: State<RotationEvent>| s, env)
            .map(|State(event)| GestureEvent::Rotation(event))
    }
}

impl GestureObserver {
    /// Creates a gesture observer for `gesture` whose handler consumes
    /// `G::Event` as its first argument, followed by environment extractors.
    ///
    /// Local state can be injected by wrapping the owning view with
    /// `.state(...)`.
    ///
    /// # Examples
    ///
    /// Simple action:
    /// ```
    /// use waterui_interaction::gesture::{GestureObserver, TapEvent, TapGesture};
    ///
    /// let observer = GestureObserver::new(TapGesture::new(), |_: TapEvent| {});
    /// ```
    ///
    /// With injected state:
    /// ```
    /// use core::num::NonZeroU32;
    /// use nami::Binding;
    /// use waterui_core::State;
    /// use waterui_interaction::gesture::{GestureObserver, TapEvent, TapGesture};
    ///
    /// let observer = GestureObserver::new(
    ///     TapGesture::repeat(NonZeroU32::new(2).unwrap()),
    ///     |_: TapEvent, State(counter): State<Binding<i32>>| *counter.get_mut() += 1,
    /// );
    /// ```
    /// Temporary Step 1 implementation keeping `waterui-map-gpu` and
    /// `waterui-video-gpu` compiling until they migrate to typed gesture args;
    /// Step 3 switches `new` to [`Self::from_event`]'s typed signature.
    #[must_use]
    pub fn new<Args>(gesture: impl Into<Gesture>, action: impl Handler<Args, ()>) -> Self {
        Self {
            gesture: gesture.into(),
            action: boxed_action(action),
        }
    }

    /// Creates a gesture observer for `gesture` whose handler consumes
    /// `G::Event` as its first argument, followed by environment extractors.
    #[doc(hidden)]
    #[must_use]
    pub fn from_event<G, H, Args>(gesture: G, action: H) -> Self
    where
        G: GestureType,
        H: EventHandler<G::Event, Args>,
    {
        let mut handler = boxed_event_handler(action);
        Self {
            gesture: gesture.into(),
            action: Box::new(move |env: &Environment| {
                let event = G::extract_event(env)?;
                handler(event, env)
            }),
        }
    }

    /// Creates a gesture observer whose handler ignores the recognized event
    /// payload (used by payload-free shortcuts such as `on_tap` and `on_long_press`).
    #[must_use]
    pub fn without_event<Args>(
        gesture: impl Into<Gesture>,
        action: impl Handler<Args, ()>,
    ) -> Self {
        Self {
            gesture: gesture.into(),
            action: boxed_action(action),
        }
    }
}

macro_rules! impl_state_extractor {
    ($($ty:ty),+ $(,)?) => {
        $(
            #[doc(hidden)]
            impl Extractor for $ty {
                fn extract(env: &Environment) -> Result<Self, Error> {
                    State::<Self>::extract(env).map(|State(value)| value)
                }

                fn extract_from_action(
                    env: &Environment,
                    state: &mut ExtractionState,
                ) -> Result<Self, Error> {
                    State::<Self>::extract_from_action(env, state).map(|State(value)| value)
                }
            }
        )+
    };
}

// Temporary Step 1 bridge for `water-rs/chart`, `water-rs/map-gpu`, and
// `water-rs/video-gpu` until Step 2 migrates them to typed `GestureObserver::new`.
impl_state_extractor!(
    TapEvent,
    LongPressEvent,
    DragEvent,
    MagnificationEvent,
    RotationEvent,
    Gesture,
);

#[doc(hidden)]
impl Extractor for GestureEvent {
    fn extract(env: &Environment) -> Result<Self, Error> {
        if let Ok(State(event)) = State::<TapEvent>::extract(env) {
            return Ok(Self::Tap(event));
        }
        if let Ok(State(event)) = State::<LongPressEvent>::extract(env) {
            return Ok(Self::LongPress(event));
        }
        if let Ok(State(event)) = State::<DragEvent>::extract(env) {
            return Ok(Self::Drag(event));
        }
        if let Ok(State(event)) = State::<MagnificationEvent>::extract(env) {
            return Ok(Self::Magnification(event));
        }
        State::<RotationEvent>::extract(env).map(|State(event)| Self::Rotation(event))
    }

    fn extract_from_action(env: &Environment, _state: &mut ExtractionState) -> Result<Self, Error> {
        Self::extract(env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_gesture_count_tracks_non_zero_repeat() {
        assert_eq!(TapGesture::new().count(), NonZeroU32::MIN);
        let double_tap = TapGesture::repeat(NonZeroU32::new(2).unwrap());
        assert_eq!(double_tap.count().get(), 2);
    }

    #[test]
    fn then_sequences_gestures() {
        let gesture = TapGesture::new()
            .then(LongPressGesture::new().minimum_duration(Duration::from_millis(300)));
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
        let gesture = TapGesture::new().exclusively_before(
            LongPressGesture::new().minimum_duration(Duration::from_millis(500)),
        );
        match gesture {
            Gesture::Exclusive(pair) => {
                assert!(matches!(pair.first(), Gesture::Tap(_)));
                assert!(matches!(pair.second(), Gesture::LongPress(_)));
            }
            other => panic!("expected Gesture::Exclusive, got {other:?}"),
        }
    }

    #[test]
    fn typed_and_composite_gesture_observers_deliver_expected_events() {
        use alloc::{rc::Rc, vec, vec::Vec};
        use core::cell::RefCell;

        let tap_loc = Rc::new(RefCell::new(None));
        let tap_observer = GestureObserver::new(TapGesture::new(), {
            let tap_loc = Rc::clone(&tap_loc);
            move |tap: TapEvent| *tap_loc.borrow_mut() = Some(tap.location())
        });
        let (_gesture, mut tap_action) = tap_observer.into_parts();
        let tap_env = Environment::new().extending(State(TapEvent::new(
            Point::new(10.0, 20.0),
            1,
            PointerButton::Primary,
        )));
        assert!(tap_action(&tap_env).is_ok());
        assert_eq!(*tap_loc.borrow(), Some(Point::new(10.0, 20.0)));

        let composite_events = Rc::new(RefCell::new(Vec::new()));
        let composite_observer = GestureObserver::new(Gesture::from(TapGesture::new()), {
            let composite_events = Rc::clone(&composite_events);
            move |event: GestureEvent| composite_events.borrow_mut().push(event)
        });
        let (_gesture, mut composite_action) = composite_observer.into_parts();
        assert!(composite_action(&tap_env).is_ok());
        assert_eq!(
            *composite_events.borrow(),
            vec![GestureEvent::Tap(TapEvent::new(
                Point::new(10.0, 20.0),
                1,
                PointerButton::Primary,
            ))]
        );
    }
}
