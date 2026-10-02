//! Structural view transitions and the retained removal contract.
//!
//! Progress measures absence: zero is the presented view and one is its absent
//! state. Insertion traverses the same effect backwards. Backends own semantic
//! membership; a [`Ghost`] is never an accessibility, input, focus, or testing
//! node. It owns only the outgoing realization and its retained layout slot.

use alloc::rc::Rc;
use core::{fmt, time::Duration};

use crate::animation::Animation;
use crate::layout::{
    LayoutDirection, ProposalSize, Rect, Size, StretchAxis, SubView, ViewDimensions,
};
use crate::metadata::MetadataKey;
use crate::{AnyView, Binding, Computed, Metadata, Signal, SignalExt, View};

/// The structural event starting an independent transition instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionPhase {
    /// New semantic content, whose slot is allocated immediately.
    Insertion,
    /// Removed semantic content, whose visual and slot remain until completion.
    Removal,
}

/// Logical edge used by movement and pixel sweep transitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// Upper edge.
    Top,
    /// Lower edge.
    Bottom,
    /// Left in left-to-right layouts, right in right-to-left layouts.
    Leading,
    /// Right in left-to-right layouts, left in right-to-left layouts.
    Trailing,
}

impl Edge {
    /// Unit direction in physical top-left coordinates.
    #[must_use]
    pub const fn vector(self, direction: LayoutDirection) -> [f32; 2] {
        match (self, direction) {
            (Self::Top, _) => [0.0, -1.0],
            (Self::Bottom, _) => [0.0, 1.0],
            (Self::Leading, LayoutDirection::LeftToRight)
            | (Self::Trailing, LayoutDirection::RightToLeft) => [-1.0, 0.0],
            (Self::Trailing, LayoutDirection::LeftToRight)
            | (Self::Leading, LayoutDirection::RightToLeft) => [1.0, 0.0],
        }
    }
}

crate::impl_constant!(Edge);

/// Geometry available when resolving native property transitions.
#[derive(Clone, Copy, Debug)]
pub struct TransitionGeometry {
    /// The untransformed layout slot, in logical pixels.
    pub size: Size,
    /// Resolves leading and trailing edges.
    pub direction: LayoutDirection,
}

/// Window-relative geometry a pixel transition needs to draw beyond its slot.
///
/// A pixel effect emits outside the input bounds and must follow scrolling,
/// so progress alone cannot describe it: the host updates this frame after
/// every placement and before drawing through the window-level overlay.
#[derive(Clone, Copy, Debug)]
pub struct TransitionFrame {
    /// The retained slot in window logical coordinates.
    pub source: Rect,
    /// The window's full overlay size in logical pixels.
    pub window: Size,
    /// Device pixels per logical pixel.
    pub scale_factor: f32,
    /// Resolves leading and trailing edges.
    pub direction: LayoutDirection,
}

crate::impl_constant!(TransitionFrame);

/// A sampled set of visual properties, independent of layout.
///
/// Translation is in logical pixels, rotation in radians, and blur in logical
/// pixels. Scale and rotation apply about the slot's centre, then translation.
/// These values compose with the view's ordinary properties rather than
/// replacing them. No property changes the retained slot's dimensions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionProperties {
    /// Multiplicative alpha.
    pub opacity: f32,
    /// Multiplicative horizontal and vertical scale.
    pub scale: [f32; 2],
    /// Physical horizontal and vertical displacement.
    pub translation: [f32; 2],
    /// Clockwise rotation in top-left coordinates.
    pub rotation: f32,
    /// Gaussian blur radius.
    pub blur: f32,
}

impl TransitionProperties {
    /// Presented view with no transition adjustment.
    pub const IDENTITY: Self = Self {
        opacity: 1.0,
        scale: [1.0, 1.0],
        translation: [0.0, 0.0],
        rotation: 0.0,
        blur: 0.0,
    };

