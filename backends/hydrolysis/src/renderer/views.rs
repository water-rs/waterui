//! Raw view handlers: layout containers, text, icons, colors, gradients
//! and shapes, plus popup-menu node resolution.

use super::*;

pub(crate) fn slider_value_epsilon(span: f64, track_width: f64) -> f64 {
    (span / track_width).abs().max(f64::EPSILON)
}

pub(super) fn call_action_discarding_result<T: 'static>(
    action: &SharedAction<T>,
    env: &Environment,
) {
    let _ = action.call(env);
}

pub(crate) fn popup_menu_nodes(items: &[ResolvedMenuItem]) -> Vec<PopupMenuNode> {
    items.iter().cloned().map(popup_menu_node).collect()
}

pub(crate) fn popup_menu_node(item: ResolvedMenuItem) -> PopupMenuNode {
    match item {
        ResolvedMenuItem::Command(command) => {
            let mut styled = command.label.content.get();
            if command.selected.get() {
                styled = StyledStr::plain("✓ ") + styled;
            }
            let plain_label = styled.to_plain().to_string();
            let label = command.semantic_label.text(Text::new(styled));
            PopupMenuNode::Command {
                label,
                plain_label,
                action: command.action,
                disabled: command.disabled.get(),
            }
        }
        ResolvedMenuItem::Divider => PopupMenuNode::Divider,
        ResolvedMenuItem::Menu(menu) => {
            let styled = menu.label.content.get() + StyledStr::plain(" ›");
            let plain_label = styled.to_plain().to_string();
            let label = menu.semantic_label.text(Text::new(styled));
            PopupMenuNode::Menu {
                label,
                plain_label,
                items: popup_menu_nodes(&menu.items.get()),
            }
        }
    }
}

impl HydrolysisRenderer {
    pub(super) fn canonical_geometry_bits(value: f64) -> u64 {
        if value == 0.0 {
            0.0f64.to_bits()
        } else {
            value.to_bits()
        }
    }

    pub(super) fn lazy_stack_slot_key(&self, ctx: RenderContext) -> LazyStackSlotKey {
        let [scale_x, skew_y, skew_x, scale_y, translate_x, translate_y] =
            ctx.transform.as_coeffs();
        LazyStackSlotKey {
            depth: self.render_depth,
            transform: [
                Self::canonical_geometry_bits(scale_x),
                Self::canonical_geometry_bits(skew_y),
                Self::canonical_geometry_bits(skew_x),
                Self::canonical_geometry_bits(scale_y),
                Self::canonical_geometry_bits(translate_x),
                Self::canonical_geometry_bits(translate_y),
            ],
            bounds: [
                Self::canonical_geometry_bits(ctx.bounds.x0),
                Self::canonical_geometry_bits(ctx.bounds.y0),
                Self::canonical_geometry_bits(ctx.bounds.x1),
                Self::canonical_geometry_bits(ctx.bounds.y1),
            ],
        }
    }

