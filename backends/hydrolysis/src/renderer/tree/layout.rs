//! Measure/layout for the retained tree: [`RenderNode::layout`] re-reads
//! signals, re-measures through [`NodeSubView`], and caches each container's
//! child frames for the flush pass.

use super::window::window_safe_area_insets;
use super::*;
use waterui_graphics::{resolve_scene_proposal, scene_stretch_axis};

impl RenderNode {
    /// The stretch axis this node exposes when it is a layout child.
    /// The layout priority this subtree carries, or `0` when nothing set one.
    ///
    /// Only layout-transparent wrappers are walked through: a priority applies to
    /// the child it wraps, not across a container boundary.
    pub(super) fn priority(&self) -> i32 {
        match self {
            RenderNode::Wrapper(node) => match &node.effect {
                WrapperEffect::LayoutPriority(priority) => priority.get(),
                _ => node.child.priority(),
            },
            RenderNode::Opacity(node) => node.child.priority(),
            RenderNode::Scale(node) => node.child.priority(),
            RenderNode::Rotation(node) => node.child.priority(),
            RenderNode::Offset(node) => node.child.priority(),
            RenderNode::Retain(node) => node.child.priority(),
            RenderNode::Env(node) => node.child.priority(),
            RenderNode::Dynamic(node) => node.child.borrow().priority(),
            RenderNode::Filtered(node) => node.child.priority(),
            RenderNode::Widget(node) => node.behavior.priority(),
            _ => 0,
        }
    }

    /// Whether this subtree draws nothing — `WaterUI`'s empty view `()`, or a
    /// container/wrapper whose every descendant does the same.
    ///
    /// This is a semantic answer, not a measured size: a zero-size `Color` or
    /// a collapsed `Spacer` still renders and still answers `false`. A stack
    /// treats a child answering `true` as a non-member (§4.4: it takes no
    /// slot and no spacing), and a `Dynamic` flipping between `()` and content
    /// is a membership change — `layout` re-asks this every pass.
    pub(super) fn is_empty(&self) -> bool {
        match self {
            RenderNode::Widget(node) => node.behavior.renders_nothing(),
            RenderNode::Opacity(node) => node.child.is_empty(),
            RenderNode::Scale(node) => node.child.is_empty(),
            RenderNode::Rotation(node) => node.child.is_empty(),
            RenderNode::Offset(node) => node.child.is_empty(),
            RenderNode::Retain(node) => node.child.is_empty(),
            RenderNode::Env(node) => node.child.is_empty(),
            RenderNode::Dynamic(node) => node.child.borrow().is_empty(),
            RenderNode::Wrapper(node) => node.child.is_empty(),
            // A filter over a child that draws nothing draws nothing itself.
            RenderNode::Filtered(node) => node.child.is_empty(),
            // A container is always a member — even a frame wrapping `()`
            // explicitly claims its configured slot, like a `Spacer` does —
            // and every variant not matched above draws nothing either.
            _ => false,
        }
    }

    pub(super) fn stretch(&self) -> StretchAxis {
        match self {
            RenderNode::Color(_) | RenderNode::Scroll(_) => StretchAxis::Both,
            // The same stack laid out eagerly is content-sized on both axes, so a
            // lazy one has to be too: making a stack virtualizable must not change
            // how it sizes. Rows that want the full cross axis ask for it
            // themselves, exactly as they do in the eager path.
            RenderNode::Text(_) | RenderNode::LazyStack(_) => StretchAxis::None,
            RenderNode::Container(container) => {
                // The retained children answer for themselves, so a transparent
                // layout reports what its content currently claims rather than
                // what it claimed when the tree was built.
                let child_axes: Vec<StretchAxis> =
                    container.children.iter().map(RenderNode::stretch).collect();
                container.layout.stretch_axis(&child_axes)
            }
            RenderNode::Opacity(node) => node.child.stretch(),
            RenderNode::Scale(node) => node.child.stretch(),
            RenderNode::Rotation(node) => node.child.stretch(),
            RenderNode::Offset(node) => node.child.stretch(),
            RenderNode::Retain(node) => node.child.stretch(),
            RenderNode::Env(node) => node.child.stretch(),
            RenderNode::Dynamic(node) => node.child.borrow().stretch(),
            // Scene content that is naturally a size is content-sized and claims
            // no leftover space; content that has no size of its own fills.
            RenderNode::SceneView(node) => {
                scene_stretch_axis(node.content.borrow().intrinsic_size())
            }
            // GPU content fills its proposal; a filtered view is a
            // layout-transparent wrapper delegating to its child.
            RenderNode::GpuContent(node) => {
                scene_stretch_axis(node.runtime.borrow().view.intrinsic_size())
            }
            RenderNode::ExternalFrame(node) => {
                scene_stretch_axis(node.runtime.borrow().view.intrinsic_size())
            }
            RenderNode::Filtered(node) => node.child.stretch(),
            RenderNode::Collection(node) => {
                // A collection's membership is reactive; its layout is one of the
                // content-sized stacks, which ignores the children anyway.
                node.layout.stretch_axis(&[])
            }
            RenderNode::Wrapper(node) => node.child.stretch(),
            RenderNode::Widget(node) => node.stretch,
        }
    }

