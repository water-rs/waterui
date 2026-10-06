//! Raw view handlers: layout containers, text, icons, colors, gradients
//! and shapes, plus popup-menu node resolution.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::renderer::recording::transform_paint;
use waterui::app::Quit;

pub fn slider_value_epsilon(span: f64, track_width: f64) -> f64 {
    (span / track_width).abs().max(f64::EPSILON)
}

pub fn call_action_discarding_result<T: 'static>(action: &SharedAction<T>, env: &Environment) {
    let _ = action.call(env);
}

/// Resolved menu items as popup-menu nodes. A declared `MenuItem::Quit`
/// becomes the row of the quit command [`Quit::command`] builds, and is
/// omitted where `env` carries no application quit.
pub fn popup_menu_nodes(items: &[ResolvedMenuItem], env: &Environment) -> Vec<PopupMenuNode> {
    items
        .iter()
        .cloned()
        .filter_map(|item| popup_menu_node(item, env))
        .collect()
}

fn popup_menu_node(item: ResolvedMenuItem, env: &Environment) -> Option<PopupMenuNode> {
    match item {
        ResolvedMenuItem::Command(command) => Some(command_node(command)),
        ResolvedMenuItem::Quit => env
            .get::<Quit>()
            .map(|quit| command_node(quit.command(env))),
        ResolvedMenuItem::Divider => Some(PopupMenuNode::Divider),
        ResolvedMenuItem::Menu(menu) => {
            let styled = menu.label.content.snapshot() + StyledStr::plain(" ›");
            let plain_label = styled.to_plain().to_string();
            let semantic_text = menu.semantic_label.semantic_text().clone();
            let label = SemanticLabel::new(semantic_text, move || {
                AnyView::new(
                    waterui_layout::frame::Frame::new(Text::new(styled.clone()))
                        .alignment(waterui_layout::alignment::Leading)
                        .max_width(f32::INFINITY),
                )
            });
            Some(PopupMenuNode::Menu {
                label,
                plain_label,
                items: popup_menu_nodes(&menu.items.snapshot(), env),
            })
        }
    }
}

#[expect(
    clippy::option_if_let_else,
    reason = "the if-let/else mirrors the control flow more clearly than the combinator chain here"
)]
fn command_node(command: ResolvedCommand) -> PopupMenuNode {
    let mut styled = command.label.content.snapshot();
    if command.selected.snapshot() {
        styled = StyledStr::plain("✓ ") + styled;
    }
    // The destructive role lives in the styled label itself: an explicit
    // span colour survives the button's environment-level foreground,
    // so the theme's error colour wins (water-rs/hydrolysis#200).
    if command.role == CommandRole::Destructive {
        styled = styled.foreground(waterui::Color::new(waterui::theme::color::Error));
    }
    let plain_label = styled.to_plain().to_string();
    // `Label::new` keeps the semantic text for accessibility while the
    // custom content fills the row's label slot and aligns it leading —
    // plain and subtitled rows share one leading edge.
    let semantic_text = command.semantic_label.semantic_text().clone();
    let subtitle = command.subtitle.clone();
    let shortcut = command.shortcut.clone();
    let label = SemanticLabel::new(semantic_text, move || {
        let leading = match subtitle.clone() {
            Some(subtitle) => AnyView::new(
                waterui_layout::stack::vstack((
                    Text::new(styled.clone()),
                    waterui_text::text(subtitle).caption().muted(),
                ))
                .alignment(HorizontalAlignment::Leading)
                .spacing(0.0),
            ),
            None => AnyView::new(Text::new(styled.clone())),
        };
        AnyView::new(
            waterui_layout::frame::Frame::new(leading)
                .alignment(waterui_layout::alignment::Leading)
                .max_width(f32::INFINITY),
        )
    });
    PopupMenuNode::Command {
        label,
        plain_label,
        action: command.action,
        disabled: command.disabled,
        shortcut,
        subtitle: command.subtitle,
    }
}

/// Emits the image accessibility node shared by the static graphics leaves
/// (gradient, shape, morph shape). Their a11y is *not* render-driven: the leaf
/// emits a single `Image` node from the surrounding environment's label and
/// value, or the leaf-provided defaults where no override is installed. Shared
/// by the dispatch path and the retained `Widget`-node path so both produce the
/// same a11y tree.
///
/// A leaf that names nothing emits no node: decorative fills and shapes are
/// presentation, not semantics. Only a resolved label — the environment's
/// `a11y_label` or the leaf's own default — a resolved `a11y_value`, or an
/// explicit `a11y_role` puts a graphics leaf in the tree.
pub fn graphics_image_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    env: &Environment,
    default_label: Option<String>,
    default_value: Option<String>,
) {
    #[cfg(feature = "accessibility")]
    {
        let label = renderer.resolve_accessibility_label(env, default_label);
        let value = renderer.resolve_accessibility_value(env, default_value);
        if label.is_none() && value.is_none() && env.get::<AccessibilityRole>().is_none() {
            renderer.note_suppressed_graphics_leaf(ctx);
            return;
        }
        let mut node =
            AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::Image,
            ));
        if let Some(label) = label {
            node.set_label(label);
        }
        if let Some(value) = value {
            node.set_value(value);
        }
        let _ = renderer.register_accessibility_leaf(ctx, node, env, None);
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, env, default_label, default_value);
    }
}