    fn combined(self, other: Self) -> Self {
        Self {
            opacity: self.opacity * other.opacity,
            scale: [
                self.scale[0] * other.scale[0],
                self.scale[1] * other.scale[1],
            ],
            translation: [
                self.translation[0] + other.translation[0],
                self.translation[1] + other.translation[1],
            ],
            rotation: self.rotation + other.rotation,
            blur: self.blur + other.blur,
        }
    }
}

impl Default for TransitionProperties {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Reactive input to a pixel transition's one-time view construction.
#[derive(Clone, Debug)]
pub struct TransitionContext {
    /// Event selecting the insertion or removal effect.
    pub phase: TransitionPhase,
    /// Absence in `[0, 1]`; subscribe instead of rebuilding the effect each frame.
    pub progress: Computed<f32>,
    /// Window-relative geometry, host-updated per placement including scroll/resize.
    pub frame: Computed<TransitionFrame>,
}

/// A structural view effect.
///
/// Pixel implementations return a `waterui_graphics::FilteredView` built on a
/// `filtrate::Effect` from `body` and declare `captures_content()`. Property implementations return sampled
/// native properties. `body` is constructed once per instance; progress updates
/// the effect precisely through its signal. Public implementations never need
/// object-safe signatures or manual boxing.
pub trait Transition: 'static {
    /// Samples property adjustments at an absence value in `[0, 1]`.
    fn properties(
        &self,
        _phase: TransitionPhase,
        _progress: f32,
        _geometry: TransitionGeometry,
    ) -> TransitionProperties {
        TransitionProperties::IDENTITY
    }

    /// Wraps content in the existing GPU effect path when capture is declared.
    fn body(&self, content: AnyView, transition_context: TransitionContext) -> impl View {
        let _ = transition_context;
        content
    }

    /// Whether this phase requires a captured content texture.
    ///
    /// Removal captures exactly once, freezes that texture and draws through a
    /// window-level overlay. Insertion uses live capture through the ordinary
    /// view-effect path. A failed capture is an error, never a property fallback.
    fn captures_content(&self, _phase: TransitionPhase) -> bool {
        false
    }

    /// Declared Reduce Motion mapping; opacity is the default.
    fn reduced(&self) -> impl Transition {
        PropertyTransition::opacity()
    }

    /// Applies both effects over the same progress signal.
    fn combined<T: Transition>(self, other: T) -> Combined<Self, T>
    where
        Self: Sized,
    {
        Combined {
            first: self,
            second: other,
        }
    }
}

trait TransitionImpl {
    fn properties_erased(
        &self,
        phase: TransitionPhase,
        progress: f32,
        geometry: TransitionGeometry,
    ) -> TransitionProperties;
    fn body_erased(&self, content: AnyView, transition_context: TransitionContext) -> AnyView;
    fn captures_content_erased(&self, phase: TransitionPhase) -> bool;
    fn reduced_erased(&self) -> AnyTransition;
}

impl<T: Transition> TransitionImpl for T {
    fn properties_erased(
        &self,
        phase: TransitionPhase,
        progress: f32,
        geometry: TransitionGeometry,
    ) -> TransitionProperties {
        Transition::properties(self, phase, progress, geometry)
    }
    fn body_erased(&self, content: AnyView, transition_context: TransitionContext) -> AnyView {
        AnyView::new(Transition::body(self, content, transition_context))
    }
    fn captures_content_erased(&self, phase: TransitionPhase) -> bool {
        Transition::captures_content(self, phase)
    }
    fn reduced_erased(&self) -> AnyTransition {
        AnyTransition::new(Transition::reduced(self))
    }
}

/// Shared immutable transition declaration behind a private dispatch shim.
///
/// Cloning this declaration never shares an animation clock or a ghost. Each
/// call to [`TransitionSpec::start`] creates a fresh instance.
#[derive(Clone)]
pub struct AnyTransition(Rc<dyn TransitionImpl>);

impl fmt::Debug for AnyTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnyTransition").finish_non_exhaustive()
    }
}