    /// Measure this node under a proposal (recursive). Text shaping runs through
    /// the renderer's [`HydroState`] on the main thread. Visible crate-wide so
    /// the `Dynamic` dispatch measure can re-measure the retained child.
    ///
    /// Memoized per frame on the node's own [`NodeMeasureEntry`], for
    /// container-like nodes only: a layout engine that re-probes a child
    /// under different proposals (a stack measuring at `None` for an extent,
    /// then at a concrete width, then the same pair again from `place`)
    /// would otherwise re-run the whole subtree measure — multiplicatively
    /// with depth. Every other node's body is a leaf answer or a
    /// `child.measure` forward (itself gated where it matters), so memoizing
    /// those buys nothing.
    ///
    /// The [`MemoGate`] runs first: a node whose probes never repeat a
    /// proposal within a frame (the common case — the memo could never hit)
    /// pays a `Cell` update and skips the `RefCell` borrow, the ring scan,
    /// and the dimensions store. Only a node observed being re-probed under
    /// a proposal it already answered keeps memoizing.
    pub(crate) fn measure(
        &self,
        state: &mut HydroState,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let (memo_gate, memo_slots) = match self {
            RenderNode::Container(node) => (&node.memo_gate, &node.memo_slots),
            RenderNode::Scroll(node) => (&node.memo_gate, &node.memo_slots),
            RenderNode::Collection(node) => (&node.memo_gate, &node.memo_slots),
            RenderNode::LazyStack(node) => (&node.memo_gate, &node.memo_slots),
            RenderNode::Text(node) => (&node.memo_gate, &node.memo_slots),
            _ => return self.measure_body(state, env, theme, proposal),
        };
        let frame = state.measurement.frame();
        let mut gate = memo_gate.get();
        if !gate.probe(frame, proposal) {
            memo_gate.set(gate);
            return self.measure_body(state, env, theme, proposal);
        }
        memo_gate.set(gate);
        // Only a probe the gate lets through needs the env identity.
        let env_identity = env.identity();
        let hit = memo_slots.borrow().dims(env_identity, frame, proposal);
        if let Some(hit) = hit {
            return hit;
        }
        let dimensions = self.measure_body(state, env, theme, proposal);
        let mut memo = memo_slots.borrow_mut();
        memo.ensure_current(frame);
        memo.push_dims(env_identity, proposal, dimensions.clone());
        dimensions
    }