    pub(super) fn render_layout_container(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        layout: Box<dyn Layout>,
        children: Vec<AnyView>,
        env: &Environment,
    ) {
        let mut resolved_children = Vec::with_capacity(children.len());
        for child in children {
            resolved_children.push(normalize_layout_view(child, env));
        }

        let state = RefCell::new(&mut renderer.state);
        let mut subviews = Vec::with_capacity(resolved_children.len());
        for child in &resolved_children {
            subviews.push(HydroSubview::from_view(child, &state, env));
        }
        let refs: Vec<&dyn SubView> = subviews.iter().map(|view| view as &dyn SubView).collect();

        let proposal = ProposalSize::new(
            Some(ctx.bounds.width() as f32),
            Some(ctx.bounds.height() as f32),
        );
        let layout_size = layout.size_that_fits(proposal, &refs);
        let stretch_axis = layout.stretch_axis();
        let width = if matches!(stretch_axis, StretchAxis::Horizontal | StretchAxis::Both) {
            ctx.bounds.width() as f32
        } else {
            layout_size.width.min(ctx.bounds.width() as f32)
        };
        let height = if matches!(stretch_axis, StretchAxis::Vertical | StretchAxis::Both) {
            ctx.bounds.height() as f32
        } else {
            layout_size.height.min(ctx.bounds.height() as f32)
        };
        let bounds = LayoutRect::from_size(LayoutSize::new(width, height));
        let child_rects = layout.place(bounds, &refs);

        // Release the measurement borrows (subviews hold the renderer state via
        // `MainThreadBound`, which carries drop glue) before re-borrowing `renderer`
        // mutably to dispatch children. Dropping `subviews` lets NLL end the
        // `state -> renderer` borrow; `refs` is already dead after `place`.
        drop(refs);
        drop(subviews);

        for ((_index, child), rect) in resolved_children.into_iter().enumerate().zip(child_rects) {
            let child_transform =
                vello::kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y())));
            let child_bounds = vello::kurbo::Rect::new(
                0.0,
                0.0,
                f64::from(rect.width()),
                f64::from(rect.height()),
            );
            Self::dispatch_any(
                renderer,
                ctx.child(child_transform, child_bounds),
                env,
                child,
            );
        }
    }

    pub(crate) fn render_fixed_container(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        container: Native<FixedContainer>,
        env: &Environment,
    ) {
        let (layout, children) = container.into_inner().into_inner();
        Self::render_layout_container(renderer, ctx, layout, children, env);
    }

    pub(crate) fn render_lazy_container(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        container: Native<LazyContainer>,
        env: &Environment,
    ) {
        let (layout, children) = container.into_inner().into_inner();
        // A collection opting into a membership transition must retain its items
        // (to keep exiting items collapsing out and entering items growing in), so
        // it always uses the retained per-id collection path even for a stack
        // layout that would otherwise be viewport-virtualized.
        let wants_transition = env
            .get::<waterui_layout::collection_transition::CollectionTransition>()
            .is_some();
        let Some(axis_config) = lazy_stack_axis_config(layout.as_ref()).filter(|_| !wants_transition)
        else {
            // A non-virtualized layout (AbsoluteLayout/ZStackLayout overlay) or a
            // transition-enabled collection: render as a retained per-id collection
            // so membership changes patch incrementally instead of re-dispatching
            // the whole set.
            Self::capture_collection(renderer, ctx, layout, children, env);
            return;
        };
        // Only the viewport-virtualized stack path depends on the scroll viewport.
        renderer.mark_scroll_content_viewport_dependent();
        let count = children.len().get();
        if count == 0 {
            return;
        }
        let visible_bounds = {
            renderer
                .lazy
                .lazy_viewport_stack
                .last()
                .copied()
                .unwrap_or(ctx.bounds)
        };
        let slot_key = renderer.lazy_stack_slot_key(ctx);
        renderer
            .lazy
            .lazy_stack_controller
            .bind(slot_key)
            .prepare_len(count);
        let (visible_start, visible_end, spacing) = match axis_config {
            LazyStackAxisConfig::Vertical { spacing, .. } => {
                (visible_bounds.y0, visible_bounds.y1, spacing)
            }
            LazyStackAxisConfig::Horizontal { spacing, .. } => {
                (visible_bounds.x0, visible_bounds.x1, spacing)
            }
        };
        let window = resolve_visible_index_window(count, visible_start, visible_end, |index| {
            let cached_extent = {
                renderer
                    .lazy
                    .lazy_stack_controller
                    .slot(slot_key)
                    .item_extents[index]
            };
            let extent = if let Some(extent) = cached_extent {
                extent
            } else {
                let child = children.get_view(index).unwrap_or_else(|| {
                    panic!("LazyContainer failed to materialize child at index {index}")
                });
                let child = normalize_layout_view(child, env);
                let state = RefCell::new(&mut renderer.state);
                let subview = HydroSubview::from_view(&child, &state, env);
                let proposal = match axis_config {
                    LazyStackAxisConfig::Vertical { .. } => {
                        ProposalSize::new(Some(ctx.bounds.width() as f32), None)
                    }
                    LazyStackAxisConfig::Horizontal { .. } => {
                        ProposalSize::new(None, Some(ctx.bounds.height() as f32))
                    }
                };
                let size = subview.measure(proposal).size;
                let extent = match axis_config {
                    LazyStackAxisConfig::Vertical { .. } => f64::from(size.height),
                    LazyStackAxisConfig::Horizontal { .. } => f64::from(size.width),
                };
                renderer
                    .lazy
                    .lazy_stack_controller
                    .slot_mut(slot_key)
                    .item_extents[index] = Some(extent);
                extent
            };
            if index + 1 < count {
                extent + spacing
            } else {
                extent
            }
        });

        let mut cursor = window.leading_offset;
        for index in window.start..window.end {
            let child = children.get_view(index).unwrap_or_else(|| {
                panic!("LazyContainer failed to materialize child at index {index}")
            });
            let child = normalize_layout_view(child, env);
            let state = RefCell::new(&mut renderer.state);
            let subview = HydroSubview::from_view(&child, &state, env);
            let proposal = match axis_config {
                LazyStackAxisConfig::Vertical { .. } => {
                    ProposalSize::new(Some(ctx.bounds.width() as f32), None)
                }
                LazyStackAxisConfig::Horizontal { .. } => {
                    ProposalSize::new(None, Some(ctx.bounds.height() as f32))
                }
            };
            let size = subview.measure(proposal).size;
            let child_rect = match axis_config {
                LazyStackAxisConfig::Vertical { alignment, .. } => {
                    assert!(
                        !(matches!(
                            subview.stretch_axis(),
                            StretchAxis::Vertical | StretchAxis::Both | StretchAxis::MainAxis
                        )),
                        "hydrolysis LazyContainer VStackLayout does not support children stretching on main axis"
                    );
                    let child_width = if matches!(
                        subview.stretch_axis(),
                        StretchAxis::Horizontal | StretchAxis::Both | StretchAxis::CrossAxis
                    ) || size.width.is_infinite()
                    {
                        ctx.bounds.width()
                    } else {
                        f64::from(size.width).min(ctx.bounds.width())
                    };
                    let child_height = f64::from(size.height);
                    let x = if alignment == HorizontalAlignment::Leading {
                        ctx.bounds.x0
                    } else if alignment == HorizontalAlignment::Trailing {
                        ctx.bounds.x1 - child_width
                    } else {
                        ctx.bounds.x0 + (ctx.bounds.width() - child_width) / 2.0
                    };
                    vello::kurbo::Rect::new(x, cursor, x + child_width, cursor + child_height)
                }
                LazyStackAxisConfig::Horizontal { alignment, .. } => {
                    assert!(
                        !(matches!(
                            subview.stretch_axis(),
                            StretchAxis::Horizontal | StretchAxis::Both | StretchAxis::MainAxis
                        )),
                        "hydrolysis LazyContainer HStackLayout does not support children stretching on main axis"
                    );
                    let child_width = f64::from(size.width);
                    let child_height = if matches!(
                        subview.stretch_axis(),
                        StretchAxis::Vertical | StretchAxis::Both | StretchAxis::CrossAxis
                    ) || size.height.is_infinite()
                    {
                        ctx.bounds.height()
                    } else {
                        f64::from(size.height).min(ctx.bounds.height())
                    };
                    let y = if alignment == VerticalAlignment::Top {
                        ctx.bounds.y0
                    } else if alignment == VerticalAlignment::Bottom {
                        ctx.bounds.y1 - child_height
                    } else {
                        ctx.bounds.y0 + (ctx.bounds.height() - child_height) / 2.0
                    };
                    vello::kurbo::Rect::new(cursor, y, cursor + child_width, y + child_height)
                }
            };
            // Release the measurement borrow (the subview holds renderer state via
            // `MainThreadBound` drop glue) before re-borrowing `renderer` and moving
            // `child`. Dropping `subview` lets NLL end the `state -> renderer` borrow.
            drop(subview);
            let extent = match axis_config {
                LazyStackAxisConfig::Vertical { .. } => child_rect.height(),
                LazyStackAxisConfig::Horizontal { .. } => child_rect.width(),
            };
            {
                renderer
                    .lazy
                    .lazy_stack_controller
                    .slot_mut(slot_key)
                    .item_extents[index] = Some(extent);
            }
            Self::dispatch_any(
                renderer,
                ctx.child(
                    vello::kurbo::Affine::translate((child_rect.x0, child_rect.y0)),
                    vello::kurbo::Rect::new(0.0, 0.0, child_rect.width(), child_rect.height()),
                ),
                env,
                child,
            );
            cursor += extent;
            if index + 1 < count {
                cursor += spacing;
            }
        }
    }

    pub(crate) fn render_str(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        text: Str,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            if !env
                .get::<AccessibilityHidden>()
                .is_some_and(AccessibilityHidden::is_hidden)
            {
                let label =
                    renderer.resolve_accessibility_label(env, Some(text.as_str().to_owned()));
                if let Some(label) = label {
                    let mut node = AccessibilityNode::new(
                        renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Label),
                    );
                    node.set_label(label);
                    let _ = renderer.register_accessibility_node(
                        node,
                        transformed_rect(ctx.hit_transform, ctx.bounds),
                        env,
                        None,
                    );
                }
            }
        }
        Self::render_styled_text(
            &mut renderer.state,
            &mut renderer.scene,
            ctx,
            StyledStr::plain(text),
            HorizontalAlignment::Leading,
            env,
        );
    }

    pub(crate) fn render_resolved_color(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        color: Native<ResolvedColor>,
        _env: &Environment,
    ) {
        let brush = resolved_color_to_peniko(color.into_inner());
        renderer.scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            brush,
            None,
            &ctx.bounds,
        );
    }

    pub(crate) fn render_resolved_gradient(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        gradient: Native<ResolvedGradient>,
        _env: &Environment,
    ) {
        let brush = resolved_gradient_to_brush(&gradient.into_inner(), ctx.bounds);
        renderer.scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            &brush,
            None,
            &ctx.bounds,
        );
    }

    pub(crate) fn render_resolved_shape(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        shape: Native<ResolvedShape>,
        _env: &Environment,
    ) {
        let resolved = shape.into_inner();
        let path = resolved_shape_to_path(&resolved, ctx.bounds);
        let fill = resolved_color_to_peniko(resolved.fill);
        renderer.scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            fill,
            None,
            &path,
        );
    }

    pub(crate) fn render_resolved_morph_shape(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        shape: Native<ResolvedMorphShape>,
        _env: &Environment,
    ) {
        let resolved = shape.into_inner();
        if resolved.progress.is_none() && renderer.dynamic_morph_capture_depth > 0 {
            renderer.dynamic_morph_draws.push(DynamicMorphDraw {
                shape: resolved,
                bounds: ctx.bounds,
                transform: ctx.transform,
                started_at: renderer.frame_instant,
            });
            return;
        }
        let progress = if let Some(progress) = resolved.progress.as_ref() {
            renderer
                .resolve_animated_scalar_with_discriminator(progress, MORPH_PROGRESS_ANIMATION_KEY)
        } else {
            renderer.sample_morph_progress(resolved.animation)
        };
        let path = resolved_morph_shape_to_path(&resolved, progress, ctx.bounds);
        let fill = resolved_color_to_peniko(resolved.fill);
        renderer.scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            fill,
            None,
            &path,
        );
    }
}
