use crate::renderer::live_progress::LiveProgressParts;
use crate::renderer::{
    HydroNativeView, HydroState, RenderContext, RetainedSubview, WidgetRenderContext,
    measure_progress_intrinsic,
};
use crate::theme::{
    IndicatorLook, LinearProgressLook, ProgressControl, ProgressSpec, SpecTint, WidgetTheme,
};
#[cfg(feature = "accessibility")]
use accesskit::{Node as AccessibilityNode, Role as AccessibilityNodeRole};
use kurbo::{Rect, Size, Vec2};
use nami::{Binding, Computed, Signal};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;
use waterui::component::progress::{ProgressConfig, ProgressStyle};
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::contract_violation;
use waterui_core::layout::Size as LayoutSize;
use waterui_core::layout::{ProposalSize, ViewDimensions};

/// A progress indicator's look: the theme's linear bar, or the indicator it
/// draws in a square.
#[derive(Clone)]
enum ProgressLook {
    Linear(LinearProgressLook),
    Indicator(IndicatorLook),
}

impl ProgressLook {
    /// The look `theme` gives an indicator of `spec` in `env`.
    fn resolve(theme: &Rc<dyn WidgetTheme>, spec: &ProgressSpec, env: &Environment) -> Self {
        match spec.style() {
            ProgressStyle::Linear => Self::Linear(theme.linear_progress(spec, env)),
            ProgressStyle::Circular => Self::Indicator(theme.circular_progress(spec, env)),
            ProgressStyle::Loading => Self::Indicator(theme.loading_indicator(spec, env)),
            style => {
                contract_violation!(
                    "hydrolysis ProgressStyle variant {style:?} is not implemented; the indicator is drawn as a linear bar"
                );
                Self::Linear(theme.linear_progress(spec, env))
            }
        }
    }

    /// The heights of the label band and the value-label band an indicator
    /// of `style` reserves, when it is `determinate` and its labels measure
    /// `labels`: a label with no extent is no label, only a linear indicator
    /// draws labels, and only a determinate one draws a value label.
    /// Measurement and rendering both place the bands from this.
    fn bands(
        style: ProgressStyle,
        determinate: bool,
        labels: (LayoutSize, LayoutSize),
    ) -> (Option<f64>, Option<f64>) {
        if !matches!(style, ProgressStyle::Linear) {
            return (None, None);
        }
        (extent(labels.0), extent(labels.1).filter(|_| determinate))
    }

    /// The bands `spec` reserves, from the heights of the labels it has.
    fn spec_bands(
        spec: &ProgressSpec,
        labels: (LayoutSize, LayoutSize),
    ) -> (Option<f64>, Option<f64>) {
        (
            spec.label().then(|| f64::from(labels.0.height)),
            spec.value_label().then(|| f64::from(labels.1.height)),
        )
    }

    /// The size an indicator of this look measures with the given bands.
    fn size(&self, (label, value_label): (Option<f64>, Option<f64>)) -> LayoutSize {
        let size = match self {
            Self::Linear(look) => {
                let width = look.minimum_width();
                let layout = look.layout(width, label, value_label);
                Size::new(width, layout.height)
            }
            Self::Indicator(look) => look.control().size(),
        };
        LayoutSize::new(
            crate::num_cast::f64_as_f32(size.width),
            crate::num_cast::f64_as_f32(size.height),
        )
    }
}

/// The height of a label of `size`, or `None` when it has no extent: the
/// indicator has no such label.
fn extent(size: LayoutSize) -> Option<f64> {
    (size.width > 0.0 || size.height > 0.0).then_some(f64::from(size.height))
}

/// The spec of the indicator `style` is in `env`: `determinate` when its
/// value is a number, with a label and a value label when they have an
/// extent.
fn progress_spec(
    style: ProgressStyle,
    four_color: bool,
    determinate: bool,
    labels: (LayoutSize, LayoutSize),
    env: &Environment,
) -> ProgressSpec {
    // A loading indicator is indeterminate by construction.
    let determinate = determinate && !matches!(style, ProgressStyle::Loading);
    let (label, value_label) = ProgressLook::bands(style, determinate, labels);
    ProgressSpec::new(
        style,
        determinate,
        waterui_core::layout::layout_direction(env).snapshot(),
        SpecTint::resolve(env),
    )
    .with_label(label.is_some())
    .with_value_label(value_label.is_some())
    .with_four_color(four_color)
}

