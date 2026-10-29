//! Structural transition ownership in the retained tree (layout-spec.md §4.5).
//!
//! A `.transition(...)` declaration is built into a transparent
//! [`WrapperEffect::Transition`] node carrying a [`TransitionDeclaration`].
//! The structural site that swaps the content — a `Dynamic` host, a
//! collection entry, a lazy-stack row, an anchored-overlay presentation —
//! claims it from the root of the *built* instance ([`claim_transition`]).
//! Type erasure is gone by then, so a type-erased row is owned exactly like
//! a concrete one; no other node is looked through. A declaration must be
//! the outermost modifier of the content a site swaps: one beneath a layout
//! or paint modifier is never claimed, so no site inspects modifier or
//! parent types. A declaration placed directly over a conditional is
//! adopted by that `Dynamic` host ([`adopt_by_dynamic`]) and applies to each
//! content instance it swaps in and out. A declaration nobody owns fails at
//! the first pass that reaches it, naming the fix.
//!
//! Every site drives its instances through the same pieces: a
//! [`TransitionClock`] per instance, a [`TransitionRuntime`] holding that
//! clock plus the blur realization, and [`flush_transitioned`], which paints
//! one instance under its sampled [`TransitionProperties`].

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use std::rc::Rc;

use waterui_core::transition::{
    AnyTransition, Ghost, GhostSlot, Transition, TransitionGeometry, TransitionPhase,
    TransitionProperties, TransitionSpec, TransitionState,
};
use waterui_graphics::{AnyEffect, ParamGuards};

use crate::renderer::effects::FilteredRuntime;

/// A `.transition(...)` declaration as the retained tree carries it: the
/// declared spec, and whether a structural site took ownership of it.
pub struct TransitionDeclaration {
    spec: TransitionSpec,
    owned: Cell<bool>,
}

impl TransitionDeclaration {
    pub const fn new(spec: TransitionSpec) -> Self {
        Self {
            spec,
            owned: Cell::new(false),
        }
    }

    /// Hands the spec to the structural site that owns this declaration.
    fn take_ownership(&self) -> TransitionSpec {
        self.owned.set(true);
        self.spec.clone()
    }

    /// Fails when no structural site claimed this declaration: a transition
    /// with no owner has no clock and no honest rendering path.
    pub fn assert_owned(&self) {
        assert!(
            self.owned.get(),
            "Hydrolysis: `.transition(...)` reached a view with no structural transition owner. \
             A transition declares how a view is inserted and removed, so it must be the \
             outermost modifier of the content a structural site swaps — of a `when`/`watch` \
             branch or the conditional itself, a `ForEach`/`List` row, a navigation page's \
             content, or an anchored-overlay body. A declaration beneath another modifier \
             (`.padding`, `.frame`, `.background`, `.alignment_guide`, …), a stack, a scroll \
             or a widget is never claimed: move `.transition(...)` outward so it is the \
             outermost modifier of that content (layout-spec.md §4.5)"
        );
    }
}

/// The declaration a node carries when the node itself is a
/// `.transition(...)`.
fn declaration<E: crate::text::TextEngine>(node: &RenderNode<E>) -> Option<&TransitionDeclaration> {
    match node {
        RenderNode::Wrapper(wrapper) => match &wrapper.effect {
            WrapperEffect::Transition(declaration) => Some(declaration),
            _ => None,
        },
        _ => None,
    }
}

/// Claims the transition declared at a structural instance's root. Only the
/// root is looked at: a declaration beneath any other node stays unowned and
/// fails with [`TransitionDeclaration::assert_owned`]'s message.
pub fn claim_transition<E: crate::text::TextEngine>(
    node: &RenderNode<E>,
) -> Option<TransitionSpec> {
    declaration(node).map(TransitionDeclaration::take_ownership)
}

/// Claims the transition declared on a navigation page's root: either on the
/// page itself, or — the page being a navigation view — on the root of its
/// content ([`WidgetBehavior::claim_page_transition`]). Only the stack calls
/// this — a navigation view anywhere else is not a structural instance, so a
/// declaration on its content stays unowned.
pub fn claim_page_transition<E: crate::text::TextEngine>(
    page: &RenderNode<E>,
) -> Option<TransitionSpec> {
    match page {
        RenderNode::Widget(widget) => widget.behavior.claim_page_transition(),
        _ => claim_transition(page),
    }
}

