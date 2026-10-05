//! Measure/layout for the retained tree: [`RenderNode::layout`] re-reads
//! signals, re-measures through [`NodeSubView`], and caches each container's
//! child frames for the flush pass.

#[cfg(test)]
use super::ContainerNode;
// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
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
            Self::Wrapper(node) => match &node.effect {
                WrapperEffect::LayoutPriority(priority) => priority.get(),
                _ => node.child.priority(),
            },
            Self::Opacity(node) => node.child.priority(),
            Self::Scale(node) => node.child.priority(),
            Self::Rotation(node) => node.child.priority(),
            Self::Offset(node) => node.child.priority(),
            Self::Retain(node) => node.child.priority(),
            Self::Env(node) => node.child.priority(),
            Self::Dynamic(node) => node.child.borrow().priority(),
            Self::Filtered(node) => node.child.priority(),
            Self::Widget(node) => node.behavior.priority(),
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
            Self::Widget(node) => node.behavior.renders_nothing(),
            Self::Opacity(node) => node.child.is_empty(),
            Self::Scale(node) => node.child.is_empty(),
            Self::Rotation(node) => node.child.is_empty(),
            Self::Offset(node) => node.child.is_empty(),
            Self::Retain(node) => node.child.is_empty(),
            Self::Env(node) => node.child.is_empty(),
            Self::Dynamic(node) => node.child.borrow().is_empty(),
            Self::Wrapper(node) => node.child.is_empty(),
            // A filter over a child that draws nothing draws nothing itself.
            Self::Filtered(node) => node.child.is_empty(),
            // A container is always a member — even a frame wrapping `()`
            // explicitly claims its configured slot, like a `Spacer` does —
            // and every variant not matched above draws nothing either.
            _ => false,
        }
    }

    pub(super) fn stretch(&self) -> StretchAxis {
        match self {
            Self::Color(_) | Self::Scroll(_) => StretchAxis::Both,
            // The same stack laid out eagerly is content-sized on both axes, so a
            // lazy one has to be too: making a stack virtualizable must not change
            // how it sizes. Rows that want the full cross axis ask for it
            // themselves, exactly as they do in the eager path.
            Self::Text(_) | Self::LazyStack(_) => StretchAxis::None,
            Self::Container(container) => {
                // The retained children answer for themselves, so a transparent
                // layout reports what its content currently claims rather than
                // what it claimed when the tree was built.
                let child_axes: Vec<StretchAxis> =
                    container.children.iter().map(Self::stretch).collect();
                container.layout.stretch_axis(&child_axes)
            }
            Self::Opacity(node) => node.child.stretch(),
            Self::Scale(node) => node.child.stretch(),
            Self::Rotation(node) => node.child.stretch(),
            Self::Offset(node) => node.child.stretch(),
            Self::Retain(node) => node.child.stretch(),
            Self::Env(node) => node.child.stretch(),
            Self::Dynamic(node) => node.child.borrow().stretch(),
            // Scene content that is naturally a size is content-sized and claims
            // no leftover space; content that has no size of its own fills.
            Self::SceneView(node) => scene_stretch_axis(node.content.borrow().intrinsic_size()),
            // GPU content fills its proposal; a filtered view is a
            // layout-transparent wrapper delegating to its child.
            Self::GpuContent(node) => {
                scene_stretch_axis(node.runtime.borrow().view.intrinsic_size())
            }
            Self::ExternalFrame(node) => {
                scene_stretch_axis(node.runtime.borrow().view.intrinsic_size())
            }
            Self::Filtered(node) => node.child.stretch(),
            Self::Collection(node) => {
                // A collection's membership is reactive; its layout is one of the
                // content-sized stacks, which ignores the children anyway.
                node.layout.stretch_axis(&[])
            }
            Self::Wrapper(node) => node.child.stretch(),
            Self::Widget(node) => node.stretch,
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
            Self::Container(node) => (&node.memo_gate, &node.memo_slots),
            Self::Scroll(node) => (&node.memo_gate, &node.memo_slots),
            Self::Collection(node) => (&node.memo_gate, &node.memo_slots),
            Self::LazyStack(node) => (&node.memo_gate, &node.memo_slots),
            Self::Text(node) => (&node.memo_gate, &node.memo_slots),
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
    #[expect(
        clippy::too_many_lines,
        reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
    )]
    pub(crate) fn measure_body(
        &self,
        state: &mut HydroState,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        state.counters.measure_calls += 1;
        match self {
            Self::Color(_) => ViewDimensions::new(Size::new(
                proposal.width.unwrap_or(0.0),
                proposal.height.unwrap_or(0.0),
            )),
            Self::Text(text) => HydrolysisRenderer::measure_text_dimensions(
                state,
                text.content.snapshot(),
                text.alignment.snapshot(),
                env,
                proposal.width,
                text.line_limit,
            ),
            Self::Container(container) => {
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
            Self::Opacity(node) => node.child.measure(state, env, theme, proposal),
            Self::Scale(node) => node.child.measure(state, env, theme, proposal),
            Self::Rotation(node) => node.child.measure(state, env, theme, proposal),
            Self::Offset(node) => node.child.measure(state, env, theme, proposal),
            Self::Retain(node) => node.child.measure(state, env, theme, proposal),
            Self::Env(node) => node.child.measure(state, &node.env, theme, proposal),
            Self::Dynamic(node) => {
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
            Self::SceneView(node) => {
                let resolved =
                    resolve_scene_proposal(node.content.borrow().intrinsic_size(), proposal);
                ViewDimensions::new(Size::new(
                    resolved.width.unwrap_or(0.0),
                    resolved.height.unwrap_or(0.0),
                ))
            }
            // GPU content answers like a scene: content that is naturally a
            // size is content-sized; content without one fills the proposal.
            Self::GpuContent(node) => {
                let resolved =
                    resolve_scene_proposal(node.runtime.borrow().view.intrinsic_size(), proposal);
                ViewDimensions::new(Size::new(
                    resolved.width.unwrap_or(0.0),
                    resolved.height.unwrap_or(0.0),
                ))
            }
            // External frames measure through their source: a stream with an
            // intrinsic size is content-sized, one without fills the proposal.
            Self::ExternalFrame(node) => node.runtime.borrow().view.measure(proposal),
            // A filtered view is sized by its content: the engine applies the
            // filter to the mount the child's layers hang from.
            Self::Filtered(node) => node.child.measure(state, &node.env, theme, proposal),
            Self::Scroll(node) => {
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
            Self::LazyStack(node) => node.measure(state, theme, proposal),
            Self::Collection(node) => node.measure(state, theme, proposal),
            // Layout-transparent: the wrapper measures its child under the node's
            // scoped environment (effect colors/a11y read env every frame).
            Self::Wrapper(node) => node.child.measure(state, &node.env, theme, proposal),
            Self::Widget(node) => node.behavior.measure(state, proposal, &node.env, theme),
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
                Self::Container(container) => return Some(&**container),
                Self::Env(inner) => node = &inner.child,
                Self::Wrapper(inner) => node = &inner.child,
                Self::Retain(inner) => node = &inner.child,
                Self::Opacity(inner) => node = &inner.child,
                Self::Scale(inner) => node = &inner.child,
                Self::Rotation(inner) => node = &inner.child,
                Self::Offset(inner) => node = &inner.child,
                Self::Filtered(inner) => node = &inner.child,
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
            Self::Widget(node) => node.behavior.prepare(renderer, &node.env),
            Self::Opacity(node) => node.child.prepare_for_measure(renderer),
            Self::Scale(node) => node.child.prepare_for_measure(renderer),
            Self::Rotation(node) => node.child.prepare_for_measure(renderer),
            Self::Offset(node) => node.child.prepare_for_measure(renderer),
            Self::Retain(node) => node.child.prepare_for_measure(renderer),
            Self::Dynamic(node) => node.child.borrow_mut().prepare_for_measure(renderer),
            Self::Env(node) => node.child.prepare_for_measure(renderer),
            Self::Wrapper(node) => node.child.prepare_for_measure(renderer),
            Self::Filtered(node) => node.child.prepare_for_measure(renderer),
            Self::Container(node) => {
                for child in &mut node.children {
                    child.prepare_for_measure(renderer);
                }
            }
            Self::Scroll(node) => node.child.prepare_for_measure(renderer),
            Self::Collection(node) => {
                for entry in &mut node.entries {
                    entry.node.prepare_for_measure(renderer);
                }
            }
            Self::LazyStack(node) => {
                node.item_cache.borrow_mut().prepare_for_measure(renderer);
            }
            Self::Color(_)
            | Self::Text(_)
            | Self::SceneView(_)
            | Self::GpuContent(_)
            | Self::ExternalFrame(_) => {}
        }
    }

    /// Re-measure and re-place this subtree, caching each container's child
    /// frames. Run on build and whenever a geometry-affecting input changes.
    ///
    /// `safe_area` is the §7.1 context this node lays out against — its
    /// laid-out frame in window space plus, per edge, the boundary the
    /// subtree's area ends at — or `None` where there is none: inside a
    /// scroll surface's content and inside widget-owned retained
    /// sub-views.
    #[expect(
        clippy::too_many_lines,
        reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
    )]
    pub(crate) fn layout(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
        safe_area: Option<SafeAreaLayout>,
        proposal: ProposalSize,
        size: Size,
    ) {
        renderer.state.counters.layout_calls += 1;
        // The selected proposal and resolved size are distinct layout inputs.
        // Transparent wrappers preserve both without reconstructing an offer.
        let theme = renderer.theme();
        match self {
            Self::Container(container) => {
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
                    let child_area = safe_area.as_ref().map(|area| {
                        area.with_frame(kurbo::Rect::new(
                            area.frame().x0 + f64::from(placement.frame.x()),
                            area.frame().y0 + f64::from(placement.frame.y()),
                            area.frame().x0 + f64::from(placement.frame.max_x()),
                            area.frame().y0 + f64::from(placement.frame.max_y()),
                        ))
                    });
                    child.layout(
                        renderer,
                        env,
                        child_area,
                        placement.proposal,
                        *placement.frame.size(),
                    );
                }
                container.placed = placements
                    .into_iter()
                    .map(|placement| placement.frame)
                    .collect();
            }
            // Transform/opacity wrappers are layout-transparent: the child lays out
            // at the same concrete size as the wrapper, inside the same
            // laid-out frame — visual transforms are not part of §7.1's
            // "laid-out frame" and never change the touch test.
            Self::Opacity(node) => node.child.layout(renderer, env, safe_area, proposal, size),
            Self::Scale(node) => node.child.layout(renderer, env, safe_area, proposal, size),
            Self::Rotation(node) => node.child.layout(renderer, env, safe_area, proposal, size),
            Self::Offset(node) => node.child.layout(renderer, env, safe_area, proposal, size),
            Self::Retain(node) => node.child.layout(renderer, env, safe_area, proposal, size),
            Self::Env(node) => {
                let node_env = node.env.clone();
                node.child
                    .layout(renderer, &node_env, safe_area, proposal, size);
            }
            // Layout-transparent: the child lays out at the same concrete size,
            // under the wrapper's scoped environment — except `.ignore_safe_area`,
            // which moves the subtree's boundary out on each named edge the
            // wrapper's laid-out frame touches, stopping at the deepest region
            // the declaration does not name (§7.1's "layout avoids"). An edge
            // the frame does not touch releases nothing, and released regions
            // accumulate, so nested declarations cannot double-release.
            Self::Wrapper(node) => {
                let node_env = node.env.clone();
                if let WrapperEffect::IgnoreSafeArea(ignore) = &node.effect {
                    let Some(area) = safe_area else {
                        node.released_offsets.set(EdgeOffsets::default());
                        node.child.layout(renderer, &node_env, None, proposal, size);
                        return;
                    };
                    let (_, child_area, released) = area.release(*ignore);
                    node.released_offsets.set(released);
                    let child_size = released_size(size, released);
                    let child_proposal = ProposalSize::new(
                        proposal.width.map(|_| child_size.width),
                        proposal.height.map(|_| child_size.height),
                    );
                    node.child.layout(
                        renderer,
                        &node_env,
                        Some(child_area),
                        child_proposal,
                        child_size,
                    );
                } else {
                    node.child
                        .layout(renderer, &node_env, safe_area, proposal, size);
                }
            }
            Self::Dynamic(node) => {
                // The context stays on the host: the flush's mid-pass layout
                // for a child that applied its pending then reuses it.
                (*node.safe_area.borrow_mut()).clone_from(&safe_area);
                node.child
                    .borrow_mut()
                    .layout(renderer, env, safe_area, proposal, size);
            }
            Self::Scroll(node) => {
                let child_proposal = match node.axis {
                    ScrollAxis::Horizontal => ProposalSize::new(None, Some(size.height)),
                    ScrollAxis::Vertical => ProposalSize::new(Some(size.width), None),
                    ScrollAxis::All => ProposalSize::UNSPECIFIED,
                    _ => panic!("hydrolysis render tree: unsupported scroll axis"),
                };
                let intrinsic = node
                    .child
                    .measure(&mut renderer.state, &node.env, &theme, child_proposal)
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
                // §7.1's scroll surface: the extension is computed once
                // here — the edges whose laid-out frame touches the
                // subtree boundary reach the window edge — and the handle
                // is rebound exactly once, with the extended viewport and
                // content, so an input closure captured mid-frame never
                // meets a second generation bump at flush.
                let facts = safe_area.as_ref().map(SafeAreaLayout::surface_facts);
                node.surface.facts.set(facts);
                let extension = facts.map_or_else(EdgeOffsets::default, |facts| facts.extension);
                let viewport_width = f64::from(size.width) + extension.horizontal();
                let viewport_height = f64::from(size.height) + extension.vertical();
                let content_width = f64::from(content_size.width) + extension.horizontal();
                let content_height = f64::from(content_size.height) + extension.vertical();
                let handle = if let Some(handle) = node.handle.borrow_mut().as_mut() {
                    handle.rebind(
                        node.axis,
                        viewport_width,
                        viewport_height,
                        content_width,
                        content_height,
                    )
                } else {
                    // `report_offset`: the handle writes the content offset
                    // into this binding on every change, glide frames
                    // included, from here on.
                    ScrollHandle::new(
                        node.axis,
                        viewport_width,
                        viewport_height,
                        content_width,
                        content_height,
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
                // The content sees no safe-area context: the surface
                // insets and clears its own subtree, so nothing inside
                // touches an edge (§7.1).
                node.child
                    .layout(renderer, &node.env, None, child_proposal, content_size);
            }
            Self::Collection(node) => node.layout(renderer, safe_area.as_ref(), proposal, size),
            Self::Filtered(node) => {
                let node_env = node.env.clone();
                node.child
                    .layout(renderer, &node_env, safe_area, proposal, size);
            }
            Self::Color(node) => {
                // §7.1's fill rule, layout side: a fill in a background
                // slot records the extension its laid-out frame earns —
                // the flush grows the paint rect by exactly this.
                if node.fill_extension.get().is_some() {
                    node.fill_extension
                        .set(Some(safe_area.map_or_else(EdgeOffsets::default, |area| {
                            area.touched_edge_offsets()
                        })));
                }
            }
            Self::Widget(node) => {
                node.behavior
                    .update_scroll_surface(safe_area.as_ref().map(SafeAreaLayout::surface_facts));
                if node.fill_extension.get().is_some() {
                    node.fill_extension
                        .set(Some(safe_area.map_or_else(EdgeOffsets::default, |area| {
                            area.touched_edge_offsets()
                        })));
                }
            }
            // A lazy stack places its items lazily at flush (offset-dependent);
            // text and GPU leaves render at flush from `ctx.bounds`.
            Self::Text(_)
            | Self::SceneView(_)
            | Self::GpuContent(_)
            | Self::ExternalFrame(_)
            | Self::LazyStack(_) => {}
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
    pub(super) const fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub(super) const fn mix(&mut self, bits: u64) {
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
            Self::Color(_)
            | Self::Text(_)
            | Self::SceneView(_)
            | Self::GpuContent(_)
            | Self::ExternalFrame(_)
            | Self::Widget(_) => {}
            Self::Opacity(node) => node.child.signature_into(frame, hasher),
            Self::Scale(node) => node.child.signature_into(frame, hasher),
            Self::Rotation(node) => node.child.signature_into(frame, hasher),
            Self::Offset(node) => node.child.signature_into(frame, hasher),
            Self::Retain(node) => node.child.signature_into(frame, hasher),
            Self::Env(node) => node.child.signature_into(frame, hasher),
            Self::Wrapper(node) => node.child.signature_into(frame, hasher),
            Self::Dynamic(node) => node.child.borrow().signature_into(frame, hasher),
            Self::Filtered(node) => node.child.signature_into(frame, hasher),
            Self::Container(node) => {
                node.placed.len().hash(hasher);
                for (child, rect) in node.children.iter().zip(&node.placed) {
                    child.signature_into(*rect, hasher);
                }
            }
            Self::Collection(node) => {
                node.placed.len().hash(hasher);
                for (entry, rect) in node.entries.iter().zip(&node.placed) {
                    entry.node.signature_into(*rect, hasher);
                }
            }
            Self::Scroll(node) => {
                hash_size(hasher, node.content_size);
                hash_size(hasher, node.viewport);
                node.child
                    .signature_into(Rect::from_size(node.content_size), hasher);
            }
            Self::LazyStack(node) => node.item_cache.borrow().signature_into(hasher),
        }
    }
}