    /// The uncached measure [`Self::measure`] memoizes; the recursive match over
    /// the node's payload.
    pub(crate) fn measure_body(
        &self,
        state: &mut HydroState,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        state.counters.measure_calls += 1;
        match self {
            RenderNode::Color(_) => ViewDimensions::new(Size::new(
                proposal.width.unwrap_or(0.0),
                proposal.height.unwrap_or(0.0),
            )),
            RenderNode::Text(text) => HydrolysisRenderer::measure_text_dimensions(
                state,
                text.content.snapshot(),
                text.alignment.snapshot(),
                env,
                proposal.width,
                text.line_limit,
            ),
            RenderNode::Container(container) => {
                let cell = RefCell::new(state);
                let subs: Vec<NodeSubView> = container
                    .children
                    .iter()
                    .map(|child| NodeSubView::new(child, &cell, env, theme))
                    .collect();
                let refs: Vec<&dyn SubView> = subs.iter().map(|sub| sub as &dyn SubView).collect();
                ViewDimensions::new(container.layout.size_that_fits(proposal, &refs))
            }
            // Transform/opacity wrappers are layout-transparent.
            RenderNode::Opacity(node) => node.child.measure(state, env, theme, proposal),
            RenderNode::Scale(node) => node.child.measure(state, env, theme, proposal),
            RenderNode::Rotation(node) => node.child.measure(state, env, theme, proposal),
            RenderNode::Offset(node) => node.child.measure(state, env, theme, proposal),
            RenderNode::Retain(node) => node.child.measure(state, env, theme, proposal),
            RenderNode::Env(node) => node.child.measure(state, &node.env, theme, proposal),
            RenderNode::Dynamic(node) => {
                let dimensions = node.child.borrow().measure(state, env, theme, proposal);
                // The connected node's real per-proposal answers feed the
                // dispatch measure of the same `Dynamic` (`measure_dynamic`).
                state.measurement.store_dynamic_dimensions(
                    node.source.identity(),
                    proposal,
                    dimensions.clone(),
                );
                dimensions
            }
            // Scene content that is naturally a size (an SVG's viewBox, a
            // formula's typeset box) answers with it on whichever axis the
            // container left open; content that is not fills the proposal.
            RenderNode::SceneView(node) => {
                let resolved =
                    resolve_scene_proposal(node.content.borrow().intrinsic_size(), proposal);
                ViewDimensions::new(Size::new(
                    resolved.width.unwrap_or(0.0),
                    resolved.height.unwrap_or(0.0),
                ))
            }
            // GPU content answers like a scene: content that is naturally a
            // size is content-sized; content without one fills the proposal.
            RenderNode::GpuContent(node) => {
                let resolved =
                    resolve_scene_proposal(node.runtime.borrow().view.intrinsic_size(), proposal);
                ViewDimensions::new(Size::new(
                    resolved.width.unwrap_or(0.0),
                    resolved.height.unwrap_or(0.0),
                ))
            }
            // External frames measure through their source: a stream with an
            // intrinsic size is content-sized, one without fills the proposal.
            RenderNode::ExternalFrame(node) => node.runtime.borrow().view.measure(proposal),
            // A filtered view is sized by its content: the engine applies the
            // filter to the mount the child's layers hang from.
            RenderNode::Filtered(node) => node.child.measure(state, &node.env, theme, proposal),
            RenderNode::Scroll(node) => {
                // layout-spec.md §6: a scroll claims the whole offer — a
                // finite proposal on either axis is answered with that
                // proposal; only a `0` proposal measures the content,
                // answering its intrinsic extent on the non-scrolling axis
                // and `0` on the scrolling axis.
                let mut size = Size::new(
                    proposal.width.unwrap_or(0.0),
                    proposal.height.unwrap_or(0.0),
                );
                if match node.axis {
                    ScrollAxis::Vertical => proposal.width == Some(0.0),
                    ScrollAxis::Horizontal => proposal.height == Some(0.0),
                    ScrollAxis::All => false,
                    _ => panic!("hydrolysis render tree: unsupported scroll axis"),
                } {
                    let floor = node.non_scrolling_minimum.get().unwrap_or_else(|| {
                        // The `0` propagates to the content: its own minimum
                        // on the non-scrolling axis is the floor the scroll
                        // reports there. The scrolling axis gets `None`.
                        let content_proposal = match node.axis {
                            ScrollAxis::Vertical => ProposalSize::new(Some(0.0), None),
                            ScrollAxis::Horizontal => ProposalSize::new(None, Some(0.0)),
                            _ => ProposalSize::UNSPECIFIED,
                        };
                        let measured = node.child.measure(state, env, theme, content_proposal).size;
                        let value = match node.axis {
                            ScrollAxis::Vertical => measured.width,
                            _ => measured.height,
                        };
                        node.non_scrolling_minimum.set(Some(value));
                        value
                    });
                    match node.axis {
                        ScrollAxis::Vertical => size.width = floor,
                        ScrollAxis::Horizontal => size.height = floor,
                        _ => {}
                    }
                }
                // A `None` proposal asks for the ideal (intrinsic) extent
                // (layout-spec.md §2), and the `min <= ideal <= max` probe
                // invariant there means it cannot sit below the `0` answer:
                // on the non-scrolling axis that minimum is already the
                // content's intrinsic extent, so the ideal is the same
                // content measure; on the scrolling axis the scroll's
                // intrinsic extent is its content's. The content is measured
                // with the proposed extent on the non-scrolling axis and
                // `None` on the scrolling axis, as §6 prescribes — so a
                // `(None, _)` answer reports the same extent the layout pass
                // measures the scroll's content at, and nested scrolls see a
                // real content size instead of `0`.
                if proposal.width.is_none() || proposal.height.is_none() {
                    let content_proposal = match node.axis {
                        ScrollAxis::Vertical => ProposalSize::new(proposal.width, None),
                        ScrollAxis::Horizontal => ProposalSize::new(None, proposal.height),
                        ScrollAxis::All => ProposalSize::UNSPECIFIED,
                        _ => panic!("hydrolysis render tree: unsupported scroll axis"),
                    };
                    let intrinsic = node.child.measure(state, env, theme, content_proposal).size;
                    if proposal.width.is_none() {
                        size.width = intrinsic.width;
                    }
                    if proposal.height.is_none() {
                        size.height = intrinsic.height;
                    }
                }
                ViewDimensions::new(size)
            }
            RenderNode::LazyStack(node) => node.measure(state, theme, proposal),
            RenderNode::Collection(node) => node.measure(state, theme, proposal),
            // Layout-transparent: the wrapper measures its child under the node's
            // scoped environment (effect colors/a11y read env every frame).
            RenderNode::Wrapper(node) => node.child.measure(state, &node.env, theme, proposal),
            RenderNode::Widget(node) => node.behavior.measure(state, proposal, &node.env, theme),
        }
    }

