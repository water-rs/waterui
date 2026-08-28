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

/// Emits the image accessibility node shared by the static graphics leaves
/// (gradient, shape, morph shape). Their a11y is *not* render-driven: the leaf
/// emits a single `Image` node from the surrounding environment's label, or the
/// leaf-provided default when no override is installed. Shared by the dispatch
/// path and the retained `Widget`-node path so both produce the same a11y tree.
pub(crate) fn graphics_image_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    env: &Environment,
    default_label: Option<String>,
) {
    #[cfg(feature = "accessibility")]
    {
        let mut node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Image),
        );
        if let Some(label) = renderer.resolve_accessibility_label(env, default_label) {
            node.set_label(label);
        }
        let _ = renderer.register_accessibility_node(
            node,
            transformed_rect(ctx.hit_transform, ctx.bounds),
            env,
            None,
        );
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, env, default_label);
    }
}

/// Graphics leaves (gradient/shape/morph/GPU surface/scene) fill the proposal:
/// they stretch to whatever bounds the layout proposes (`StretchAxis::Both`).
pub(crate) fn graphics_dimensions_from_proposal(proposal: ProposalSize) -> ViewDimensions {
    ViewDimensions::new(LayoutSize::new(
        proposal
            .width
            .filter(|width| width.is_finite())
            .unwrap_or(0.0)
            .max(0.0),
        proposal
            .height
            .filter(|height| height.is_finite())
            .unwrap_or(0.0)
            .max(0.0),
    ))
}

/// Measures a retained gradient leaf: a gradient fills the proposed bounds.
pub(crate) fn measure_gradient_node(
    _gradient: &ResolvedGradient,
    proposal: ProposalSize,
    _state: &mut HydroState,
    _env: &Environment,
) -> ViewDimensions {
    graphics_dimensions_from_proposal(proposal)
}

/// Renders a retained gradient leaf every flush: emits a11y (unless hidden) then
/// fills the gradient. The payload is fully resolved data (no signal), so no watch.
pub(crate) fn render_gradient_node(
    ctx: &mut WidgetRenderContext<'_>,
    gradient: &Rc<RefCell<ResolvedGradient>>,
    env: &Environment,
) {
    let hidden = env
        .get::<AccessibilityHidden>()
        .is_some_and(AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        graphics_image_accessibility(ctx.renderer_mut(), render_ctx, env, None);
    }
    render_gradient_parts(ctx, gradient, env);
}

pub(crate) fn render_gradient_parts(
    ctx: &mut WidgetRenderContext<'_>,
    gradient: &Rc<RefCell<ResolvedGradient>>,
    _env: &Environment,
) {
    let bounds = ctx.bounds;
    let brush = resolved_gradient_to_brush(&gradient.borrow(), bounds);
    let transform = ctx.transform;
    ctx.renderer_mut().scene.fill(
        vello::peniko::Fill::NonZero,
        transform,
        &brush,
        None,
        &bounds,
    );
}

/// Measures a retained shape leaf: a shape fills the proposed bounds.
pub(crate) fn measure_shape_node(
    _shape: &ResolvedShape,
    proposal: ProposalSize,
    _state: &mut HydroState,
    _env: &Environment,
) -> ViewDimensions {
    graphics_dimensions_from_proposal(proposal)
}

/// Renders a retained shape leaf every flush: emits a11y (unless hidden), tracks
/// the resolved fill signal, then fills the shape path.
pub(crate) fn render_shape_node(
    ctx: &mut WidgetRenderContext<'_>,
    shape: &Rc<RefCell<ResolvedShape>>,
    env: &Environment,
) {
    let hidden = env
        .get::<AccessibilityHidden>()
        .is_some_and(AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        graphics_image_accessibility(ctx.renderer_mut(), render_ctx, env, None);
    }
    render_shape_parts(ctx, shape, env);
}

pub(crate) fn render_shape_parts(
    ctx: &mut WidgetRenderContext<'_>,
    shape: &Rc<RefCell<ResolvedShape>>,
    _env: &Environment,
) {
    let bounds = ctx.bounds;
    let (path, fill_signal) = {
        let resolved = shape.borrow();
        (
            resolved_shape_to_path(&resolved, bounds),
            resolved.fill.clone(),
        )
    };
    let fill = resolved_color_to_peniko(ctx.renderer_mut().read_signal(&fill_signal));
    let transform = ctx.transform;
    ctx.renderer_mut()
        .scene
        .fill(vello::peniko::Fill::NonZero, transform, fill, None, &path);
}