impl AnyTransition {
    /// Erases a concrete transition at the backend boundary.
    pub fn new(transition: impl Transition) -> Self {
        Self(Rc::new(transition))
    }

    /// Uses different effects for insertion and removal.
    pub const fn asymmetric<I: Transition, R: Transition>(
        insertion: I,
        removal: R,
    ) -> Asymmetric<I, R> {
        Asymmetric { insertion, removal }
    }
}

impl Transition for AnyTransition {
    fn properties(
        &self,
        phase: TransitionPhase,
        progress: f32,
        geometry: TransitionGeometry,
    ) -> TransitionProperties {
        self.0.properties_erased(phase, progress, geometry)
    }
    fn body(&self, content: AnyView, transition_context: TransitionContext) -> impl View {
        self.0.body_erased(content, transition_context)
    }
    fn captures_content(&self, phase: TransitionPhase) -> bool {
        self.0.captures_content_erased(phase)
    }
    fn reduced(&self) -> impl Transition {
        self.0.reduced_erased()
    }
}

/// One native property transition, evaluated without capturing pixels.
#[derive(Clone, Copy, Debug)]
pub struct PropertyTransition {
    absent: TransitionProperties,
    edge: Option<Edge>,
}

impl PropertyTransition {
    /// Constructs a property effect from its absent endpoint.
    #[must_use]
    pub const fn new(absent: TransitionProperties) -> Self {
        Self { absent, edge: None }
    }

    /// Fades between transparent and the view's normal alpha.
    #[must_use]
    pub const fn opacity() -> Self {
        Self::new(TransitionProperties {
            opacity: 0.0,
            ..TransitionProperties::IDENTITY
        })
    }

    /// Scales uniformly about the slot centre.
    #[must_use]
    pub const fn scale(absent_scale: f32) -> Self {
        Self::new(TransitionProperties {
            scale: [absent_scale, absent_scale],
            ..TransitionProperties::IDENTITY
        })
    }

    /// Moves one slot extent toward a logical edge when absent.
    #[must_use]
    pub const fn move_in(edge: Edge) -> Self {
        Self {
            absent: TransitionProperties::IDENTITY,
            edge: Some(edge),
        }
    }

    /// Slides in from leading and out toward trailing.
    #[must_use]
    pub const fn slide() -> Asymmetric<Self, Self> {
        AnyTransition::asymmetric(Self::move_in(Edge::Leading), Self::move_in(Edge::Trailing))
    }

    /// Blurs the absent view by a logical-pixel radius.
    #[must_use]
    pub const fn blur(radius: f32) -> Self {
        Self::new(TransitionProperties {
            blur: radius,
            ..TransitionProperties::IDENTITY
        })
    }

    /// Rotates the absent view clockwise by radians.
    #[must_use]
    pub const fn rotation(radians: f32) -> Self {
        Self::new(TransitionProperties {
            rotation: radians,
            ..TransitionProperties::IDENTITY
        })
    }
}

impl Transition for PropertyTransition {
    fn properties(
        &self,
        _phase: TransitionPhase,
        progress: f32,
        geometry: TransitionGeometry,
    ) -> TransitionProperties {
        let movement = self
            .edge
            .map_or([0.0, 0.0], |edge| edge.vector(geometry.direction));
        TransitionProperties {
            opacity: (self.absent.opacity - 1.0).mul_add(progress, 1.0),
            scale: [
                (self.absent.scale[0] - 1.0).mul_add(progress, 1.0),
                (self.absent.scale[1] - 1.0).mul_add(progress, 1.0),
            ],
            translation: [
                movement[0].mul_add(geometry.size.width, self.absent.translation[0]) * progress,
                movement[1].mul_add(geometry.size.height, self.absent.translation[1]) * progress,
            ],
            rotation: self.absent.rotation * progress,
            blur: self.absent.blur * progress,
        }
    }
}

/// Two concrete transitions sharing one structural lifetime.
#[derive(Clone, Debug)]
pub struct Combined<A, B> {
    first: A,
    second: B,
}

