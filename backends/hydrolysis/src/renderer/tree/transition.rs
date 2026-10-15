//! Structural transition ownership in the retained tree (layout-spec.md §4.5).
//!
//! A `.transition(...)` declaration is built into a transparent
//! [`WrapperEffect::Transition`] node carrying a [`TransitionDeclaration`].
//! The structural site that swaps the content — a `Dynamic` host, a
//! collection entry, a lazy-stack row, an anchored-overlay presentation —
//! claims it from the *built* instance by walking the instance's root chain
//! ([`claim_transition`]), so ownership never depends on the declared view's
//! static type: a type-erased row and a declaration beneath other modifiers
//! resolve the same way. A declaration placed directly over a conditional is
//! adopted by that `Dynamic` host ([`adopt_by_dynamic`]) and applies to each
//! content instance it swaps in and out. A declaration nobody owns fails at
//! the first pass that reaches it.
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
    Ghost, GhostSlot, TransitionGeometry, TransitionPhase, TransitionProperties, TransitionSpec,
    TransitionState,
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
             A transition declares how a view is inserted and removed, so it must sit on the \
             root of the content a structural site swaps — a `when`/`watch` branch or the \
             conditional itself, a `ForEach`/`List` row, a navigation page root, or an \
             anchored-overlay body — reachable \
             through modifiers only, not beneath a stack, scroll or widget (layout-spec.md §4.5)"
        );
    }
}

/// The child a node's transition claim continues into, when the node is
/// transparent to the claim: single-content wrappers and modifier containers
/// (padding, frame, aspect ratio, background, overlay) keep the root chain of
/// the instance; any other node ends it.
fn claim_child(node: &RenderNode) -> Option<&RenderNode> {
    match node {
        RenderNode::Wrapper(node) => Some(&node.child),
        RenderNode::Env(node) => Some(&node.child),
        RenderNode::Opacity(node) => Some(&node.child),
        RenderNode::Scale(node) => Some(&node.child),
        RenderNode::Rotation(node) => Some(&node.child),
        RenderNode::Offset(node) => Some(&node.child),
        RenderNode::Retain(node) => Some(&node.child),
        RenderNode::Filtered(node) => Some(&node.child),
        RenderNode::Fill(node) => Some(&node.child),
        RenderNode::Container(node) => node.claim_slot.map(|slot| &node.children[slot]),
        _ => None,
    }
}

/// Claims the transition declared on a structural instance's root chain:
/// the first [`TransitionDeclaration`] reachable through transparent nodes.
/// A nested structural site (`Dynamic`, collection, stack) ends the chain —
/// a declaration beneath it belongs to that site.
pub fn claim_transition(node: &RenderNode) -> Option<TransitionSpec> {
    let mut node = node;
    loop {
        if let RenderNode::Wrapper(wrapper) = node
            && let WrapperEffect::Transition(declaration) = &wrapper.effect
        {
            return Some(declaration.take_ownership());
        }
        node = claim_child(node)?;
    }
}

/// Claims the transition declared on a navigation page's root: the page is
/// a navigation view whose content root chain the owning stack claims
/// through ([`WidgetBehavior::claim_page_transition`]). Only the stack calls
/// this — a navigation view anywhere else is not a structural instance, so a
/// declaration on its content stays unowned.
pub fn claim_page_transition(page: &RenderNode) -> Option<TransitionSpec> {
    let mut node = page;
    loop {
        match node {
            RenderNode::Wrapper(wrapper) => {
                if let WrapperEffect::Transition(declaration) = &wrapper.effect {
                    return Some(declaration.take_ownership());
                }
            }
            RenderNode::Widget(widget) => return widget.behavior.claim_page_transition(),
            _ => {}
        }
        node = claim_child(node)?;
    }
}

/// Hands a declaration built directly over a conditional to that `Dynamic`
/// host: its content instances transition with it whenever the host swaps
/// them. Walks the same transparent chain [`claim_transition`] does.
pub fn adopt_by_dynamic(child: &RenderNode, declaration: &TransitionDeclaration) {
    let mut node = child;
    loop {
        if let RenderNode::Dynamic(host) = node {
            host.adopt(declaration.take_ownership());
            return;
        }
        let Some(next) = claim_child(node) else {
            return;
        };
        node = next;
    }
}