    /// The innermost container under this node's layout-transparent wrappers —
    /// the `Env` an `hstack`'s `with(Axis)` installs, retains, effects — for
    /// tests that drive its `Layout` against recording children.
    #[cfg(test)]
    pub(in crate::renderer) fn transparent_container(&self) -> Option<&ContainerNode> {
        let mut node = self;
        loop {
            match node {
                RenderNode::Container(container) => return Some(&**container),
                RenderNode::Env(inner) => node = &inner.child,
                RenderNode::Wrapper(inner) => node = &inner.child,
                RenderNode::Retain(inner) => node = &inner.child,
                RenderNode::Opacity(inner) => node = &inner.child,
                RenderNode::Scale(inner) => node = &inner.child,
                RenderNode::Rotation(inner) => node = &inner.child,
                RenderNode::Offset(inner) => node = &inner.child,
                RenderNode::Filtered(inner) => node = &inner.child,
                _ => return None,
            }
        }
    }

    /// The stretch contract a `NodeSubView` over this child would report — for
    /// tests that drive a container's `Layout` against recording children.
    #[cfg(test)]
    pub(in crate::renderer) fn stretch_for_test(&self) -> StretchAxis {
        self.stretch()
    }

    /// Run the layout-time prepare pass over this subtree: every widget leaf
    /// applies theme paint that could not be resolved at tree-build time —
    /// build contexts carry no theme — and builds the retained sub-views its
    /// measure path then reads. Called once at each layout or measure entry
    /// point (`RetainedSubview::{measure_intrinsic, patch_and_measure,
    /// flush_in_rect, flush_in_ctx, render_built_scene}` and the window's
    /// layout pump), before any node is measured. A semantic runtime never
    /// runs this pass, so no theme reaches it.
    pub(in crate::renderer) fn prepare_for_measure(&mut self, renderer: &mut HydrolysisRenderer) {
        match self {
            RenderNode::Widget(node) => node.behavior.prepare(renderer, &node.env),
            RenderNode::Opacity(node) => node.child.prepare_for_measure(renderer),
            RenderNode::Scale(node) => node.child.prepare_for_measure(renderer),
            RenderNode::Rotation(node) => node.child.prepare_for_measure(renderer),
            RenderNode::Offset(node) => node.child.prepare_for_measure(renderer),
            RenderNode::Retain(node) => node.child.prepare_for_measure(renderer),
            RenderNode::Dynamic(node) => node.child.borrow_mut().prepare_for_measure(renderer),
            RenderNode::Env(node) => node.child.prepare_for_measure(renderer),
            RenderNode::Wrapper(node) => node.child.prepare_for_measure(renderer),
            RenderNode::Filtered(node) => node.child.prepare_for_measure(renderer),
            RenderNode::Container(node) => {
                for child in &mut node.children {
                    child.prepare_for_measure(renderer);
                }
            }
            RenderNode::Scroll(node) => node.child.prepare_for_measure(renderer),
            RenderNode::Collection(node) => {
                for entry in &mut node.entries {
                    entry.node.prepare_for_measure(renderer);
                }
            }
            RenderNode::LazyStack(node) => {
                node.item_cache.borrow_mut().prepare_for_measure(renderer);
            }
            RenderNode::Color(_)
            | RenderNode::Text(_)
            | RenderNode::SceneView(_)
            | RenderNode::GpuContent(_)
            | RenderNode::ExternalFrame(_) => {}
        }
    }