/// Hands a declaration built directly over a conditional to that `Dynamic`
/// host: its content instances transition with it whenever the host swaps
/// them. Only a conditional that is the declaration's direct child adopts
/// it; anything else leaves the declaration unowned. A watched conditional
/// is built as the host under the `Retain` node holding its watcher guard —
/// both are the conditional itself, not a modifier over it.
pub fn adopt_by_dynamic<E: crate::text::TextEngine>(
    child: &RenderNode<E>,
    declaration: &TransitionDeclaration,
) {
    let host = match child {
        RenderNode::Dynamic(host) => host,
        RenderNode::Retain(retain) => match &retain.child {
            RenderNode::Dynamic(host) => host,
            _ => return,
        },
        _ => return,
    };
    host.adopt(declaration.take_ownership());
}

/// Fails fast on a transition whose resolved form still needs pixel capture:
/// capture-based effects are realized by the transition overlay, which is not
/// implemented in Hydrolysis yet. The Reduce Motion form is resolved first, so
/// a capture effect whose declared reduction is a property effect still runs.
pub fn checked_transition(state: TransitionState) -> TransitionState {
    assert_property_transition(state.captures_content());
    state
}

/// The capture rejection [`checked_transition`] applies, for a transition
/// resolved without a clock.
fn assert_property_transition(captures_content: bool) {
    assert!(
        !captures_content,
        "Hydrolysis: pixel-capture transitions (a `Transition` whose `captures_content` \
         is true, such as disintegrate or dissolve) are not realized yet — their \
         window-level capture overlay is water-rs/waterui#1388. Use a property transition \
         (opacity, scale, move, blur, rotation) or declare a property `reduced()` form \
         and enable Reduce Motion"
    );
}

/// Starts a removal ghost after its semantic detachment, rejecting a
/// transition that still needs pixel capture.
pub fn start_ghost<T>(
    realization: T,
    slot: GhostSlot,
    spec: &TransitionSpec,
    reduce_motion: bool,
) -> Ghost<T> {
    let Ghost {
        realization,
        slot,
        transition,
    } = Ghost::new(realization, slot, spec, reduce_motion);
    Ghost {
        realization,
        slot,
        transition: checked_transition(transition),
    }
}

/// A slot's context with its bounds moved to the origin, and the placement
/// delta that puts it back: what [`flush_transitioned`] scopes an instance
/// that is already placed at `ctx`.
pub fn slot_context(ctx: RenderContext) -> (RenderContext, kurbo::Affine) {
    let delta = kurbo::Affine::translate((ctx.bounds().x0, ctx.bounds().y0));
    (
        RenderContext::new(
            ctx.local() * delta,
            kurbo::Rect::new(0.0, 0.0, ctx.bounds().width(), ctx.bounds().height()),
        ),
        delta,
    )
}

/// The frame instant a per-instance clock last advanced at — owned by the
/// clock itself, so an idle stretch between site frames never leaks into a
/// fresh clock's first delta.
pub struct TransitionClock {
    last_advanced_at: Instant,
}

impl TransitionClock {
    pub const fn new(now: Instant) -> Self {
        Self {
            last_advanced_at: now,
        }
    }

    /// Advances `state` by the time elapsed since this clock's last sample.
    pub fn advance(&mut self, state: &mut TransitionState, now: Instant) -> bool {
        let delta = now.saturating_duration_since(self.last_advanced_at);
        self.last_advanced_at = now;
        state.advance(delta)
    }
}

/// The blur realization of one transition instance: a filter layer host
/// wrapping the instance's node, whose engine `Blur` reads `radius` live.
struct TransitionBlur<E: crate::text::TextEngine> {
    core: NodeCore<E>,
    radius: nami::Binding<f32>,
    runtime: Rc<RefCell<FilteredRuntime>>,
}