impl<A: Transition, B: Transition> Transition for Combined<A, B> {
    fn properties(
        &self,
        phase: TransitionPhase,
        progress: f32,
        geometry: TransitionGeometry,
    ) -> TransitionProperties {
        self.first
            .properties(phase, progress, geometry)
            .combined(self.second.properties(phase, progress, geometry))
    }
    fn body(&self, content: AnyView, transition_context: TransitionContext) -> impl View {
        self.second.body(
            AnyView::new(self.first.body(content, transition_context.clone())),
            transition_context,
        )
    }
    fn captures_content(&self, phase: TransitionPhase) -> bool {
        self.first.captures_content(phase) || self.second.captures_content(phase)
    }
    fn reduced(&self) -> impl Transition {
        self.first.reduced().combined(self.second.reduced())
    }
}

/// Separate concrete declarations for entering and exiting content.
#[derive(Clone, Debug)]
pub struct Asymmetric<I, R> {
    insertion: I,
    removal: R,
}

impl<I: Transition, R: Transition> Transition for Asymmetric<I, R> {
    fn properties(
        &self,
        phase: TransitionPhase,
        progress: f32,
        geometry: TransitionGeometry,
    ) -> TransitionProperties {
        match phase {
            TransitionPhase::Insertion => self.insertion.properties(phase, progress, geometry),
            TransitionPhase::Removal => self.removal.properties(phase, progress, geometry),
        }
    }
    fn body(&self, content: AnyView, transition_context: TransitionContext) -> impl View {
        match transition_context.phase {
            TransitionPhase::Insertion => {
                AnyView::new(self.insertion.body(content, transition_context))
            }
            TransitionPhase::Removal => {
                AnyView::new(self.removal.body(content, transition_context))
            }
        }
    }
    fn captures_content(&self, phase: TransitionPhase) -> bool {
        match phase {
            TransitionPhase::Insertion => self.insertion.captures_content(phase),
            TransitionPhase::Removal => self.removal.captures_content(phase),
        }
    }
    fn reduced(&self) -> impl Transition {
        AnyTransition::asymmetric(self.insertion.reduced(), self.removal.reduced())
    }
}

/// Strict metadata attached by [`View::transition`].
#[derive(Clone, Debug)]
pub struct TransitionSpec {
    /// Reusable effect declaration.
    pub transition: AnyTransition,
    /// Timing shared by all combined effects.
    pub animation: Animation,
}

impl MetadataKey for TransitionSpec {}

impl TransitionSpec {
    /// Creates a declaration using the default animation.
    pub fn new(transition: impl Transition) -> Self {
        Self {
            transition: AnyTransition::new(transition),
            animation: Animation::default(),
        }
    }

    /// Starts a new clock, resolving Reduce Motion before constructing effects.
    #[must_use]
    pub fn start(&self, phase: TransitionPhase, reduce_motion: bool) -> TransitionState {
        let transition = if reduce_motion {
            self.transition.0.reduced_erased()
        } else {
            self.transition.clone()
        };
        let progress = Binding::f32(match phase {
            TransitionPhase::Insertion => 1.0,
            TransitionPhase::Removal => 0.0,
        });
        let mut state = TransitionState {
            transition,
            animation: self.animation.clone(),
            phase,
            elapsed: Duration::ZERO,
            progress,
        };
        state.advance(Duration::ZERO);
        state
    }
}

impl Metadata<TransitionSpec> {
    /// Sets insertion and removal timing without rebuilding reactive content.
    pub const fn animation(mut self, animation: Animation) -> Self {
        self.value.animation = animation;
        self
    }
}

/// An independent frame-driven transition lifetime.
#[derive(Debug)]
pub struct TransitionState {
    transition: AnyTransition,
    animation: Animation,
    phase: TransitionPhase,
    elapsed: Duration,
    progress: Binding<f32>,
}