    /// Re-measure and re-place this subtree, caching each container's child
    /// frames. Run on build and whenever a geometry-affecting input changes.
    pub(crate) fn layout(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
        proposal: ProposalSize,
        size: Size,
    ) {
        renderer.state.counters.layout_calls += 1;
        // The selected proposal and resolved size are distinct layout inputs.
        // Transparent wrappers preserve both without reconstructing an offer.
        let theme = renderer.theme();
        match self {
            RenderNode::Container(container) => {
                let placements = {
                    let cell = RefCell::new(&mut renderer.state);
                    let subs: Vec<NodeSubView> = container
                        .children
                        .iter()
                        .map(|child| NodeSubView::new(child, &cell, env, &theme))
                        .collect();
                    let refs: Vec<&dyn SubView> =
                        subs.iter().map(|sub| sub as &dyn SubView).collect();
                    // Only the accessibility scope reads `resolved` — skip the
                    // measure in builds without it.
                    #[cfg(feature = "accessibility")]
                    {
                        container.resolved = resolved_content_rect(
                            container.layout.size_that_fits(proposal, &refs),
                            size,
                        );
                    }
                    container
                        .layout
                        .place(Rect::from_size(size), proposal, &refs)
                };
                for (child, placement) in container.children.iter_mut().zip(&placements) {
                    child.layout(renderer, env, placement.proposal, *placement.frame.size());
                }
                container.placed = placements
                    .into_iter()
                    .map(|placement| placement.frame)
                    .collect();
            }
            // Transform/opacity wrappers are layout-transparent: the child lays out
            // at the same concrete size as the wrapper.
            RenderNode::Opacity(node) => node.child.layout(renderer, env, proposal, size),
            RenderNode::Scale(node) => node.child.layout(renderer, env, proposal, size),
            RenderNode::Rotation(node) => node.child.layout(renderer, env, proposal, size),
            RenderNode::Offset(node) => node.child.layout(renderer, env, proposal, size),
            RenderNode::Retain(node) => node.child.layout(renderer, env, proposal, size),
            RenderNode::Env(node) => {
                let node_env = node.env.clone();
                node.child.layout(renderer, &node_env, proposal, size);
            }
            // Layout-transparent: the child lays out at the same concrete size,
            // under the wrapper's scoped environment — except `.ignore_safe_area`,
            // which releases the window's safe-area insets on its flagged edges.
            RenderNode::Wrapper(node) => {
                let node_env = node.env.clone();
                if let WrapperEffect::IgnoreSafeArea(edges) = &node.effect {
                    let insets = window_safe_area_insets(renderer, env);
                    let width = size.width
                        + if edges.leading { insets.leading() } else { 0.0 }
                        + if edges.trailing {
                            insets.trailing()
                        } else {
                            0.0
                        };
                    let height = size.height
                        + if edges.top { insets.top() } else { 0.0 }
                        + if edges.bottom { insets.bottom() } else { 0.0 };
                    let child_proposal = ProposalSize::new(
                        proposal.width.map(|_| width),
                        proposal.height.map(|_| height),
                    );
                    node.child.layout(
                        renderer,
                        &node_env,
                        child_proposal,
                        Size::new(width, height),
                    );
                } else {
                    node.child.layout(renderer, &node_env, proposal, size);
                }
            }
            RenderNode::Dynamic(node) => {
                node.child
                    .borrow_mut()
                    .layout(renderer, env, proposal, size);
            }
            RenderNode::Scroll(node) => {
                let child_proposal = match node.axis {
                    ScrollAxis::Horizontal => ProposalSize::new(None, Some(size.height)),
                    ScrollAxis::Vertical => ProposalSize::new(Some(size.width), None),
                    ScrollAxis::All => ProposalSize::UNSPECIFIED,
                    _ => panic!("hydrolysis render tree: unsupported scroll axis"),
                };
                let intrinsic = node
                    .child
                    .measure(&mut renderer.state, env, &theme, child_proposal)
                    .size;
                let content_size = match node.axis {
                    ScrollAxis::Horizontal => {
                        Size::new(intrinsic.width.max(size.width), size.height)
                    }
                    ScrollAxis::Vertical => {
                        Size::new(size.width, intrinsic.height.max(size.height))
                    }
                    ScrollAxis::All => Size::new(
                        intrinsic.width.max(size.width),
                        intrinsic.height.max(size.height),
                    ),
                    _ => panic!("hydrolysis render tree: unsupported scroll axis"),
                };
                node.child
                    .layout(renderer, env, child_proposal, content_size);
                let handle = if let Some(handle) = node.handle.borrow_mut().as_mut() {
                    handle.rebind(
                        node.axis,
                        f64::from(size.width),
                        f64::from(size.height),
                        f64::from(content_size.width),
                        f64::from(content_size.height),
                    )
                } else {
                    // `report_offset`: the handle writes the content offset
                    // into this binding on every change, glide frames
                    // included, from here on.
                    ScrollHandle::new(
                        node.axis,
                        f64::from(size.width),
                        f64::from(size.height),
                        f64::from(content_size.width),
                        f64::from(content_size.height),
                        node.offset.clone(),
                    )
                };
                if let Some(controller) = &node.controller {
                    let generation = renderer.read_signal(&controller.generation());
                    if generation != node.applied_scroll_generation.get() {
                        let target = renderer.read_signal(&controller.target());
                        let _ = handle.scroll_to(f64::from(target.x), f64::from(target.y));
                        node.applied_scroll_generation.set(generation);
                    }
                }
                *node.handle.borrow_mut() = Some(handle);
                node.content_size = content_size;
                node.viewport = size;
            }
            RenderNode::Collection(node) => node.layout(renderer, proposal, size),
            RenderNode::Filtered(node) => {
                let node_env = node.env.clone();
                node.child.layout(renderer, &node_env, proposal, size);
            }
            // A lazy stack places its items lazily at flush (offset-dependent); a
            // widget leaf or GPU content view renders itself at flush from
            // `ctx.bounds`. Nothing to pre-lay-out for any of these.
            RenderNode::Color(_)
            | RenderNode::Text(_)
            | RenderNode::SceneView(_)
            | RenderNode::GpuContent(_)
            | RenderNode::ExternalFrame(_)
            | RenderNode::LazyStack(_)
            | RenderNode::Widget(_) => {}
        }
    }
}