/// The per-instance machinery a structural site keeps beside a
/// [`TransitionState`]: its clock, the identity its paint scope is keyed by,
/// and — created on the first frame that samples a nonzero blur radius — the
/// blur filter host.
pub struct TransitionRuntime<E: crate::text::TextEngine> {
    clock: TransitionClock,
    /// Identity of this instance's paint scope: unique among every live
    /// instance, so two sites recording into one program never share a key.
    marker: Rc<()>,
    blur: RefCell<Option<TransitionBlur<E>>>,
}

impl<E: crate::text::TextEngine> TransitionRuntime<E> {
    pub fn new(now: Instant) -> Self {
        Self {
            clock: TransitionClock::new(now),
            marker: Rc::new(()),
            blur: RefCell::new(None),
        }
    }

    /// The paint-scope key item of this instance.
    fn scope_item(&self) -> u64 {
        u64::try_from(Rc::as_ptr(&self.marker).addr())
            .expect("hydrolysis transition: a marker address fits u64")
    }

    /// Advances `state` on this instance's own clock; returns whether another
    /// frame is needed.
    pub fn advance(&mut self, state: &mut TransitionState, now: Instant) -> bool {
        self.clock.advance(state, now)
    }
}

/// One instance's transition in flight: the state and the runtime that drives
/// it, so a state cannot exist without the clock that advances it.
pub struct TransitionRun<E: crate::text::TextEngine> {
    pub state: TransitionState,
    pub runtime: TransitionRuntime<E>,
}

impl<E: crate::text::TextEngine> TransitionRun<E> {
    /// Starts the insertion of a new instance — timestamped at the frame
    /// instant of the structural change, never a previous frame.
    pub fn insertion(spec: &TransitionSpec, reduce_motion: bool, now: Instant) -> Self {
        Self::start(spec, TransitionPhase::Insertion, reduce_motion, now)
    }

    /// Starts the removal of an instance that left its site at `now`.
    pub fn removal(spec: &TransitionSpec, reduce_motion: bool, now: Instant) -> Self {
        Self::start(spec, TransitionPhase::Removal, reduce_motion, now)
    }

    fn start(
        spec: &TransitionSpec,
        phase: TransitionPhase,
        reduce_motion: bool,
        now: Instant,
    ) -> Self {
        Self {
            state: checked_transition(spec.start(phase, reduce_motion)),
            runtime: TransitionRuntime::new(now),
        }
    }

    /// Advances the clock; `false` once the transition settled.
    pub fn advance(&mut self, now: Instant) -> bool {
        self.runtime.advance(&mut self.state, now)
    }
}

/// A removal whose absence an interactive gesture drives instead of a clock:
/// sampled at whatever progress the gesture reports, moving back toward
/// presence when the gesture is cancelled. Reduce Motion and the capture
/// rejection resolve exactly as [`TransitionSpec::start`] and
/// [`checked_transition`] resolve them for a clock-driven removal.
pub struct ScrubbedRemoval<E: crate::text::TextEngine> {
    transition: AnyTransition,
    runtime: TransitionRuntime<E>,
}

impl<E: crate::text::TextEngine> ScrubbedRemoval<E> {
    pub fn new(spec: &TransitionSpec, reduce_motion: bool, now: Instant) -> Self {
        let transition = if reduce_motion {
            AnyTransition::new(Transition::reduced(spec.transition()))
        } else {
            spec.transition().clone()
        };
        assert_property_transition(transition.captures_content(TransitionPhase::Removal));
        Self {
            transition,
            runtime: TransitionRuntime::new(now),
        }
    }

    /// Samples the removal at absence `progress` for a slot of `size`.
    pub fn paint<'a>(
        &'a self,
        progress: f32,
        geometry: TransitionGeometry,
        hit: bool,
        owner: &'a Rc<NodeCell<E>>,
    ) -> TransitionPaint<'a, E> {
        TransitionPaint {
            properties: self
                .transition
                .properties(TransitionPhase::Removal, progress, geometry),
            hit,
            runtime: &self.runtime,
            owner,
        }
    }
}