/// Graphics leaves (gradient/shape/morph/GPU surface/scene) fill the proposal:
/// they stretch to whatever bounds the layout proposes (`StretchAxis::Both`).
pub fn graphics_dimensions_from_proposal(proposal: ProposalSize) -> ViewDimensions {
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
pub fn measure_gradient_node(
    _gradient: &waterui_graphics::Gradient,
    proposal: ProposalSize,
    _state: &mut HydroState,
    _env: &Environment,
    _theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    graphics_dimensions_from_proposal(proposal)
}

/// Renders a retained gradient leaf every flush: emits a11y (unless hidden) then
/// fills the gradient. The payload is fully resolved data (no signal), so no watch.
pub fn render_gradient_node(
    ctx: &mut WidgetRenderContext<'_>,
    gradient: &Rc<RefCell<waterui_graphics::Gradient>>,
    env: &Environment,
) {
    let hidden = env
        .get::<AccessibilityHidden>()
        .is_some_and(AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        graphics_image_accessibility(ctx.renderer_mut(), Some(render_ctx), env, None, None);
    }
    render_gradient_parts(ctx, gradient, env);
}

/// The gradient view's unit-space paint on a `width` × `height` box: points
/// map through [`Gradient::transform_to`], and a radial gradient's radii scale
/// by `min(width, height)` so a unit-space radius stays a circle.
fn gradient_paint_in_bounds(paint: Paint, width: f32, height: f32) -> Paint {
    let transform = Gradient::transform_to(width, height);
    match paint {
        Paint::Radial(mut radial) => {
            radial.start_center = transform * radial.start_center;
            radial.end_center = transform * radial.end_center;
            let scale = f64::from(width.min(height));
            radial.start_radius *= scale;
            radial.end_radius *= scale;
            Paint::Radial(radial)
        }
        other => transform_paint(other, Some(transform)),
    }
}

pub fn render_gradient_parts(
    ctx: &mut WidgetRenderContext<'_>,
    gradient: &Rc<RefCell<waterui_graphics::Gradient>>,
    _env: &Environment,
) {
    let bounds = ctx.bounds;
    // The view's `Paint` is authored in unit space; map it onto the placed
    // box, scaling radial radii by its shorter side, and fill it under the
    // frame's transform.
    let width = crate::num_cast::f64_as_f32(bounds.width());
    let height = crate::num_cast::f64_as_f32(bounds.height());
    let paint = gradient_paint_in_bounds(gradient.borrow().paint().clone(), width, height);
    let transform = ctx.transform;
    ctx.renderer_mut()
        .scene
        .fill_paint(peniko::Fill::NonZero, transform, paint, &bounds);
}

/// Measures a retained shape leaf: a shape fills the proposed bounds.
pub fn measure_shape_node(
    _shape: &ResolvedShape,
    proposal: ProposalSize,
    _state: &mut HydroState,
    _env: &Environment,
    _theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    graphics_dimensions_from_proposal(proposal)
}

/// Renders a retained shape leaf every flush: emits a11y (unless hidden), tracks
/// the resolved fill signal, then fills the shape path.
pub fn render_shape_node(
    ctx: &mut WidgetRenderContext<'_>,
    shape: &Rc<RefCell<ResolvedShape>>,
    env: &Environment,
) {
    let hidden = env
        .get::<AccessibilityHidden>()
        .is_some_and(AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        graphics_image_accessibility(ctx.renderer_mut(), Some(render_ctx), env, None, None);
    }
    render_shape_parts(ctx, shape, env);
}

pub fn render_shape_parts(
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
    let fill = waterui_graphics::draw::Paint::Solid(ctx.renderer_mut().read_signal(&fill_signal));
    let transform = ctx.transform;
    ctx.renderer_mut()
        .scene
        .fill_paint(peniko::Fill::NonZero, transform, fill, &path);
}

/// Measures a retained morph-shape leaf: a morph shape fills the proposed bounds.
pub fn measure_morph_shape_node(
    _shape: &ResolvedMorphShape,
    proposal: ProposalSize,
    _state: &mut HydroState,
    _env: &Environment,
    _theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    graphics_dimensions_from_proposal(proposal)
}

/// Renders a retained morph-shape leaf every flush: emits a11y (unless hidden) then
/// fills the morphed path at the current animation progress. The morph progress is
/// resolved via the animation controller every frame (explicit `progress` signal
/// watched through `resolve_animated_scalar_with_discriminator`; time-based
/// animation driven by `sample_morph_progress`), so the morph stays live.
pub fn render_morph_shape_node(
    ctx: &mut WidgetRenderContext<'_>,
    shape: &Rc<RefCell<ResolvedMorphShape>>,
    env: &Environment,
) {
    let hidden = env
        .get::<AccessibilityHidden>()
        .is_some_and(AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        graphics_image_accessibility(ctx.renderer_mut(), Some(render_ctx), env, None, None);
    }
    render_morph_shape_parts(ctx, shape, env);
}

pub fn render_morph_shape_parts(
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
            waterui_graphics::draw::Paint::Solid(fill),
        )
    };
    renderer
        .scene
        .fill_paint(peniko::Fill::NonZero, transform, fill, &path);
}