impl TransitionState {
    /// Advances by the host frame delta; returns whether another frame is needed.
    pub fn advance(&mut self, delta: Duration) -> bool {
        self.elapsed = self.elapsed.saturating_add(delta);
        let completed = self.is_complete();
        let eased = if completed {
            1.0
        } else {
            self.animation.progress(self.elapsed).clamp(0.0, 1.0)
        };
        self.progress.set(match self.phase {
            TransitionPhase::Insertion => 1.0 - eased,
            TransitionPhase::Removal => eased,
        });
        !completed
    }

    /// True only when the animation lifetime ends, including zero-duration timing.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.animation.is_complete(self.elapsed)
    }

    /// Current absence value.
    #[must_use]
    pub fn progress(&self) -> f32 {
        self.progress.snapshot()
    }

    /// Samples the property descriptor for the slot's current geometry.
    #[must_use]
    pub fn properties(&self, geometry: TransitionGeometry) -> TransitionProperties {
        self.transition
            .0
            .properties_erased(self.phase, self.progress(), geometry)
    }

    /// Samples the absent endpoint for a native property animator.
    #[must_use]
    pub fn absent_properties(&self, geometry: TransitionGeometry) -> TransitionProperties {
        self.transition
            .0
            .properties_erased(self.phase, 1.0, geometry)
    }

    /// Whether the realization needs a texture for this phase.
    #[must_use]
    pub fn captures_content(&self) -> bool {
        self.transition.0.captures_content_erased(self.phase)
    }

    /// Builds the effect once, retaining its progress signal until completion.
    ///
    /// A backend may pass an empty content view when it already owns the captured
    /// realization, then give the resulting `FilteredView` that GPU texture.
    pub fn body(&self, content: AnyView, frame: Computed<TransitionFrame>) -> AnyView {
        self.transition.0.body_erased(
            content,
            TransitionContext {
                phase: self.phase,
                progress: self.progress.clone().computed(),
                frame,
            },
        )
    }

    /// Animation definition for native property projection.
    #[must_use]
    pub const fn animation(&self) -> &Animation {
        &self.animation
    }
}

/// Frozen geometry of a removed view, kept in its original container.
///
/// Every proposal returns the last dimensions, including alignment guides. This
/// slot remains a layout member even though its semantic node has been removed.
/// Its global origin is recalculated by normal placement on every frame, so a
/// window overlay follows scrolling and ancestor movement without clipping.
#[derive(Clone, Debug)]
pub struct GhostSlot {
    dimensions: ViewDimensions,
    priority: i32,
}

impl GhostSlot {
    /// Retains the dimensions and priority of the outgoing realization.
    #[must_use]
    pub const fn new(dimensions: ViewDimensions, priority: i32) -> Self {
        Self {
            dimensions,
            priority,
        }
    }

    /// Fixed logical size at removal.
    #[must_use]
    pub const fn size(&self) -> Size {
        self.dimensions.size
    }
}

impl SubView for GhostSlot {
    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        self.dimensions.clone()
    }
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
    fn priority(&self) -> i32 {
        self.priority
    }
}

/// Ownership of an outgoing visual after its semantic node has been detached.
///
/// Backends remove accessibility, testing, hit-test and focus membership before
/// constructing this value. The realization must not receive semantic updates.
/// Drop the ghost and release its slot on completion, invalidating its container
/// with the container's animation. Re-insertion always constructs a new node;
/// it never revives a ghost, even when the collection identity is reused.
#[derive(Debug)]
pub struct Ghost<T> {
    /// Backend-owned non-interactive visual or frozen GPU capture.
    pub realization: T,
    /// Fixed-size leaf in the original parent's layout membership.
    pub slot: GhostSlot,
    /// Independent removal clock.
    pub transition: TransitionState,
}