/// Measures a retained morph-shape leaf: a morph shape fills the proposed bounds.
pub(crate) fn measure_morph_shape_node(
    _shape: &ResolvedMorphShape,
    proposal: ProposalSize,
    _state: &mut HydroState,
    _env: &Environment,
) -> ViewDimensions {
    graphics_dimensions_from_proposal(proposal)
}

/// Renders a retained morph-shape leaf every flush: emits a11y (unless hidden) then
/// fills the morphed path at the current animation progress. The morph progress is
/// resolved via the animation controller every frame (explicit `progress` signal
/// watched through `resolve_animated_scalar_with_discriminator`; time-based
/// animation driven by `sample_morph_progress`), so the morph stays live.
pub(crate) fn render_morph_shape_node(
    ctx: &mut WidgetRenderContext<'_>,
    shape: &Rc<RefCell<ResolvedMorphShape>>,
    env: &Environment,
) {
    let hidden = env
        .get::<AccessibilityHidden>()
        .is_some_and(AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        graphics_image_accessibility(ctx.renderer_mut(), render_ctx, env, None);
    }
    render_morph_shape_parts(ctx, shape, env);
}

pub(crate) fn render_morph_shape_parts(
    ctx: &mut WidgetRenderContext<'_>,
    shape: &Rc<RefCell<ResolvedMorphShape>>,
    _env: &Environment,
) {
    let bounds = ctx.bounds;
    let transform = ctx.transform;
    // Stable identity of this morph node: the retained shape `Rc`'s address keys the
    // time-based morph slot so it survives structural changes (no `render_depth`).
    let node_id = Rc::as_ptr(shape) as usize;
    let renderer = ctx.renderer_mut();
    let (path, fill) = {
        let resolved = shape.borrow();
        let progress = if let Some(progress) = resolved.progress.as_ref() {
            renderer
                .resolve_animated_scalar_with_discriminator(progress, MORPH_PROGRESS_ANIMATION_KEY)
        } else {
            renderer.sample_morph_progress(resolved.animation, node_id)
        };
        let fill = renderer.read_signal(&resolved.fill);
        (
            resolved_morph_shape_to_path(&resolved, progress, bounds),
            resolved_color_to_peniko(fill),
        )
    };
    renderer
        .scene
        .fill(vello::peniko::Fill::NonZero, transform, fill, None, &path);
}

/// Emits a string leaf's accessibility node from its content. Shared by the
/// dispatch path ([`HydrolysisRenderer::render_str`]) and the retained
/// `Widget`-node path so both produce the same a11y tree.
pub(crate) fn str_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    text: &Str,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        if env
            .get::<AccessibilityHidden>()
            .is_some_and(AccessibilityHidden::is_hidden)
        {
            return;
        }
        let label = renderer.resolve_accessibility_label(env, Some(text.as_str().to_owned()));
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
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, text, env);
    }
}

/// Measures a retained string leaf from its immutable content, mirroring how
/// [`measure_view_dimensions_with_proposal`] measures a `Str` (plain styled text,
/// leading alignment, wrapped at the proposal width).
pub(crate) fn measure_str_node(
    text: &Str,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    HydrolysisRenderer::measure_text_dimensions(
        state,
        StyledStr::plain(text.clone()),
        HorizontalAlignment::Leading,
        env,
        proposal.width,
        None,
    )
}

/// Renders a retained string leaf every flush: emits a11y (unless hidden) then the
/// plain styled text. The content is an immutable `Str`, so no signal watch is
/// needed — it never changes for a given node.
pub(crate) fn render_str_node(
    ctx: &mut WidgetRenderContext<'_>,
    text: &Rc<RefCell<Str>>,
    env: &Environment,
) {
    let render_ctx = ctx.render_context();
    str_accessibility(ctx.renderer_mut(), render_ctx, &text.borrow(), env);
    render_str_parts(ctx, text, env);
}

pub(crate) fn render_str_parts(
    ctx: &mut WidgetRenderContext<'_>,
    text: &Rc<RefCell<Str>>,
    env: &Environment,
) {
    let styled = StyledStr::plain(text.borrow().clone());
    let render_ctx = ctx.render_context();
    let (state, scene) = ctx.renderer_mut().state_and_scene_mut();
    HydrolysisRenderer::render_styled_text(
        state,
        scene,
        render_ctx,
        styled,
        HorizontalAlignment::Leading,
        env,
    );
}