/// The retained render state of a progress indicator. Its `label`/`value_label`
/// are move-only `AnyView`s (they cannot be re-dispatched twice), so the
/// persistent `Widget` node holds them as [`RetainedSubview`]s built once and
/// re-flushed each frame. The look the theme gave, and the parts recorded
/// from it, are held across renders while the indicator's spec and frame stay
/// the same; the `value` moves the parts' operands and re-renders nothing.
pub struct ProgressRenderState<E: crate::text::TextEngine> {
    label: RetainedSubview<E>,
    value_label: RetainedSubview<E>,
    value: Computed<f64>,
    style: ProgressStyle,
    four_color: bool,
    /// Whether the value is a number. It changes only when the value
    /// turns indeterminate or back, so reading it re-renders the indicator
    /// exactly then.
    determinate: Binding<bool>,
    /// Keeps `determinate` following `value`.
    _determinacy: Box<dyn Any>,
    look: Option<(ProgressSpec, ProgressLook)>,
    parts: Option<LiveProgressParts>,
}

impl<E: crate::text::TextEngine> ProgressRenderState<E> {
    pub(crate) fn from_config(config: ProgressConfig) -> Self {
        let (label, value_label, value, style, four_color) = config.into_parts();
        let determinate = Binding::container(value.snapshot().is_finite());
        let determinacy = value.watch({
            let determinate = determinate.clone();
            move |context| {
                let finite = context.value().is_finite();
                if determinate.snapshot() != finite {
                    determinate.set(finite);
                }
            }
        });
        Self {
            label: RetainedSubview::new(label),
            value_label: RetainedSubview::new(value_label),
            value,
            style,
            four_color,
            determinate,
            _determinacy: Box::new(determinacy),
            look: None,
            parts: None,
        }
    }

    /// Eagerly build the label sub-views (the measure path has no renderer).
    pub(crate) fn prebuild_labels(
        &mut self,
        renderer: &mut crate::renderer::SemanticCore<E>,
        env: &Environment,
    ) {
        self.label.ensure_built(renderer, env);
        self.value_label.ensure_built(renderer, env);
    }

    /// Consume the label sub-views' layout-dirty marks: their measured sizes
    /// feed the indicator's, so an enclosing retained sub-view must see them
    /// (water-rs/waterui#1957).
    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        self.label.take_layout_dirty_if_built() | self.value_label.take_layout_dirty_if_built()
    }

    /// The look for `spec` in `env`: the one already held when the spec is
    /// unchanged, otherwise requested again.
    fn look(
        &mut self,
        theme: &Rc<dyn WidgetTheme>,
        spec: ProgressSpec,
        env: &Environment,
    ) -> &ProgressLook {
        if self.held(&spec).is_none() {
            let look = ProgressLook::resolve(theme, &spec, env);
            self.look = Some((spec, look));
        }
        &self.look.as_ref().expect("the look was just ensured").1
    }

    fn held(&self, spec: &ProgressSpec) -> Option<&ProgressLook> {
        self.look
            .as_ref()
            .filter(|(held, _)| held == spec)
            .map(|(_, look)| look)
    }

    /// The look for `spec` in `env` without holding it: the held one, or a
    /// fresh one for a measurement ahead of the render that holds it.
    fn peek(
        &self,
        theme: &Rc<dyn WidgetTheme>,
        spec: &ProgressSpec,
        env: &Environment,
    ) -> ProgressLook {
        self.held(spec)
            .cloned()
            .unwrap_or_else(|| ProgressLook::resolve(theme, spec, env))
    }
}

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<ProgressConfig> {
    fn intrinsic(
        state: &mut HydroState<E>,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        measure_progress_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// The measured size of the indicator `config` describes, from its labels'
/// measured sizes.
pub fn progress_size(
    theme: &Rc<dyn WidgetTheme>,
    style: ProgressStyle,
    four_color: bool,
    determinate: bool,
    labels: (LayoutSize, LayoutSize),
    env: &Environment,
) -> LayoutSize {
    let spec = progress_spec(style, four_color, determinate, labels, env);
    ProgressLook::resolve(theme, &spec, env).size(ProgressLook::spec_bands(&spec, labels))
}

/// Emits a progress indicator's accessibility node. Shared by the dispatch path
/// (passing the config label's extracted default text) and the retained node path
/// (passing the [`RetainedSubview`]'s build-time default text).
pub fn progress_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    ctx: Option<RenderContext>,
    default_label: Option<String>,
    value: &Computed<f64>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let mut node = AccessibilityNode::new(
            crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::ProgressIndicator,
            ),
        );
        let resolved = renderer.resolve_accessibility_label(env, default_label);
        if let Some(resolved) = resolved {
            node.set_label(resolved);
        }
        if let Some(value) = renderer.resolve_accessibility_value(env, None) {
            node.set_value(value);
        }
        node.set_min_numeric_value(0.0);
        node.set_max_numeric_value(1.0);
        let current = renderer.read_signal(value);
        if current.is_finite() {
            node.set_numeric_value(current.clamp(0.0, 1.0));
        }
        let _ = renderer.register_accessibility_leaf(ctx, node, env, None);
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, default_label, value, env);
    }
}