/// The rect a container resolved for itself — the size it answered to the
/// selected proposal, centred on the assigned frame — in the same local space
/// as `placed`. A container is routinely assigned more than it answered (a
/// window's `Overlay` places its base over the whole bounds), so the assigned
/// frame is not the element's own extent; centring the answer on the assigned
/// frame reports where the view actually sits, matching the centre-anchored
/// underfill convention the layout contract and `SwiftUI` share. Anchoring on
/// the placed envelope instead would shift the rect by wherever the children
/// happen to sit — a leading-inset padding pushes it outside the assigned
/// frame entirely. A non-finite answer axis falls back to the assigned extent
/// on that axis.
#[cfg(feature = "accessibility")]
pub(super) fn resolved_content_rect(answer: Size, assigned: Size) -> Rect {
    let width = if answer.width.is_finite() {
        answer.width
    } else {
        assigned.width
    };
    let height = if answer.height.is_finite() {
        answer.height
    } else {
        assigned.height
    };
    let center = Rect::from_size(assigned).center();
    Rect::new(
        Point::new(center.x - width / 2.0, center.y - height / 2.0),
        Size::new(width, height),
    )
}

/// `rect` in kurbo coordinates — [`Rect`] is the f32 layout space while
/// `RenderContext` geometry is kurbo f64.
#[cfg(feature = "accessibility")]
pub(super) fn kurbo_rect(rect: Rect) -> kurbo::Rect {
    kurbo::Rect::new(
        f64::from(rect.x()),
        f64::from(rect.y()),
        f64::from(rect.max_x()),
        f64::from(rect.max_y()),
    )
}