impl<T> Ghost<T> {
    /// Starts retention after semantic detachment.
    pub fn new(
        realization: T,
        slot: GhostSlot,
        spec: &TransitionSpec,
        reduce_motion: bool,
    ) -> Self {
        Self {
            realization,
            slot,
            transition: spec.start(TransitionPhase::Removal, reduce_motion),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear(transition: impl Transition) -> TransitionSpec {
        TransitionSpec {
            transition: AnyTransition::new(transition),
            animation: Animation::linear(Duration::from_millis(100)),
        }
    }

    fn geometry(direction: LayoutDirection) -> TransitionGeometry {
        TransitionGeometry {
            size: Size::new(300.0, 60.0),
            direction,
        }
    }

    fn near(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
    }

    #[test]
    fn reinserted_instance_does_not_reverse_the_outgoing_clock() {
        let spec = linear(PropertyTransition::opacity());
        let mut outgoing = Ghost::new(
            (),
            GhostSlot::new(ViewDimensions::new(Size::new(300.0, 60.0)), 2),
            &spec,
            false,
        );
        assert!(outgoing.transition.advance(Duration::from_millis(40)));
        let mut incoming = spec.start(TransitionPhase::Insertion, false);
        assert!(incoming.advance(Duration::from_millis(20)));
        near(outgoing.transition.progress(), 0.4);
        near(incoming.progress(), 0.8);
        for proposal in [
            ProposalSize::new(None, None),
            ProposalSize::new(Some(0.0), Some(0.0)),
            ProposalSize::new(Some(f32::INFINITY), Some(f32::INFINITY)),
        ] {
            assert_eq!(outgoing.slot.measure(proposal).size, Size::new(300.0, 60.0));
        }
        assert_eq!(outgoing.slot.stretch_axis(), StretchAxis::None);
        assert!(!outgoing.slot.is_empty());
        assert!(!outgoing.transition.advance(Duration::from_millis(60)));
        assert!(!incoming.is_complete());
        near(outgoing.transition.progress(), 1.0);
    }

    #[test]
    fn asymmetric_move_resolves_logical_edges_and_plays_backwards() {
        let spec = linear(AnyTransition::asymmetric(
            PropertyTransition::move_in(Edge::Leading),
            PropertyTransition::move_in(Edge::Bottom),
        ));
        let mut incoming = spec.start(TransitionPhase::Insertion, false);
        incoming.advance(Duration::from_millis(50));
        near(
            incoming
                .properties(geometry(LayoutDirection::RightToLeft))
                .translation[0],
            150.0,
        );
        near(
            incoming
                .properties(geometry(LayoutDirection::LeftToRight))
                .translation[0],
            -150.0,
        );
        let mut outgoing = spec.start(TransitionPhase::Removal, false);
        outgoing.advance(Duration::from_millis(50));
        near(
            outgoing
                .properties(geometry(LayoutDirection::LeftToRight))
                .translation[1],
            30.0,
        );
    }

    #[test]
    fn combined_properties_share_timing_and_reduce_motion_is_declared() {
        let spec =
            linear(PropertyTransition::scale(0.5).combined(PropertyTransition::rotation(2.0)));
        let mut state = spec.start(TransitionPhase::Removal, false);
        state.advance(Duration::from_millis(50));
        let properties = state.properties(geometry(LayoutDirection::LeftToRight));
        near(properties.scale[0], 0.75);
        near(properties.rotation, 1.0);
        let mut reduced = spec.start(TransitionPhase::Removal, true);
        reduced.advance(Duration::from_millis(50));
        let properties = reduced.properties(geometry(LayoutDirection::LeftToRight));
        near(properties.scale[0], 1.0);
        near(properties.rotation, 0.0);
        near(properties.opacity, 0.25);
        assert!(!reduced.captures_content());
    }

    #[test]
    fn zero_duration_finishes_without_requiring_a_frame() {
        let mut spec = linear(PropertyTransition::opacity());
        spec.animation = Animation::linear(Duration::ZERO);
        let incoming = spec.start(TransitionPhase::Insertion, false);
        let outgoing = spec.start(TransitionPhase::Removal, false);
        assert!(incoming.is_complete());
        assert!(outgoing.is_complete());
        near(incoming.progress(), 0.0);
        near(outgoing.progress(), 1.0);
    }
}