/// Measures a retained progress leaf from its [`ProgressRenderState`], reading
/// the label sizes from its already-built [`RetainedSubview`]s so layout and
/// render agree on which bands the indicator reserves.
pub fn measure_progress_node<E: crate::text::TextEngine>(
    render_state: &ProgressRenderState<E>,
    _proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> ViewDimensions {
    let determinate = state.measure_signal(&render_state.determinate);
    let labels = (
        render_state.label.measure_built(state, env, theme),
        render_state.value_label.measure_built(state, env, theme),
    );
    let spec = progress_spec(
        render_state.style,
        render_state.four_color,
        determinate,
        labels,
        env,
    );
    ViewDimensions::new(
        render_state
            .peek(theme, &spec, env)
            .size(ProgressLook::spec_bands(&spec, labels)),
    )
}

/// Renders a retained progress leaf every flush: emits a11y (unless hidden) then
/// the indicator + labels. The value moves the indicator's operands directly;
/// the node renders again only for what the accessibility node and the
/// look's spec read.
pub fn render_progress_node<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ProgressRenderState<E>>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        let (default_label, value) = {
            let progress = state.borrow();
            (progress.label.default_a11y_label(), progress.value.clone())
        };
        progress_accessibility(
            ctx.renderer_mut(),
            Some(render_ctx),
            default_label,
            &value,
            env,
        );
    }
    render_progress_parts(ctx, state, env);
}

pub fn render_progress_parts<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ProgressRenderState<E>>>,
    env: &Environment,
) {
    let theme = ctx.theme();
    let mut progress = state.borrow_mut();
    let determinate = {
        let signal = progress.determinate.clone();
        ctx.renderer_mut().read_signal(&signal)
    };
    let labels = (
        progress.label.measure_intrinsic(ctx.renderer_mut(), env),
        progress
            .value_label
            .measure_intrinsic(ctx.renderer_mut(), env),
    );
    let spec = progress_spec(
        progress.style,
        progress.four_color,
        determinate,
        labels,
        env,
    );
    let (label_height, value_label_height) = ProgressLook::spec_bands(&spec, labels);
    let look = progress.look(&theme, spec, env).clone();
    let origin = Vec2::new(ctx.bounds.x0, ctx.bounds.y0);
    let (control, frame): (&ProgressControl, Rect) = match &look {
        ProgressLook::Linear(linear) => {
            let layout = linear.layout(ctx.bounds.width(), label_height, value_label_height);
            // The labels' semantics are merged into the indicator's own
            // node by `progress_accessibility`, so the sub-views flush
            // visual-only.
            if let Some(band) = layout.label {
                place_label(ctx, &mut progress.label, band + origin, env);
            }
            if let Some(band) = layout.value_label {
                place_label(ctx, &mut progress.value_label, band + origin, env);
            }
            (linear.control(), layout.bar + origin)
        }
        ProgressLook::Indicator(indicator) => (indicator.control(), indicator.frame(ctx.bounds)),
    };
    // The parts read the value through its signal: a value change moves
    // their operands and records nothing.
    let value = progress.value.clone();
    let parts = LiveProgressParts::ensure(&mut progress.parts, control, frame, &value).clone();
    let render_ctx = ctx.render_context();
    parts.push(ctx.renderer_mut(), render_ctx);
}

/// Places a retained label sub-view in `band`, visual-only.
fn place_label<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    label: &mut RetainedSubview<E>,
    band: Rect,
    env: &Environment,
) {
    let render_ctx = ctx.render_context();
    let area = ctx.safe_area_for(band);
    ctx.renderer_mut()
        .with_suppressed_accessibility(|renderer| {
            label.place(
                renderer,
                render_ctx,
                env,
                ProposalSize::UNSPECIFIED,
                band,
                area,
            );
        });
}

/// Emits a retained progress indicator's accessibility node for the semantic
/// walk — the same node `progress_accessibility` registers, with no bounds.
/// The label and value-label sub-views flush visual-only, so there is nothing
/// else to emit.
#[cfg(feature = "accessibility")]
pub fn emit_progress_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    state: &Rc<RefCell<ProgressRenderState<E>>>,
    env: &Environment,
) {
    let (default_label, value) = {
        let progress = state.borrow();
        (progress.label.default_a11y_label(), progress.value.clone())
    };
    progress_accessibility(renderer, None, default_label, &value, env);
}