/// Emits a string leaf's accessibility node from its content. Shared by the
/// dispatch path ([`HydrolysisRenderer::render_str`]) and the retained
/// `Widget`-node path so both produce the same a11y tree.
// empty when the accessibility feature is off
#[cfg_attr(not(feature = "accessibility"), allow(clippy::missing_const_for_fn))]
pub fn str_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
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
        // An empty text leaf names nothing: like a decorative graphics leaf
        // it emits no node until its content becomes non-empty
        // (water-rs/hydrolysis#176).
        let default_label = (!text.as_str().is_empty()).then(|| text.as_str().to_owned());
        let label = renderer.resolve_accessibility_label(env, default_label);
        let value = renderer.resolve_accessibility_value(env, None);
        if (label.is_some() || value.is_some())
            && !label
                .as_deref()
                .is_some_and(|label| renderer.consume_accessibility_descendant_text(env, label))
        {
            let mut node =
                AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                    env,
                    AccessibilityNodeRole::Label,
                ));
            if let Some(label) = label {
                node.set_label(label);
            }
            if let Some(value) = value {
                node.set_value(value);
            }
            let _ = renderer.register_accessibility_leaf(ctx, node, env, None);
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
pub fn measure_str_node(
    text: &Str,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    _theme: &Rc<dyn crate::engine::WidgetTheme>,
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
pub fn render_str_node(
    ctx: &mut WidgetRenderContext<'_>,
    text: &Rc<RefCell<Str>>,
    env: &Environment,
) {
    let render_ctx = ctx.render_context();
    str_accessibility(ctx.renderer_mut(), Some(render_ctx), &text.borrow(), env);
    render_str_parts(ctx, text, env);
}

pub fn render_str_parts(
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

/// Emits a retained `Str` leaf's accessibility node for the semantic walk —
/// the same node `str_accessibility` registers, with no bounds.
#[cfg(feature = "accessibility")]
pub fn emit_str_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    text: &Rc<RefCell<Str>>,
    env: &Environment,
) {
    str_accessibility(renderer, None, &text.borrow(), env);
}

/// Emits a static graphics leaf's `Image` accessibility node for the semantic
/// walk — the same node `graphics_image_accessibility` registers, with no
/// bounds.
#[cfg(feature = "accessibility")]
pub fn emit_graphics_leaf_accessibility<T>(
    renderer: &mut crate::renderer::SemanticCore,
    _state: &Rc<RefCell<T>>,
    env: &Environment,
) {
    graphics_image_accessibility(renderer, None, env, None, None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_graphics::draw::WorkingColor;

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "the scale and radii use exact-representable test values"
    )]
    fn radial_gradient_radii_use_the_shorter_side_of_a_200x100_box() {
        let gradient = Gradient::radial(
            vec![(0.0, WorkingColor::WHITE), (1.0, WorkingColor::BLACK)],
            [0.5, 0.5],
            0.25,
            0.5,
        );
        let paint = gradient_paint_in_bounds(gradient.paint().clone(), 200.0, 100.0);
        let Paint::Radial(radial) = paint else {
            panic!("a radial gradient remains a radial paint");
        };

        assert_eq!(radial.start_center, kurbo::Point::new(100.0, 50.0));
        assert_eq!(radial.end_center, kurbo::Point::new(100.0, 50.0));
        assert_eq!(radial.start_radius, 25.0);
        assert_eq!(radial.end_radius, 50.0);
    }
}