/// One instance painted under its sampled transition: the properties,
/// whether it takes input, and the runtime and owning cell its paint scope and
/// blur host belong to.
pub struct TransitionPaint<'a, E: crate::text::TextEngine> {
    pub properties: TransitionProperties,
    pub hit: bool,
    pub runtime: &'a TransitionRuntime<E>,
    pub owner: &'a Rc<NodeCell<E>>,
}

impl<'a, E: crate::text::TextEngine> TransitionPaint<'a, E> {
    /// Samples `state` for a slot of `size`.
    pub fn sample(
        state: &TransitionState,
        size: Size,
        direction: waterui_core::layout::LayoutDirection,
        hit: bool,
        runtime: &'a TransitionRuntime<E>,
        owner: &'a Rc<NodeCell<E>>,
    ) -> Self {
        Self {
            properties: state.properties(TransitionGeometry::new(size, direction)),
            hit,
            runtime,
            owner,
        }
    }
}

/// Paints one transition instance inside a scope carrying its sampled
/// properties: `opacity` as the scope alpha (and the hit fade), translation,
/// scale and rotation composed about the slot's centre in the documented
/// order, the slot as the clip, and — while the sampled radius is nonzero —
/// a Gaussian blur filter layer around the instance. `hit: false` (a ghost)
/// keeps the scope painted while nothing inside takes input.
///
/// `child_ctx` is the slot's context and `delta` its placement relative to
/// the recording node's own placement; `flush` records the instance's node
/// at the context it receives, with an identity placement delta.
///
/// The scope's layer sits where the slot's content records: runs record in
/// the program's space, so the layer is `child_ctx`'s paint transform
/// relative to the enclosing scope's origin, never `delta`. The two differ
/// for a site that records into its enclosing layer's program — a `Dynamic`
/// host placed at an offset inside a stack paints its instances at that
/// offset.
pub fn flush_transitioned<E: crate::text::TextEngine>(
    renderer: &mut HydrolysisRenderer<E>,
    child_ctx: RenderContext,
    delta: kurbo::Affine,
    paint: &TransitionPaint<'_, E>,
    flush: impl FnOnce(&mut HydrolysisRenderer<E>, RenderContext),
) {
    let properties = paint.properties;
    let bounds = child_ctx.bounds();
    let local = transition_transform(&properties, bounds);
    let paint_space = child_ctx.local() * local;
    let layer = renderer.program().origin().inverse() * paint_space;
    let scope = crate::renderer::ScopeDelta {
        transform: delta * local,
        hit_alpha: properties.opacity(),
    };
    let key = crate::renderer::mount::ScopeKey {
        role: "transition",
        item: paint.runtime.scope_item(),
    };
    renderer.open_scope_with(
        key,
        layer,
        properties.opacity(),
        Some(&crate::renderer::frame::ScopeClip::Rect(kurbo::Rect::new(
            0.0,
            0.0,
            bounds.width(),
            bounds.height(),
        ))),
        paint_space,
        scope,
        paint.hit,
    );
    record_blurred(renderer, child_ctx, paint, flush);
    renderer.close_scope();
}

/// The sampled translation, scale and rotation of a slot with `bounds`:
/// scale and rotation resolve about the slot's centre, then translation —
/// `TransitionProperties`' documented compose order.
pub fn transition_transform(
    properties: &TransitionProperties,
    bounds: kurbo::Rect,
) -> kurbo::Affine {
    let centre = (
        bounds.x0 + bounds.width() / 2.0,
        bounds.y0 + bounds.height() / 2.0,
    );
    kurbo::Affine::translate((
        f64::from(properties.translation()[0]),
        f64::from(properties.translation()[1]),
    )) * kurbo::Affine::translate(centre)
        * kurbo::Affine::rotate(f64::from(properties.rotation()))
        * kurbo::Affine::scale_non_uniform(
            f64::from(properties.scale()[0]),
            f64::from(properties.scale()[1]),
        )
        * kurbo::Affine::translate((-centre.0, -centre.1))
}