/// The `Dynamic` host a container child presents its removal ghosts through:
/// the host reached from the child through nodes that neither paint nor
/// change geometry, so a ghost painted by the container directly loses
/// nothing the live member keeps. Behind any other node the host keeps its
/// ghosts inside its own slot.
pub fn member_host(node: &RenderNode) -> Option<&DynamicHostNode> {
    match node {
        RenderNode::Dynamic(host) => Some(host),
        RenderNode::Env(node) => member_host(&node.child),
        RenderNode::Retain(node) => member_host(&node.child),
        RenderNode::Wrapper(node)
            if matches!(
                node.effect,
                WrapperEffect::Transition(_)
                    | WrapperEffect::LayoutPriority(_)
                    | WrapperEffect::LifeCycle(_)
            ) =>
        {
            member_host(&node.child)
        }
        _ => None,
    }
}

/// Fails fast on a transition whose resolved form still needs pixel capture:
/// capture-based effects are realized by the transition overlay, which is not
/// implemented in Hydrolysis yet. The Reduce Motion form is resolved first, so
/// a capture effect whose declared reduction is a property effect still runs.
pub fn checked_transition(state: TransitionState) -> TransitionState {
    assert!(
        !state.captures_content(),
        "Hydrolysis: pixel-capture transitions (a `Transition` whose `captures_content` \
         is true, such as disintegrate or dissolve) are not realized yet — their \
         window-level capture overlay is water-rs/waterui#1388. Use a property transition \
         (opacity, scale, move, blur, rotation) or declare a property `reduced()` form \
         and enable Reduce Motion"
    );
    state
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
    let delta = kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0));
    (
        RenderContext {
            local: ctx.local * delta,
            bounds: kurbo::Rect::new(0.0, 0.0, ctx.bounds.width(), ctx.bounds.height()),
        },
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
struct TransitionBlur {
    core: NodeCore,
    radius: nami::Binding<f32>,
    runtime: Rc<RefCell<FilteredRuntime>>,
}

/// The per-instance machinery a structural site keeps beside a
/// [`TransitionState`]: its clock, the identity its paint scope is keyed by,
/// and — created on the first frame that samples a nonzero blur radius — the
/// blur filter host.
pub struct TransitionRuntime {
    clock: TransitionClock,
    /// Identity of this instance's paint scope: unique among every live
    /// instance, so two sites recording into one program never share a key.
    marker: Rc<()>,
    blur: RefCell<Option<TransitionBlur>>,
}

impl TransitionRuntime {
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
pub struct TransitionRun {
    pub state: TransitionState,
    pub runtime: TransitionRuntime,
}

impl TransitionRun {
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

/// One instance painted under its sampled transition: the properties,
/// whether it takes input, and the runtime and owning cell its paint scope and
/// blur host belong to.
pub struct TransitionPaint<'a> {
    pub properties: TransitionProperties,
    pub hit: bool,
    pub runtime: &'a TransitionRuntime,
    pub owner: &'a Rc<NodeCell>,
}

impl<'a> TransitionPaint<'a> {
    /// Samples `state` for a slot of `size`.
    pub fn sample(
        state: &TransitionState,
        size: Size,
        direction: waterui_core::layout::LayoutDirection,
        hit: bool,
        runtime: &'a TransitionRuntime,
        owner: &'a Rc<NodeCell>,
    ) -> Self {
        Self {
            properties: state.properties(TransitionGeometry { size, direction }),
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
/// `child_ctx` is the slot's context and `delta` its placement in the
/// enclosing record; `flush` records the instance's node at the context it
/// receives, with an identity placement delta.
pub fn flush_transitioned(
    renderer: &mut HydrolysisRenderer,
    child_ctx: RenderContext,
    delta: kurbo::Affine,
    paint: &TransitionPaint<'_>,
    flush: impl FnOnce(&mut HydrolysisRenderer, RenderContext),
) {
    let properties = paint.properties;
    let bounds = child_ctx.bounds;
    let local = transition_transform(&properties, bounds);
    let layer = delta * local;
    let scope = crate::renderer::ScopeDelta {
        transform: layer,
        hit_alpha: properties.opacity,
    };
    let key = crate::renderer::mount::ScopeKey {
        role: "transition",
        item: paint.runtime.scope_item(),
    };
    renderer.open_scope_with(
        key,
        layer,
        properties.opacity,
        Some(&crate::renderer::frame::ScopeClip::Rect(kurbo::Rect::new(
            0.0,
            0.0,
            bounds.width(),
            bounds.height(),
        ))),
        child_ctx.local * local,
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
        f64::from(properties.translation[0]),
        f64::from(properties.translation[1]),
    )) * kurbo::Affine::translate(centre)
        * kurbo::Affine::rotate(f64::from(properties.rotation))
        * kurbo::Affine::scale_non_uniform(
            f64::from(properties.scale[0]),
            f64::from(properties.scale[1]),
        )
        * kurbo::Affine::translate((-centre.0, -centre.1))
}

/// Records one instance at `ctx` — through a Gaussian blur filter layer
/// while the sampled radius is nonzero, directly otherwise. The caller has
/// already opened the scope carrying the instance's other properties.
pub fn record_blurred(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    paint: &TransitionPaint<'_>,
    flush: impl FnOnce(&mut HydrolysisRenderer, RenderContext),
) {
    let properties = paint.properties;
    let bounds = ctx.bounds;
    assert!(
        properties.blur.is_finite() && properties.blur >= 0.0,
        "Hydrolysis: a transition sampled the blur radius {}, which is not a finite \
         non-negative logical-pixel radius",
        properties.blur
    );
    if properties.blur > 0.0 {
        let mut slot = paint.runtime.blur.borrow_mut();
        let blur = slot.get_or_insert_with(|| {
            let radius = nami::Binding::container(properties.blur);
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
        if nami::Signal::snapshot(&blur.radius) != properties.blur {
            blur.radius.set(properties.blur);
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

/// One layout member of a container: a child, or a removal ghost the child's
/// `Dynamic` host presents beside it.
#[derive(Clone, Copy)]
pub enum Member {
    /// The child at this index.
    Child(usize),
    /// Ghost `ghost` of the host under child `child`, whose slot is
    /// `Members::slots[slot]`.
    Ghost {
        child: usize,
        ghost: usize,
        slot: usize,
    },
}

/// A container's layout members in order: every child, each preceded by the
/// ghosts of the instances its `Dynamic` host swapped out — an outgoing
/// instance keeps its slot where it was, and the instance replacing it gets
/// a new slot after it (layout-spec.md §4.5).
pub struct Members {
    pub order: Vec<Member>,
    pub slots: Vec<GhostSlot>,
}

impl Members {
    pub fn of(children: &[RenderNode]) -> Self {
        let mut order = Vec::with_capacity(children.len());
        let mut slots = Vec::new();
        for (child, node) in children.iter().enumerate() {
            if let Some(host) = member_host(node) {
                for (ghost, slot) in host.member_ghost_slots().into_iter().enumerate() {
                    order.push(Member::Ghost {
                        child,
                        ghost,
                        slot: slots.len(),
                    });
                    slots.push(slot);
                }
            }
            order.push(Member::Child(child));
        }
        Self { order, slots }
    }

    /// The members as the layout sees them, `subs` being the children's
    /// sub-views by child index.
    pub fn refs<'a, S: SubView>(&'a self, subs: &'a [S]) -> Vec<&'a dyn SubView> {
        self.order
            .iter()
            .map(|member| match *member {
                Member::Child(index) => &subs[index] as &dyn SubView,
                Member::Ghost { slot, .. } => &self.slots[slot] as &dyn SubView,
            })
            .collect()
    }
}

/// The `Dynamic` host under a container child that presents ghost members —
/// the host [`Members::of`] found for a [`Member::Ghost`].
pub fn ghost_host(node: &RenderNode) -> &DynamicHostNode {
    member_host(node).expect("hydrolysis transition: a ghost member's child lost its Dynamic host")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use waterui_core::transition::{AnyTransition, PropertyTransition};

    fn removal_spec(milliseconds: u64) -> TransitionSpec {
        TransitionSpec {
            transition: AnyTransition::new(PropertyTransition::opacity()),
            animation: Animation::linear(Duration::from_millis(milliseconds)),
        }
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