/// A deterministic digest of a layout pass's output — FNV-1a over every node's
/// variant tag and placed frame, walked depth-first. The frame-profile
/// example compares digests across runs to prove a change left layout
/// byte-identical.
#[cfg(feature = "frame-profile")]
pub(super) struct SignatureHasher(u64);

#[cfg(feature = "frame-profile")]
impl SignatureHasher {
    pub(super) fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub(super) fn mix(&mut self, bits: u64) {
        self.0 = (self.0 ^ bits).wrapping_mul(0x0000_0100_0000_01b3);
    }
}

#[cfg(feature = "frame-profile")]
impl std::hash::Hasher for SignatureHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.mix(u64::from(byte));
        }
    }

    fn write_u32(&mut self, value: u32) {
        self.mix(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.mix(value);
    }

    fn write_usize(&mut self, value: usize) {
        self.mix(value as u64);
    }
}

/// Hash a [`Size`] into the layout digest.
#[cfg(feature = "frame-profile")]
pub(super) fn hash_size(hasher: &mut SignatureHasher, size: Size) {
    hasher.mix(u64::from(size.width.to_bits()));
    hasher.mix(u64::from(size.height.to_bits()));
}

/// Hash a node's placed [`Rect`] into the layout digest.
#[cfg(feature = "frame-profile")]
pub(super) fn hash_frame(hasher: &mut SignatureHasher, frame: Rect) {
    hasher.mix(u64::from(frame.x().to_bits()));
    hasher.mix(u64::from(frame.y().to_bits()));
    hash_size(hasher, *frame.size());
}

#[cfg(feature = "frame-profile")]
impl RenderNode {
    /// Digest of what the layout pass computed for this subtree under `frame`:
    /// every node's variant tag and its placed frame, depth-first, plus the
    /// geometry nodes cache for the flush pass (a scroll's content size and
    /// viewport, a lazy stack's retained items). Identical digests on two runs
    /// mean layout produced identical bounds for every node.
    pub(crate) fn placed_signature(&self, frame: Rect) -> u64 {
        let mut hasher = SignatureHasher::new();
        self.signature_into(frame, &mut hasher);
        std::hash::Hasher::finish(&hasher)
    }

    pub(super) fn signature_into(&self, frame: Rect, hasher: &mut SignatureHasher) {
        use std::hash::Hash;
        core::mem::discriminant(self).hash(hasher);
        hash_frame(hasher, frame);
        match self {
            RenderNode::Color(_)
            | RenderNode::Text(_)
            | RenderNode::SceneView(_)
            | RenderNode::GpuContent(_)
            | RenderNode::ExternalFrame(_)
            | RenderNode::Widget(_) => {}
            RenderNode::Opacity(node) => node.child.signature_into(frame, hasher),
            RenderNode::Scale(node) => node.child.signature_into(frame, hasher),
            RenderNode::Rotation(node) => node.child.signature_into(frame, hasher),
            RenderNode::Offset(node) => node.child.signature_into(frame, hasher),
            RenderNode::Retain(node) => node.child.signature_into(frame, hasher),
            RenderNode::Env(node) => node.child.signature_into(frame, hasher),
            RenderNode::Wrapper(node) => node.child.signature_into(frame, hasher),
            RenderNode::Dynamic(node) => node.child.borrow().signature_into(frame, hasher),
            RenderNode::Filtered(node) => node.child.signature_into(frame, hasher),
            RenderNode::Container(node) => {
                node.placed.len().hash(hasher);
                for (child, rect) in node.children.iter().zip(&node.placed) {
                    child.signature_into(*rect, hasher);
                }
            }
            RenderNode::Collection(node) => {
                node.placed.len().hash(hasher);
                for (entry, rect) in node.entries.iter().zip(&node.placed) {
                    entry.node.signature_into(*rect, hasher);
                }
            }
            RenderNode::Scroll(node) => {
                hash_size(hasher, node.content_size);
                hash_size(hasher, node.viewport);
                node.child
                    .signature_into(Rect::from_size(node.content_size), hasher);
            }
            RenderNode::LazyStack(node) => node.item_cache.borrow().signature_into(hasher),
        }
    }
}