/// Records one instance at `ctx` — through a Gaussian blur filter layer
/// while the sampled radius is nonzero, directly otherwise. The caller has
/// already opened the scope carrying the instance's other properties.
pub fn record_blurred<E: crate::text::TextEngine>(
    renderer: &mut HydrolysisRenderer<E>,
    ctx: RenderContext,
    paint: &TransitionPaint<'_, E>,
    flush: impl FnOnce(&mut HydrolysisRenderer<E>, RenderContext),
) {
    let properties = paint.properties;
    let bounds = ctx.bounds();
    assert!(
        properties.blur().is_finite() && properties.blur() >= 0.0,
        "Hydrolysis: a transition sampled the blur radius {}, which is not a finite \
         non-negative logical-pixel radius",
        properties.blur()
    );
    if properties.blur() > 0.0 {
        let mut slot = paint.runtime.blur.borrow_mut();
        let blur = slot.get_or_insert_with(|| {
            let radius = nami::Binding::container(properties.blur());
            let mut guards = ParamGuards::default();
            let filter = waterui_graphics::filtrate::filters::Blur(guards.bind(radius.clone()));
            let core = renderer.new_core();
            core.cell.set_parent(paint.owner);
            TransitionBlur {
                core,
                radius,
                runtime: Rc::new(RefCell::new(FilteredRuntime::new(
                    AnyEffect::filter(filter),
                    guards,
                ))),
            }
        });
        #[expect(
            clippy::float_cmp,
            reason = "an unchanged sample skips the parameter write; any change is written"
        )]
        if nami::Signal::snapshot(&blur.radius) != properties.blur() {
            blur.radius.set(properties.blur());
        }
        // The filtered layer host records the instance exactly as a
        // `FilteredView` node records its child: the engine filter covers
        // the host's whole program.
        renderer.link_placement(&blur.core, kurbo::Affine::IDENTITY);
        let runtime = Rc::clone(&blur.runtime);
        renderer.record_layer_node(&blur.core, bounds, |renderer, local| {
            renderer.program().program_mut().filter = Some(runtime);
            flush(renderer, local);
        });
    } else {
        flush(renderer, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use waterui_core::transition::PropertyTransition;

    fn removal_spec(milliseconds: u64) -> TransitionSpec {
        TransitionSpec::new(PropertyTransition::opacity())
            .with_animation(Animation::linear(Duration::from_millis(milliseconds)))
    }

    /// A fresh clock records its creation instant: sampling it then reports
    /// zero elapsed, while an older clock over the same spec completes —
    /// an idle stretch between collection frames cannot leak into a new
    /// clock's first delta.
    #[test]
    fn a_clock_born_after_idle_carries_no_elapsed_time() {
        let t0 = Instant::now();
        let spec = removal_spec(1_000);
        let mut old_state = spec.start(TransitionPhase::Removal, false);
        let mut old_clock = TransitionClock::new(t0);
        let mut fresh_state = spec.start(TransitionPhase::Removal, false);
        let mut fresh_clock = TransitionClock::new(t0 + Duration::from_secs(10));

        // The older clock over a 1s removal is done by t0+10s; the fresh
        // clock sampled at the same instant advanced nothing.
        assert!(!old_clock.advance(&mut old_state, t0 + Duration::from_secs(10)));
        assert!(fresh_clock.advance(&mut fresh_state, t0 + Duration::from_secs(10)));

        // +500ms on the fresh clock: half progress for a linear 1s removal.
        assert!(fresh_clock.advance(
            &mut fresh_state,
            t0 + Duration::from_secs(10) + Duration::from_millis(500)
        ));
        let progress = fresh_state.progress();
        assert!((progress - 0.5).abs() < 0.02, "{progress}");
    }

    /// A zero-duration state completes on its first advance regardless of the
    /// clock's delta.
    #[test]
    fn zero_duration_state_completes_on_first_advance() {
        let t0 = Instant::now();
        let spec = removal_spec(0);
        let mut state = spec.start(TransitionPhase::Removal, false);
        let mut clock = TransitionClock::new(t0);
        assert!(!clock.advance(&mut state, t0));
        assert!(state.is_complete());
    }
}
