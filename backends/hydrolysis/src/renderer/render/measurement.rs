// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::num_cast::f64_as_f32;
use crate::text::TextEngine;
use crate::text::TextLayout as _;
use crate::theme::{ListMetrics, WidgetTheme};
use crate::widgets::layout::container::resolve_lazy_container_extent;
use crate::widgets::nav::tabs::{tabs_decide_layout, tabs_item_natural_width};
use std::rc::Rc;
use waterui::navigation::tab::TabIcon;
use waterui_core::contract_violation;
use waterui_core::handler::BoxedAction;
use waterui_core::layout::{LayoutError, ResolvedExtent, layout_direction, resolve_layout_extent};
use waterui_core::views::ViewSnapshot;
use waterui_form::picker::PickerStyle;
use waterui_form::picker::date::DatePickerConfig;
use waterui_layout::container::resolve_directional_extent;
use waterui_layout::padding::EdgeInsets;

pub struct MeasuredTableMetrics {
    pub(crate) column_widths: Vec<f64>,
    pub(crate) table_width: f64,
    pub table_height: f64,
}

pub fn table_header_cell_rect(
    origin_x: f64,
    origin_y: f64,
    x_offset: f64,
    width: f64,
    metrics: crate::theme::TableMetrics,
) -> kurbo::Rect {
    kurbo::Rect::new(
        origin_x + x_offset,
        origin_y,
        origin_x + x_offset + width,
        origin_y + metrics.header_height,
    )
}

pub fn table_data_cell_rect(
    origin_x: f64,
    origin_y: f64,
    x_offset: f64,
    width: f64,
    row_index: usize,
    metrics: crate::theme::TableMetrics,
) -> kurbo::Rect {
    let y0 = metrics.row_height.mul_add(
        crate::num_cast::usize_as_f64(row_index),
        origin_y + metrics.header_height,
    );
    kurbo::Rect::new(
        origin_x + x_offset,
        y0,
        origin_x + x_offset + width,
        y0 + metrics.row_height,
    )
}

fn navigation_bar_height<E: crate::text::TextEngine>(
    view: &NavigationView,
    state: &mut HydroState<E>,
    theme: &Rc<dyn WidgetTheme>,
) -> f64 {
    if state.measure_signal(&view.bar.hidden) {
        0.0
    } else {
        let metrics = theme.navigation_metrics();
        let base =
            navigation_base_bar_height_for_display_mode_metrics(view.bar.display_mode, metrics);
        let search_extra = if view.bar.search.is_some() {
            metrics
                .search_vertical_inset
                .mul_add(2.0, metrics.search_height)
        } else {
            0.0
        };
        let bottom_extra = if view.bar.toolbar.items().iter().any(|item| {
            matches!(
                item.placement(),
                NavigationToolbarPlacement::BottomBar | NavigationToolbarPlacement::Status
            )
        }) {
            metrics.inline_bar_height
        } else {
            0.0
        };
        base + search_extra + bottom_extra
    }
}

pub fn navigation_base_bar_height_for_display_mode(
    display_mode: waterui::navigation::NavigationTitleDisplayMode,
    theme: &Rc<dyn WidgetTheme>,
) -> f64 {
    navigation_base_bar_height_for_display_mode_metrics(display_mode, theme.navigation_metrics())
}

const fn navigation_base_bar_height_for_display_mode_metrics(
    display_mode: waterui::navigation::NavigationTitleDisplayMode,
    metrics: crate::theme::NavigationMetrics,
) -> f64 {
    match display_mode {
        waterui::navigation::NavigationTitleDisplayMode::Automatic => metrics.automatic_bar_height,
        waterui::navigation::NavigationTitleDisplayMode::Inline => metrics.inline_bar_height,
        waterui::navigation::NavigationTitleDisplayMode::Medium => metrics.medium_bar_height,
        waterui::navigation::NavigationTitleDisplayMode::Large => metrics.large_bar_height,
    }
}

pub fn split_compact_threshold(sidebar_width: f64) -> f64 {
    sidebar_width + 360.0
}

pub fn measure_view_intrinsic<E: crate::text::TextEngine>(
    view: &AnyView,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    measure_view_dimensions(view, state, env, theme).size
}

/// Measures a view this measurement materialized rather than one the retained
/// tree owns.
///
/// The intrinsic-measurement cache is keyed by heap address, which names a view
/// only while that view is allocated, so a view built in order to be measured —
/// and dropped the moment it has been — must not reach that cache: the next one
/// materialized in the same frame is handed its address and reads its size back
/// as its own. Use this at every site that measures a view it just built. See
/// [`begin_transient_measurement`](MeasurementCaches::begin_transient_measurement).
pub fn measure_transient_view_intrinsic<E: crate::text::TextEngine>(
    view: &AnyView,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    measure_transient_view_with_proposal(view, ProposalSize::UNSPECIFIED, state, env, theme)
}

/// Measures a view this measurement materialized rather than one the retained
/// tree owns, under `proposal`. Same contract as
/// [`measure_transient_view_intrinsic`]; the proposal is the rect the layout
/// hands the content, so bounded axes answer the proposal.
pub fn measure_transient_view_with_proposal<E: crate::text::TextEngine>(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    state.measurement.begin_transient_measurement();
    let size = measure_view_dimensions_with_proposal(view, proposal, state, env, theme).size;
    state.measurement.end_transient_measurement();
    size
}

/// Measures the intrinsic visual size of a control's [`Label`].
///
/// The label is type-erased at this single boundary rather than at every call
/// site. The `AnyView` exists only for the measurement, so this goes through
/// [`measure_transient_view_intrinsic`]. The semantic identity of the label
/// remains typed inside the control's config.
pub fn measure_label_intrinsic<E: crate::text::TextEngine>(
    label: &waterui_controls::label::Label,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    measure_transient_view_intrinsic(&AnyView::new(label.clone()), state, env, theme)
}

pub fn measure_view_dimensions<E: crate::text::TextEngine>(
    view: &AnyView,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> ViewDimensions {
    measure_view_dimensions_with_proposal(view, ProposalSize::UNSPECIFIED, state, env, theme)
}

pub fn measure_view_dimensions_with_proposal<E: crate::text::TextEngine>(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> ViewDimensions {
    let identity = view.stable_ptr() as usize;
    let env_identity = env.identity();
    if let Some(dimensions) = state
        .measurement
        .view_dimensions(identity, env_identity, proposal)
    {
        return dimensions;
    }

    let dimensions =
        measure_view_dimensions_with_proposal_with_budget(view, proposal, state, env, theme, 256);
    state
        .measurement
        .store_view_dimensions(identity, env_identity, proposal, dimensions.clone());
    dimensions
}

#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
fn measure_view_dimensions_with_proposal_with_budget<E: crate::text::TextEngine>(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
    remaining: usize,
) -> ViewDimensions {
    if remaining == 0 {
        contract_violation!(
            "hydrolysis view measurement exceeded recursion budget for {}; the view measures as zero",
            view.name()
        );
        return ViewDimensions::new(LayoutSize::zero());
    }
    let (view, scoped_env) = flatten_environment_metadata_ref(view, env);

    if let Some(content) = passthrough_content(view) {
        return measure_view_dimensions_with_proposal_with_budget(
            content,
            proposal,
            state,
            &scoped_env,
            theme,
            remaining - 1,
        );
    }

    if view.downcast_ref::<()>().is_some() {
        return ViewDimensions::new(LayoutSize::zero());
    }

    if let Some(text) = view.downcast_ref::<Str>() {
        return HydrolysisRenderer::measure_text_dimensions(
            state,
            StyledStr::plain(text.clone()),
            HorizontalAlignment::Leading,
            &scoped_env,
            proposal.width,
            None,
        );
    }
    if let Some(text) = view.downcast_ref::<&'static str>() {
        let body = AnyView::new((*text).body(&scoped_env));
        return measure_view_dimensions_with_proposal_with_budget(
            &body,
            proposal,
            state,
            &scoped_env,
            theme,
            remaining - 1,
        );
    }
    if let Some(text) = view.downcast_ref::<String>() {
        let body = AnyView::new(text.clone().body(&scoped_env));
        return measure_view_dimensions_with_proposal_with_budget(
            &body,
            proposal,
            state,
            &scoped_env,
            theme,
            remaining - 1,
        );
    }
    if let Some(text) = view.downcast_ref::<Cow<'static, str>>() {
        let body = AnyView::new(text.clone().body(&scoped_env));
        return measure_view_dimensions_with_proposal_with_budget(
            &body,
            proposal,
            state,
            &scoped_env,
            theme,
            remaining - 1,
        );
    }
    if let Some(text) = view.downcast_ref::<Text>() {
        let resolved = text.resolve(&scoped_env);
        let content = state.measure_signal(&resolved.content());
        let alignment = state.measure_signal(resolved.paragraph_alignment());
        return HydrolysisRenderer::measure_text_dimensions(
            state,
            content,
            alignment,
            &scoped_env,
            proposal.width,
            resolved.line_limit().map(core::num::NonZeroUsize::get),
        );
    }
    if let Some(label) = view.downcast_ref::<SemanticLabel>() {
        let body_env = scoped_env.clone();
        let body = normalize_layout_view(AnyView::new(label.clone().body(&body_env)), &body_env);
        return measure_view_dimensions_with_proposal_with_budget(
            &body,
            proposal,
            state,
            &body_env,
            theme,
            remaining - 1,
        );
    }
    if let Some(button) = view.downcast_ref::<Button<BoxedAction<()>>>() {
        return ViewDimensions::new(measure_button_view_intrinsic(
            button,
            state,
            &scoped_env,
            theme,
        ));
    }
    // Normalization keeps a `FixedContainer` plain (its `body` runs at
    // build); the estimator measures it like the body-produced Native form —
    // `body` wraps the layout in `DirectionalLayout`, which mirrors the
    // placements under RTL before explicit guides resolve against them.
    if let Some(container) = view.downcast_ref::<FixedContainer>() {
        let (layout, children) = container.as_parts();
        return measure_layout_dimensions_mirrored(
            layout,
            children.iter(),
            proposal,
            state,
            &scoped_env,
            theme,
            waterui_core::layout::layout_direction(&scoped_env)
                .snapshot()
                .is_right_to_left(),
        );
    }

    if let Some(dimensions) =
        dimensions_for_known_native_views(view, proposal, state, &scoped_env, theme)
    {
        return dimensions;
    }

    if view.downcast_ref::<Divider>().is_some() {
        return ViewDimensions::new(LayoutSize::new(1.0, 1.0));
    }

    contract_violation!(
        "hydrolysis dimensions estimation encountered unsupported view type {}; the view measures as zero",
        view.name()
    );
    ViewDimensions::new(LayoutSize::zero())
}

/// The response geometry (water-rs/waterui#1323) of a view the estimator
/// measures, dispatched like
/// [`measure_view_dimensions_with_proposal_with_budget`] so the two cannot
/// drift: environment scopes and passthrough metadata forward to their
/// content with the same bounds and proposal, and a container answers its
/// layout's rule over its children.
///
/// `None` means a leaf: the caller answers [`ResolvedExtent::centred`] with its
/// own measured, stretch-applied answer.
///
/// # Errors
///
/// A [`LayoutError`] when the answer or `bounds` is not valid geometry, or a
/// layout broke the response-extent identity.
pub fn resolve_view_extent<E: TextEngine>(
    view: &AnyView,
    bounds: LayoutRect,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> Result<Option<ResolvedExtent>, LayoutError> {
    let (view, scoped_env) = flatten_environment_metadata_ref(view, env);

    if let Some(content) = passthrough_content(view) {
        return resolve_view_extent(content, bounds, proposal, state, &scoped_env, theme);
    }
    // A plain `FixedContainer` has not run `body`, which wraps its layout for
    // the layout direction; resolve it the way that wrapper does. The Native
    // form already carries the wrapper.
    if let Some(container) = view.downcast_ref::<FixedContainer>() {
        let (layout, children) = container.as_parts();
        let direction = layout_direction(&scoped_env).snapshot();
        let state = RefCell::new(state);
        let subviews: Vec<HydroSubview<'_, E>> = children
            .iter()
            .map(|child| HydroSubview::from_view(child, &state, &scoped_env, theme))
            .collect();
        let refs: Vec<&dyn SubView> = subviews.iter().map(|view| view as &dyn SubView).collect();
        return resolve_directional_extent(layout, direction, bounds, proposal, &refs).map(Some);
    }
    if let Some(container) = view.downcast_ref::<Native<FixedContainer>>() {
        let (layout, children) = container.as_inner().as_parts();
        return resolve_layout_view_extent(
            layout,
            children.iter(),
            bounds,
            proposal,
            state,
            &scoped_env,
            theme,
        )
        .map(Some);
    }
    if let Some(lazy) = view.downcast_ref::<Native<LazyContainer>>() {
        return resolve_lazy_container_extent(state, lazy, &scoped_env, theme, bounds, proposal)
            .map(Some);
    }
    Ok(None)
}

/// [`resolve_layout_extent`] over `children` seen through [`HydroSubview`]s.
///
/// # Errors
///
/// As [`resolve_layout_extent`].
pub fn resolve_layout_view_extent<'a, E: TextEngine>(
    layout: &dyn Layout,
    children: impl IntoIterator<Item = &'a AnyView>,
    bounds: LayoutRect,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> Result<ResolvedExtent, LayoutError> {
    let state = RefCell::new(state);
    let subviews: Vec<HydroSubview<'_, E>> = children
        .into_iter()
        .map(|child| HydroSubview::from_view(child, &state, env, theme))
        .collect();
    let refs: Vec<&dyn SubView> = subviews.iter().map(|view| view as &dyn SubView).collect();
    resolve_layout_extent(layout, bounds, proposal, &refs)
}

pub fn measure_layout_dimensions<'a, E: crate::text::TextEngine>(
    layout: &dyn Layout,
    children: impl IntoIterator<Item = &'a AnyView>,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> ViewDimensions {
    measure_layout_dimensions_mirrored(layout, children, proposal, state, env, theme, false)
}

/// `measure_layout_dimensions` with the `DirectionalLayout` mirror applied
/// to the placements: a layout that never went through `body` (the plain
/// `FixedContainer` normalization keeps) must still resolve its explicit
/// horizontal guides against the frames the built tree sees, which
/// `DirectionalLayout` mirrors under RTL.
fn measure_layout_dimensions_mirrored<'a, E: crate::text::TextEngine>(
    layout: &dyn Layout,
    children: impl IntoIterator<Item = &'a AnyView>,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
    mirror_placements: bool,
) -> ViewDimensions {
    let state = RefCell::new(state);
    let children: Vec<&AnyView> = children.into_iter().collect();
    let mut subviews = Vec::new();
    for child in children {
        subviews.push(HydroSubview::from_view(child, &state, env, theme));
    }
    let refs: Vec<&dyn SubView> = subviews.iter().map(|view| view as &dyn SubView).collect();
    let size = layout.size_that_fits(proposal, &refs);
    if size.width.is_infinite()
        || size.height.is_infinite()
        || can_skip_layout_alignment_measurement(layout, &subviews)
    {
        return ViewDimensions::new(size);
    }

    let bounds = LayoutRect::from_size(size);
    let placements = layout.place(bounds, proposal, &refs);
    let placements: Vec<SubviewPlacement> = if mirror_placements {
        placements
            .into_iter()
            .map(|placement| {
                SubviewPlacement::new(
                    LayoutRect::new(
                        LayoutPoint::new(
                            bounds.min_x() + bounds.max_x() - placement.frame.max_x(),
                            placement.frame.y(),
                        ),
                        *placement.frame.size(),
                    ),
                    placement.proposal,
                )
            })
            .collect()
    } else {
        placements
    };
    let placed_subviews: Vec<PlacedSubview<'_>> = subviews
        .iter()
        .zip(placements)
        .map(|(view, placement)| PlacedSubview::new(view as &dyn SubView, placement))
        .collect();

    let mut dimensions = ViewDimensions::new(size);
    let mut horizontal_keys = Vec::new();
    let mut vertical_keys = Vec::new();

    for alignment in layout.explicit_horizontal_alignments() {
        if !horizontal_keys.contains(&alignment) {
            horizontal_keys.push(alignment);
        }
    }
    for alignment in layout.explicit_vertical_alignments() {
        if !vertical_keys.contains(&alignment) {
            vertical_keys.push(alignment);
        }
    }

    for child in &placed_subviews {
        let child_dimensions = child.dimensions();
        for (alignment, _) in child_dimensions.explicit_horizontal_guides() {
            if !horizontal_keys.contains(&alignment) {
                horizontal_keys.push(alignment);
            }
        }
        for (alignment, _) in child_dimensions.explicit_vertical_guides() {
            if !vertical_keys.contains(&alignment) {
                vertical_keys.push(alignment);
            }
        }
    }

    for alignment in horizontal_keys {
        if let Some(value) = layout.explicit_horizontal(alignment, bounds, &placed_subviews) {
            dimensions.set_horizontal(alignment, value);
        }
    }
    for alignment in vertical_keys {
        if let Some(value) = layout.explicit_vertical(alignment, bounds, &placed_subviews) {
            dimensions.set_vertical(alignment, value);
        }
    }

    dimensions
}

fn can_skip_layout_alignment_measurement<E: crate::text::TextEngine>(
    layout: &dyn Layout,
    children: &[HydroSubview<'_, E>],
) -> bool {
    layout.explicit_horizontal_alignments().is_empty()
        && layout.explicit_vertical_alignments().is_empty()
        && children
            .iter()
            .all(|child| view_has_plain_alignment_dimensions(child.view()))
}

fn view_has_plain_alignment_dimensions(view: &AnyView) -> bool {
    if let Some(content) = passthrough_content(view) {
        return view_has_plain_alignment_dimensions(content);
    }
    // Normalization keeps a `FixedContainer` plain (its `body` runs at
    // build), so both the modifier-time and the body-produced (Native) forms
    // reach here.
    if let Some(container) = view.downcast_ref::<FixedContainer>().or_else(|| {
        view.downcast_ref::<Native<FixedContainer>>()
            .map(Native::as_inner)
    }) {
        let (layout, children) = container.as_parts();
        return layout.explicit_horizontal_alignments().is_empty()
            && layout.explicit_vertical_alignments().is_empty()
            && children.iter().all(view_has_plain_alignment_dimensions);
    }
    is_hydro_native_view(view)
        || view.downcast_ref::<()>().is_some()
        || view.downcast_ref::<Str>().is_some()
        || view.downcast_ref::<&'static str>().is_some()
        || view.downcast_ref::<String>().is_some()
        || view.downcast_ref::<Cow<'static, str>>().is_some()
        || view.downcast_ref::<Text>().is_some()
        || view.downcast_ref::<Divider>().is_some()
}

impl<E: crate::text::TextEngine> HydrolysisRenderer<E> {
    pub(crate) fn render_styled_text(
        &mut self,
        ctx: RenderContext,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
    ) {
        self.render_styled_text_limited(
            ctx,
            ctx.bounds_width_f32(),
            styled,
            alignment,
            env,
            TailMark::None,
            None,
        );
    }

    /// Records `styled` broken at `max_width` with its `tail` treatment, its
    /// default-coloured runs drawn in `paint`, live, when given.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    #[expect(
        clippy::too_many_arguments,
        reason = "the text, its placement, its environment and its paint are independent inputs"
    )]
    pub(crate) fn render_styled_text_limited(
        &mut self,
        ctx: RenderContext,
        max_width: f32,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        tail: TailMark,
        paint: Option<&nami::Computed<cherenkov_record::Paint>>,
    ) {
        let input = resolve_text_layout_input(&styled, alignment, env);
        self.record_text_input(ctx, max_width, &input, tail, paint);
    }

    /// [`Self::render_styled_text_limited`], also returning the layout the
    /// recording was made from — the handle `record` shaped under its key,
    /// re-fetched from the shared cache rather than shaped twice. The text
    /// leaf uses it for per-span geometry (tap targets and `Link`
    /// accessibility children); paint-only callers skip the lookup.
    #[expect(
        clippy::too_many_arguments,
        reason = "the text, its placement, its environment and its paint are independent inputs"
    )]
    pub(crate) fn render_styled_text_with_layout(
        &mut self,
        ctx: RenderContext,
        max_width: f32,
        styled: &StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        tail: TailMark,
        paint: Option<&nami::Computed<cherenkov_record::Paint>>,
    ) -> E::Layout {
        let input = resolve_text_layout_input(styled, alignment, env);
        self.record_text_input(ctx, max_width, &input, tail, paint);
        let (max_lines, tail_ellipsis) = tail.parts();
        if tail_ellipsis {
            self.state
                .text
                .shape_limited(&input, Some(max_width), max_lines)
        } else {
            self.state.text.shape(&input, Some(max_width))
        }
    }

    /// Records `input` broken at `max_width` and positioned at `ctx`'s
    /// origin, its default-coloured runs in `paint` when given, and pushes
    /// the recording as a live source.
    fn record_text_input(
        &mut self,
        ctx: RenderContext,
        max_width: f32,
        input: &crate::text::ResolvedTextLayoutInput,
        tail: TailMark,
        paint: Option<&nami::Computed<cherenkov_record::Paint>>,
    ) {
        let state = &mut self.state;
        let max_width = Some(max_width);
        let fragment = match paint {
            Some(paint) => {
                state
                    .text
                    .record_painted(input, max_width, tail, paint, &mut state.counters)
            }
            None => state.text.record(
                input,
                max_width,
                tail,
                &mut state.counters,
                &mut state.text_scenes,
            ),
        };
        let source: Rc<dyn crate::renderer::chrome::LiveSource> = fragment;
        self.push_live(
            {
                let mut literal = ctx;
                literal.set_local(
                    ctx.local() * kurbo::Affine::translate((ctx.bounds().x0, ctx.bounds().y0)),
                );
                literal
            },
            &source,
        );
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    pub(crate) fn render_styled_text_single_line_centered(
        &mut self,
        ctx: RenderContext,
        styled: StyledStr,
        env: &Environment,
        scale: Option<(&Rc<crate::renderer::chrome::LiveChrome>, kurbo::Affine)>,
        paint: Option<&nami::Computed<cherenkov_record::Paint>>,
    ) {
        let state = &mut self.state;
        let input = resolve_text_layout_input(&styled, HorizontalAlignment::Leading, env);
        let layout = state.measuring_text().shape(&input, None);
        if layout.line_count() == 0 {
            return;
        }
        let metrics = layout.line_metrics(0);
        // Center the measured frame — advance widened to cover overhanging
        // ink — matching what `dimensions` reports for it.
        let width = state.text.ink_extent(&layout, Some(1)).map_or_else(
            || f64::from(metrics.advance),
            |(ink_min, ink_max)| f64::from(metrics.advance.max(ink_max) - ink_min.min(0.0)),
        );
        let height = f64::from(metrics.line_height);
        let x = ((ctx.bounds().width() - width) * 0.5).max(0.0);
        let y = ((ctx.bounds().height() - height) * 0.5).max(0.0);
        let fragment = match paint {
            Some(paint) => state.text.record_painted(
                &input,
                None,
                TailMark::Clip(1),
                paint,
                &mut state.counters,
            ),
            None => state.text.record(
                &input,
                None,
                TailMark::Clip(1),
                &mut state.counters,
                &mut state.text_scenes,
            ),
        };
        let mut literal = ctx;
        literal.set_local(ctx.local() * kurbo::Affine::translate((x, y)));
        let source: Rc<dyn crate::renderer::chrome::LiveSource> = match scale {
            Some((chrome, widget)) => Rc::new(crate::renderer::chrome::ScaledContent::new(
                fragment,
                Rc::clone(chrome),
                widget.inverse() * literal.local(),
            )),
            None => fragment,
        };
        self.push_live(literal, &source);
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    pub(crate) fn build_text_layout(
        state: &mut HydroState<E>,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        max_width: Option<f32>,
    ) -> E::Layout {
        let input = resolve_text_layout_input(&styled, alignment, env);
        state.measuring_text().shape(&input, max_width)
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    pub(crate) fn measure_text_dimensions(
        state: &mut HydroState<E>,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        max_width: Option<f32>,
        max_lines: Option<usize>,
    ) -> ViewDimensions {
        let input = resolve_text_layout_input(&styled, alignment, env);
        let text = state.measuring_text();
        let layout = text.shape_limited(&input, max_width, max_lines);
        text.dimensions(&layout, max_lines)
    }

    pub(crate) fn measure_text_intrinsic_size(
        state: &mut HydroState<E>,
        styled: StyledStr,
        env: &Environment,
    ) -> LayoutSize {
        Self::measure_text_dimensions(state, styled, HorizontalAlignment::Leading, env, None, None)
            .size
    }

    pub(crate) fn measure_text_intrinsic_size_with_line_limit(
        state: &mut HydroState<E>,
        styled: StyledStr,
        env: &Environment,
        max_lines: Option<usize>,
    ) -> LayoutSize {
        Self::measure_text_dimensions(
            state,
            styled,
            HorizontalAlignment::Leading,
            env,
            None,
            max_lines,
        )
        .size
    }

    /// The size of an editable field's content, at most `max_lines` lines.
    /// Empty text still measures the one line a caret occupies, so a field
    /// keeps its height as text is entered.
    pub(crate) fn measure_editable_text_size(
        state: &HydroState<E>,
        styled: &StyledStr,
        env: &Environment,
        max_lines: Option<usize>,
    ) -> LayoutSize {
        let input = resolve_text_layout_input(styled, HorizontalAlignment::Leading, env);
        let layout = state.text.shape_editable(&input, None);
        state.text.dimensions(&layout, max_lines).size
    }
}

#[expect(
    clippy::option_if_let_else,
    reason = "the if-let/else mirrors the control flow more clearly than the combinator chain here"
)]
pub fn measure_navigation_view_intrinsic<E: crate::text::TextEngine>(
    navigation: &NavigationView,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let bar_height = navigation_bar_height(navigation, state, theme);
    let mut principal_width = 0.0_f64;
    let mut principal_height = 0.0_f64;
    let mut leading_width = 0.0_f64;
    let mut leading_height = 0.0_f64;
    let mut trailing_width = 0.0_f64;
    let mut trailing_height = 0.0_f64;
    let mut bottom_width = 0.0_f64;
    let mut bottom_height = 0.0_f64;
    let metrics = theme.navigation_metrics();
    for item in navigation.bar.toolbar.items() {
        let (content, scoped) = flatten_environment_metadata_ref(item.content(), env);
        let size = measure_view_intrinsic(
            content,
            state,
            &crate::widgets::nav::navigation::toolbar_item_env(&scoped),
            theme,
        );
        let (width, height) = match item.placement() {
            NavigationToolbarPlacement::Principal => (&mut principal_width, &mut principal_height),
            NavigationToolbarPlacement::Cancellation
            | NavigationToolbarPlacement::TopBarLeading => {
                (&mut leading_width, &mut leading_height)
            }
            NavigationToolbarPlacement::BottomBar | NavigationToolbarPlacement::Status => {
                (&mut bottom_width, &mut bottom_height)
            }
            NavigationToolbarPlacement::PrimaryAction
            | NavigationToolbarPlacement::SecondaryAction
            | NavigationToolbarPlacement::Confirmation
            | NavigationToolbarPlacement::TopBarTrailing => {
                (&mut trailing_width, &mut trailing_height)
            }
        };
        if *width > 0.0 {
            *width += metrics.item_spacing;
        }
        *width += f64::from(size.width);
        *height = (*height).max(f64::from(size.height));
    }
    let title_size = if bar_height > 0.0 && principal_width == 0.0 {
        let title = measure_view_intrinsic(&navigation.bar.title, state, env, theme);
        let subtitle = if navigation.bar.subtitle.is::<()>() {
            LayoutSize::zero()
        } else {
            measure_view_intrinsic(&navigation.bar.subtitle, state, env, theme)
        };
        LayoutSize::new(
            title.width.max(subtitle.width),
            title.height + subtitle.height,
        )
    } else if bar_height > 0.0 {
        LayoutSize::new(
            crate::num_cast::f64_as_f32(principal_width),
            crate::num_cast::f64_as_f32(principal_height),
        )
    } else {
        LayoutSize::zero()
    };
    let leading_size = LayoutSize::new(
        crate::num_cast::f64_as_f32(leading_width),
        crate::num_cast::f64_as_f32(leading_height),
    );
    let trailing_size = LayoutSize::new(
        crate::num_cast::f64_as_f32(trailing_width),
        crate::num_cast::f64_as_f32(trailing_height),
    );
    let search_size = if let Some(search) = navigation.bar.search.as_ref() {
        let body_env = env.clone();
        // Mirrors the search field built in `widgets::nav::navigation`.
        let search_field = TextField::new(search.prompt().clone(), search.text())
            .hide_label()
            .prompt(search.prompt().clone());
        let search_body =
            normalize_layout_view(AnyView::new(search_field.body(&body_env)), &body_env);
        measure_transient_view_intrinsic(&search_body, state, &body_env, theme)
    } else {
        LayoutSize::zero()
    };
    let content_size = measure_view_intrinsic(&navigation.content, state, env, theme);
    let width = f64::from(content_size.width)
        .max(metrics.item_spacing.mul_add(
            2.0,
            metrics.horizontal_inset.mul_add(
                2.0,
                f64::from(leading_size.width)
                    + f64::from(title_size.width)
                    + f64::from(trailing_size.width),
            ),
        ))
        .max(
            metrics
                .horizontal_inset
                .mul_add(2.0, f64::from(search_size.width)),
        )
        .max(metrics.horizontal_inset.mul_add(2.0, bottom_width));
    let height = f64::from(content_size.height) + bar_height;
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    )
}

/// Measures an owned `NavigationView` materialized for the probe — e.g. a
/// split's detail column resolved from its selection — under `proposal`, the
/// rect the container hands the column. The whole view dies with the call, so
/// it is normalized and measured as transient: none of its parts may leave an
/// entry under an address the next build is handed.
pub fn measure_owned_navigation_view_with_proposal<E: crate::text::TextEngine>(
    navigation: NavigationView,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let navigation = normalize_layout_view(AnyView::new(navigation), env);
    measure_transient_view_with_proposal(&navigation, proposal, state, env, theme)
}

/// Measures a `TabsLayout` under `proposal`: each tab's content answers the
/// proposal its rendered content rect hands it — the pane minus the tab bar —
/// and the layout echoes bounded axes. `intrinsic` is the `UNSPECIFIED` call.
pub fn measure_tabs_layout<E: crate::text::TextEngine>(
    tabs: &TabsLayout,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    // An empty set mounts: nothing answers but the bar's own metrics.
    let snapshot = tabs.tabs().snapshot();
    let tab_views: Vec<_> = (0..snapshot.len())
        .filter_map(|index| {
            let tab = snapshot.get_view(index);
            if tab.is_none() {
                contract_violation!(
                    "hydrolysis tabs: tab {index} failed to materialize although it is in the snapshot's range; the tab is skipped"
                );
            }
            tab
        })
        .collect();

    // Labels measure title-only: the bar places the icon itself, so the label
    // must not count it a second time.
    let label_env = env.extending(waterui_controls::label::LabelDisplayMode::TitleOnly);
    let item_sizes: Vec<(LayoutSize, Option<LayoutSize>)> = tab_views
        .iter()
        .map(|tab| {
            let label_size = measure_view_intrinsic(&tab.label().build(), state, &label_env, theme);
            let icon_size = tab.icon().as_ref().map(|icon| {
                let icon_view = match icon {
                    TabIcon::System(icon) => AnyView::new(icon.clone()),
                    TabIcon::View(builder) => builder.build(),
                };
                measure_transient_view_with_proposal(
                    &normalize_layout_view(icon_view, env),
                    ProposalSize::UNSPECIFIED,
                    state,
                    env,
                    theme,
                )
            });
            (label_size, icon_size)
        })
        .collect();
    // Decide the layout once from the bar's own extent so the measured bar
    // and the drawn bar answer the same layout (see `tabs_decide_layout`).
    let (layout, metrics) = tabs_decide_layout(
        theme,
        tabs.get_style(),
        proposal.width.map(f64::from),
        &item_sizes,
    );
    let content_proposal = tabs_content_proposal(proposal, tabs.get_style(), metrics.bar_height);
    let mut max_content_width: f64 = 0.0;
    let mut max_content_height: f64 = 0.0;
    let mut bar_width = 0.0;
    for (tab, (label_size, icon_size)) in tab_views.iter().zip(&item_sizes) {
        bar_width += tabs_item_natural_width(*label_size, *icon_size, &metrics, layout);

        let content = normalize_layout_view(AnyView::new(tab.content().build()), env);
        let content_size =
            measure_transient_view_with_proposal(&content, content_proposal, state, env, theme);
        max_content_width = max_content_width.max(f64::from(content_size.width));
        max_content_height = max_content_height.max(f64::from(content_size.height));
    }

    let (width, height) = match tabs.get_style() {
        NativeTabStyle::Automatic | NativeTabStyle::TabBar => (
            max_content_width.max(bar_width),
            max_content_height + metrics.bar_height,
        ),
        NativeTabStyle::Sidebar => (
            max_content_width + metrics.bar_height,
            max_content_height
                .max(metrics.button_min_width * crate::num_cast::usize_as_f64(tab_views.len())),
        ),
    };
    LayoutSize::new(
        proposal
            .width
            .unwrap_or_else(|| crate::num_cast::f64_as_f32(width)),
        proposal
            .height
            .unwrap_or_else(|| crate::num_cast::f64_as_f32(height)),
    )
}

/// The proposal a tab's content measures at: the pane minus the tab bar — a
/// bottom strip for `Automatic`/`TabBar`, a leading strip for `Sidebar`.
/// This is a *measurement* proposal, not the rendered content rect: the §7.1
/// chrome split at render time carves the band out of the laid-out frame,
/// and the rendered content rect keeps the whole frame once the keyboard
/// covers the band outright (keyboard deeper than container inset plus bar
/// height). Bounded axes echo the offer; an axis the container left open
/// stays open.
pub fn tabs_content_proposal(
    proposal: ProposalSize,
    style: NativeTabStyle,
    bar_extent: f64,
) -> ProposalSize {
    match style {
        NativeTabStyle::Automatic | NativeTabStyle::TabBar => ProposalSize::new(
            proposal.width,
            proposal.height.map(|height| {
                crate::num_cast::f64_as_f32((f64::from(height) - bar_extent).max(0.0))
            }),
        ),
        NativeTabStyle::Sidebar => ProposalSize::new(
            proposal
                .width
                .map(|width| crate::num_cast::f64_as_f32((f64::from(width) - bar_extent).max(0.0))),
            proposal.height,
        ),
    }
}

pub fn tabs_button_rect(
    bar_rect: kurbo::Rect,
    tab_count: usize,
    index: usize,
    style: NativeTabStyle,
) -> kurbo::Rect {
    match style {
        NativeTabStyle::Automatic | NativeTabStyle::TabBar => {
            let button_width = bar_rect.width() / crate::num_cast::usize_as_f64(tab_count);
            let x0 = button_width.mul_add(crate::num_cast::usize_as_f64(index), bar_rect.x0);
            kurbo::Rect::new(x0, bar_rect.y0, x0 + button_width, bar_rect.y1)
        }
        NativeTabStyle::Sidebar => {
            let button_height = bar_rect.height() / crate::num_cast::usize_as_f64(tab_count);
            let y0 = button_height.mul_add(crate::num_cast::usize_as_f64(index), bar_rect.y0);
            kurbo::Rect::new(bar_rect.x0, y0, bar_rect.x1, y0 + button_height)
        }
    }
}

pub fn navigation_back_button_rect(
    bounds: kurbo::Rect,
    metrics: crate::theme::NavigationMetrics,
) -> kurbo::Rect {
    kurbo::Rect::new(
        bounds.x0 + metrics.back_button_leading_inset,
        bounds.y0 + metrics.back_button_top_inset,
        bounds.x0 + metrics.back_button_leading_inset + metrics.back_button_size,
        bounds.y0 + metrics.back_button_top_inset + metrics.back_button_size,
    )
}

/// The `(left, right)` insets a list row opens its content by: the row's
/// `insets` replace the theme's row insets edge for edge, and unset the theme's
/// symmetric horizontal inset applies. `EdgeInsets` is logical, so leading is
/// the side the layout direction starts from.
pub fn list_row_horizontal_insets(
    insets: Option<&EdgeInsets>,
    metrics: ListMetrics,
    env: &Environment,
) -> (f64, f64) {
    let (leading, trailing) = insets.map_or(
        (metrics.horizontal_inset, metrics.horizontal_inset),
        |insets| (f64::from(insets.leading()), f64::from(insets.trailing())),
    );
    if layout_direction(env).snapshot().is_right_to_left() {
        (trailing, leading)
    } else {
        (leading, trailing)
    }
}

/// The spec of a list in `env`.
#[must_use]
pub fn list_spec(env: &Environment) -> crate::theme::ListSpec {
    crate::theme::ListSpec::new(layout_direction(env).snapshot())
}

/// Measures a list row's content at the width the row lays it out at: the row
/// width less the row's horizontal insets, with the height open. A row's
/// height comes from this answer, so content that wraps is held at the lines it
/// wraps to there. The content is a view this measurement materialized, so it
/// goes through the transient path.
pub fn measure_list_row_content<E: TextEngine>(
    content: &AnyView,
    row_width: f64,
    insets: Option<&EdgeInsets>,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
    metrics: ListMetrics,
) -> LayoutSize {
    let (left, right) = list_row_horizontal_insets(insets, metrics, env);
    let content_width = (row_width - left - right).max(0.0);
    measure_transient_view_with_proposal(
        content,
        ProposalSize::new(Some(f64_as_f32(content_width)), None),
        state,
        env,
        theme,
    )
}

/// The list's own size. `width` is the width the list is laid out at when one
/// is proposed: rows are then measured at it, so the height answers the
/// wrapping that width causes; `None` is the ideal query, rows at their natural
/// width.
pub fn measure_list_intrinsic<E: crate::text::TextEngine>(
    list: &ListConfig,
    width: Option<f32>,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    // One immutable row set for the measure: the count, the sampled item, and
    // the section-marker walk all read the same membership.
    let contents = list.contents().snapshot();
    let row_count = contents.len();
    if row_count == 0 {
        return LayoutSize::zero();
    }
    let editing = state.measure_signal(list.editing());
    let first_item = materialize_list_item(&contents, 0, env);
    let metrics = theme.list(&list_spec(env), env).metrics();
    let content_size = match width {
        Some(width) => measure_list_row_content(
            first_item.content(),
            f64::from(width),
            first_item.get_insets(),
            state,
            env,
            theme,
            metrics,
        ),
        None => measure_transient_view_intrinsic(first_item.content(), state, env, theme),
    };
    let row_height = list_row_height_for_content(
        f64::from(content_size.height),
        first_item.get_insets(),
        list.min_row_height(),
        metrics,
    );

    let mut row_width = metrics
        .horizontal_inset
        .mul_add(2.0, f64::from(content_size.width));
    if editing && list.on_move().is_some() {
        row_width += metrics.move_control_width + metrics.trailing_control_spacing;
    }
    if editing && list.on_delete().is_some() {
        row_width += metrics.delete_control_width + metrics.trailing_control_spacing;
    }
    // Section chrome is part of the list's own height: each section holding
    // rows draws its header and footer once. The sections come from row ids,
    // so no row is materialized for them.
    let mut section_height = 0.0;
    for span in list.sections().spans(&contents) {
        let Some(section) = span.section else {
            continue;
        };
        if section.label().is_some() {
            section_height += metrics.section_header_height;
        }
        if section.get_footer().is_some() {
            section_height += metrics.section_footer_height;
        }
    }

    let total_height = row_height.mul_add(crate::num_cast::usize_as_f64(row_count), section_height);
    let max_width = row_width.max(metrics.horizontal_inset * 2.0);

    LayoutSize::new(
        crate::num_cast::f64_as_f32(max_width),
        crate::num_cast::f64_as_f32(total_height),
    )
}

/// Materializes the `ListItem` at `index` from `contents` — an immutable
/// snapshot of the row collection, so the caller's whole pass (section walk,
/// accessibility emit, or draw loop) reads one coherent membership.
pub fn materialize_list_item(
    contents: &impl ViewSnapshot<View = ListItem>,
    index: usize,
    env: &Environment,
) -> ListItem {
    let mut item = contents.get_view(index).unwrap_or_else(|| {
        contract_violation!(
            "ListConfig failed to materialize item at index {index}; the row is empty"
        );
        ListItem::new(())
    });
    let content = normalize_layout_view(item.take_content(), env);
    item.set_content(content);
    item
}

/// A list row's extent from its content's measured height: the content plus
/// the row's vertical insets, floored at the configured minimum. `insets` is
/// `ListItem::insets` — `None` uses the theme's `vertical_inset` — and
/// `min_height` is `ListConfig::min_row_height` — `None` uses the theme's
/// `one_line_row_height`, so both unset reproduces the theme metrics exactly.
/// The caller measures the content ([`measure_list_row_content`] on the
/// materialized `ListItem`) and the section chrome height is added on top
/// of what this returns.
pub fn list_row_height_for_content(
    content_height: f64,
    insets: Option<&EdgeInsets>,
    min_height: Option<f32>,
    metrics: crate::theme::ListMetrics,
) -> f64 {
    let vertical_insets = insets.map_or(metrics.vertical_inset * 2.0, |insets| {
        f64::from(insets.top() + insets.bottom())
    });
    let floor = min_height.map_or(metrics.one_line_row_height, f64::from);
    (content_height + vertical_insets).max(floor)
}

pub fn measure_progress_intrinsic<E: crate::text::TextEngine>(
    progress: &ProgressConfig,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let determinate = state.measure_signal(progress.value()).is_finite();
    let labels = (
        measure_view_intrinsic(progress.label(), state, env, theme),
        measure_view_intrinsic(progress.value_label(), state, env, theme),
    );
    crate::widgets::controls::progress::progress_size(
        theme,
        progress.style(),
        progress.four_color(),
        determinate,
        labels,
        env,
    )
}

pub fn measure_text_field_intrinsic<E: crate::text::TextEngine>(
    text_field: &ResolvedTextFieldConfig,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let label_size = measure_label_intrinsic(text_field.label(), state, env, theme);
    measure_text_field_intrinsic_with_label_size(text_field, label_size, state, env, theme)
}

/// Measures a text field's intrinsic size from a precomputed label size. The
/// dispatch path passes the label measured via `measure_label_intrinsic`; the
/// retained-node path passes the label measured from its built `RetainedSubview`,
/// so layout and the floating-label render agree on the label height.
pub fn measure_text_field_intrinsic_with_label_size<E: crate::text::TextEngine>(
    text_field: &ResolvedTextFieldConfig,
    label_size: LayoutSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    measure_text_field_size_with_label_size(
        text_field,
        label_size,
        state,
        env,
        theme,
        ProposalSize::UNSPECIFIED,
    )
}

/// Measures a text field's size under a concrete proposal, from a precomputed
/// label size. The field is a `Horizontal` leaf: it answers the proposal
/// width floored at the label's extent, so the reported width never depends
/// on the field's text — text scrolls inside the dealt width. `None` answers
/// the ideal, the theme's `min_width` floored at the label. Height is the
/// intrinsic height, whose text line an empty field measures too.
pub fn measure_text_field_size_with_label_size<E: crate::text::TextEngine>(
    text_field: &ResolvedTextFieldConfig,
    label_size: LayoutSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
    proposal: ProposalSize,
) -> LayoutSize {
    let metrics = theme.input_field_metrics();
    let line_limit = text_field.line_limit().map(NonZeroUsize::get);
    let prompt = state.measure_signal(&text_field.prompt().content());
    let value = state.measure_signal(text_field.value());
    let prompt_size = HydrolysisRenderer::measure_text_intrinsic_size_with_line_limit(
        state, prompt, env, line_limit,
    );
    let value_size = HydrolysisRenderer::measure_editable_text_size(state, &value, env, line_limit);
    let label_height = measured_input_label_height(label_size, metrics.label_height);
    let text_height = prompt_size.height.max(value_size.height);
    let label_width = metrics
        .horizontal_inset
        .mul_add(2.0, f64::from(label_size.width));

    let field_height = measured_input_field_height(text_height, label_height, metrics);
    let width = input_field_width(
        proposal.width,
        label_width.max(metrics.min_width),
        label_width,
    );
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(field_height),
    )
}

/// Resolves an input field's width from a proposal: every `Some` proposal is
/// answered floored at `minimum` — the `0` probe included — so an unbounded
/// probe answers `INFINITY`; `None` answers the `ideal` (theme `min_width`
/// floor applied). The field's text is not an input to the width.
fn input_field_width(proposal: Option<f32>, ideal: f64, minimum: f64) -> f64 {
    proposal.map_or(ideal, |width| f64::from(width).max(minimum))
}

pub fn measure_secure_field_intrinsic<E: crate::text::TextEngine>(
    secure_field: &SecureFieldConfig,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let label_size = measure_label_intrinsic(secure_field.label(), state, env, theme);
    measure_secure_field_intrinsic_with_label_size(secure_field, label_size, state, env, theme)
}

/// Measures a secure field's intrinsic size from a precomputed label size. The
/// dispatch path passes the label measured via `measure_label_intrinsic`; the
/// retained-node path passes the label measured from its built `RetainedSubview`,
/// so layout and the floating-label render agree on the label height.
pub fn measure_secure_field_intrinsic_with_label_size<E: crate::text::TextEngine>(
    secure_field: &SecureFieldConfig,
    label_size: LayoutSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    measure_secure_field_size_with_label_size(
        secure_field,
        label_size,
        state,
        env,
        theme,
        ProposalSize::UNSPECIFIED,
    )
}

/// Measures a secure field's size under a concrete proposal, from a
/// precomputed label size. Same `Horizontal`-leaf contract as the text field:
/// the width answers the proposal floored at the label's extent and never
/// the masked text; `None` answers the theme's `min_width` ideal.
pub fn measure_secure_field_size_with_label_size<E: crate::text::TextEngine>(
    secure_field: &SecureFieldConfig,
    label_size: LayoutSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
    proposal: ProposalSize,
) -> LayoutSize {
    let metrics = theme.input_field_metrics();
    let secure_len = state
        .measure_signal(secure_field.value())
        .expose()
        .chars()
        .count();
    let masked = StyledStr::plain("*".repeat(secure_len));
    let value_size = HydrolysisRenderer::measure_editable_text_size(state, &masked, env, None);
    let label_height = measured_input_label_height(label_size, metrics.label_height);
    let label_width = metrics
        .horizontal_inset
        .mul_add(2.0, f64::from(label_size.width));
    let field_height = measured_input_field_height(value_size.height, label_height, metrics);
    let width = input_field_width(
        proposal.width,
        label_width.max(metrics.min_width),
        label_width,
    );
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(field_height),
    )
}

/// The band a labeled field reserves for its floating label is exactly the
/// theme's `label_height`: the label scales into the band when it floats, so
/// the unscaled label measure only decides whether the band exists, never
/// how tall it is (water-rs/waterui#2387).
fn measured_input_label_height(label_size: LayoutSize, label_height: f64) -> f64 {
    if label_size.width > 0.0 || label_size.height > 0.0 {
        label_height
    } else {
        0.0
    }
}

fn measured_input_field_height(
    text_height: f32,
    label_height: f64,
    metrics: crate::theme::InputFieldMetrics,
) -> f64 {
    let text_height = f64::from(text_height);
    let measured_height = if label_height > 0.0 {
        label_height + metrics.vertical_inset + text_height + metrics.vertical_inset
    } else {
        metrics.vertical_inset.mul_add(2.0, text_height)
    };
    measured_height.max(metrics.min_height)
}

pub fn measure_table_metrics<E: crate::text::TextEngine>(
    columns: &[TableColumn],
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
    metrics: crate::theme::TableMetrics,
) -> MeasuredTableMetrics {
    let mut column_widths = Vec::with_capacity(columns.len());
    let mut max_rows = 0usize;
    for column in columns {
        let mut width = metrics.min_column_width;
        let label_view = normalize_layout_view(AnyView::new(column.label()), env);
        let label_size = measure_transient_view_intrinsic(&label_view, state, env, theme);
        width = width.max(f64::from(label_size.width) + metrics.cell_horizontal_padding);

        let rows = column.rows();
        max_rows = max_rows.max(state.measure_signal(&rows.len()));
        column_widths.push(width);
    }

    let table_width: f64 = column_widths.iter().sum();
    let table_height = metrics.row_height.mul_add(
        crate::num_cast::usize_as_f64(max_rows),
        metrics.header_height,
    );
    MeasuredTableMetrics {
        column_widths,
        table_width,
        table_height,
    }
}

pub fn refresh_table_slot_baseline<E: crate::text::TextEngine>(
    columns: &[TableColumn],
    slot: &mut LazyTableSlot,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
    metrics: crate::theme::TableMetrics,
) {
    slot.prepare_columns(columns.len(), metrics);
    slot.max_rows = 0;
    for (index, column) in columns.iter().enumerate() {
        let label_view = normalize_layout_view(AnyView::new(column.label()), env);
        let label_size = measure_transient_view_intrinsic(&label_view, state, env, theme);
        let width = (f64::from(label_size.width) + metrics.cell_horizontal_padding)
            .max(metrics.min_column_width);
        if slot.column_widths[index] < width {
            slot.column_widths[index] = width;
        }
        slot.max_rows = slot.max_rows.max(column.rows().len().snapshot());
    }
}

pub fn update_table_slot_visible_cell_widths<E: crate::text::TextEngine>(
    columns: &[TableColumn],
    slot: &mut LazyTableSlot,
    row_window: VisibleIndexWindow,
    col_window: VisibleColumnWindow,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
    metrics: crate::theme::TableMetrics,
) {
    for (column_index, column) in columns
        .iter()
        .enumerate()
        .take(col_window.end)
        .skip(col_window.start)
    {
        let rows = column.rows().snapshot();
        for row_index in row_window.start..row_window.end {
            if let Some(cell) = rows.get_view(row_index) {
                let cell_view = normalize_layout_view(AnyView::new(cell), env);
                let size = measure_transient_view_intrinsic(&cell_view, state, env, theme);
                let width = (f64::from(size.width) + metrics.cell_horizontal_padding)
                    .max(metrics.min_column_width);
                if slot.column_widths[column_index] < width {
                    slot.column_widths[column_index] = width;
                }
            }
        }
    }
}

fn resolved_text_styled<E: crate::text::TextEngine>(
    text: &Text,
    env: &Environment,
    state: &mut HydroState<E>,
) -> StyledStr {
    let resolved = text.resolve(env);
    state.measure_signal(&resolved.content())
}

pub fn measure_date_picker_intrinsic<E: crate::text::TextEngine>(
    date_picker: &DatePickerConfig,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let metrics = theme
        .menu_picker(
            &crate::widgets::controls::date_picker::date_picker_spec(env),
            env,
        )
        .layout();
    let input_metrics = theme.input_field_metrics();
    let label_size = measure_label_intrinsic(date_picker.label(), state, env, theme);
    let has_label = label_size.width > 0.0 || label_size.height > 0.0;
    let label_height = if has_label {
        f64::from(label_size.height).max(input_metrics.label_height)
    } else {
        0.0
    };
    let current = state
        .measure_signal(date_picker.value())
        .clamp(*date_picker.range().start(), *date_picker.range().end());
    let candidates = [
        date_picker.ty().format_value(*date_picker.range().start()),
        date_picker.ty().format_value(current),
        date_picker.ty().format_value(*date_picker.range().end()),
    ];
    let mut field_text_width: f64 = 0.0;
    let mut field_text_height: f64 = 0.0;
    for candidate in candidates {
        let size = HydrolysisRenderer::measure_text_intrinsic_size(
            state,
            StyledStr::plain(candidate),
            env,
        );
        field_text_width = field_text_width.max(f64::from(size.width));
        field_text_height = field_text_height.max(f64::from(size.height));
    }
    let field_width = (input_metrics
        .horizontal_inset
        .mul_add(2.0, field_text_width)
        + metrics.indicator_space)
        .max(input_metrics.min_width);
    let field_height = input_metrics
        .vertical_inset
        .mul_add(2.0, field_text_height)
        .max(input_metrics.min_height);
    let width = f64::from(label_size.width).max(field_width);
    let height = label_height + field_height;
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    )
}

pub fn measure_button_view_intrinsic<E: crate::text::TextEngine>(
    button: &Button<BoxedAction<()>>,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    crate::widgets::controls::button::measure_button_look_intrinsic(
        button.button_style(),
        button.button_size(),
        button.label(),
        state,
        env,
        theme,
    )
}

/// Measures a segmented picker of `items` under `look` beneath a heading of
/// `label_size`: each item's label at the larger of its selected and
/// unselected fonts, laid out by the look's layout.
pub fn measure_segmented_intrinsic<E: crate::text::TextEngine>(
    look: &crate::theme::SegmentedLook,
    items: &[waterui_form::picker::PickerItem<waterui_core::id::Id>],
    label_size: LayoutSize,
    state: &mut HydroState<E>,
    env: &Environment,
) -> LayoutSize {
    if items.is_empty() {
        contract_violation!(
            "hydrolysis picker requires at least one item; the picker measures as zero"
        );
        return LayoutSize::zero();
    }
    let count = items.len();
    let sizes: Vec<LayoutSize> = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let styled = resolved_text_styled(&item.content, env, state);
            crate::widgets::controls::picker::segmented_label_size(
                look,
                &styled,
                crate::theme::Segment { index, count },
                state,
                env,
            )
        })
        .collect();
    crate::widgets::controls::picker::segmented_intrinsic(look.layout(), &sizes, label_size)
}

pub fn measure_picker_intrinsic<E: crate::text::TextEngine>(
    picker: &PickerConfig,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    // The label view is materialized only for this measurement, so it goes
    // through the transient path — see `measure_transient_view_intrinsic`.
    let label_size = measure_transient_view_intrinsic(
        &crate::widgets::controls::picker::menu_picker_label_view(picker.label()),
        state,
        env,
        theme,
    );
    measure_picker_intrinsic_with_label_size(picker, label_size, state, env, theme)
}

/// The intrinsic size of a menu picker's field: the widest item value, the
/// in-field label above it and the look's insets and indicator space.
fn measure_menu_picker_intrinsic<E: crate::text::TextEngine>(
    look: &crate::theme::MenuPickerLook,
    items: &[waterui_form::picker::PickerItem<waterui_core::id::Id>],
    label_size: LayoutSize,
    state: &mut HydroState<E>,
    env: &Environment,
) -> LayoutSize {
    let metrics = look.layout();
    let mut max_item_width: f64 = 0.0;
    let mut max_item_height: f64 = 0.0;
    for item in items {
        let styled = crate::widgets::controls::picker::label_styled(
            look.get_value(),
            resolved_text_styled(&item.content, env, state),
        );
        let size = HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env);
        max_item_width = max_item_width.max(f64::from(size.width));
        max_item_height = max_item_height.max(f64::from(size.height));
    }

    // The field label sits inside the field above the value: it adds its own
    // height plus the metrics' label spacing to the field height, and its
    // width competes with the items for the field's content width. A label
    // measuring zero height (a hidden one) adds nothing — the field keeps its
    // unlabelled size. The render path decides presence the same way, on
    // `label_size.height > 0`.
    let has_label = label_size.height > 0.0;
    let content_width = max_item_width.max(f64::from(label_size.width));
    let width = (metrics.horizontal_inset.mul_add(2.0, content_width) + metrics.indicator_space)
        .max(metrics.min_width);
    let height = metrics
        .vertical_inset
        .mul_add(
            2.0,
            max_item_height
                + f64::from(label_size.height)
                + if has_label {
                    metrics.label_spacing
                } else {
                    0.0
                },
        )
        .max(metrics.min_height);
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    )
}

/// The intrinsic size of a radio group: one row per item, stacked with the
/// look's row spacing, under the group label.
fn measure_radio_picker_intrinsic<E: crate::text::TextEngine>(
    look: &crate::theme::RadioGroupLook,
    items: &[waterui_form::picker::PickerItem<waterui_core::id::Id>],
    label_size: LayoutSize,
    state: &mut HydroState<E>,
    env: &Environment,
) -> LayoutSize {
    let metrics = look.layout();
    let indicator = look.indicator();
    let mut max_item_width: f64 = 0.0;
    let mut total_height = 0.0;
    for (index, item) in items.iter().enumerate() {
        let styled = crate::widgets::controls::picker::label_styled(
            look.get_label(),
            resolved_text_styled(&item.content, env, state),
        );
        let size = HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env);
        max_item_width = max_item_width.max(f64::from(size.width));
        total_height += f64::from(size.height).max(indicator.size().height);
        if index + 1 < items.len() {
            total_height += metrics.row_spacing;
        }
    }
    // A visible group label heads the picker above the option rows: inside
    // the horizontal insets, its width competes with a row's content width,
    // and its height plus the metrics' label spacing stacks on top of the
    // rows. A zero-height (hidden) label adds nothing — same presence rule as
    // the menu field label.
    let content_width = (indicator.size().width + indicator.get_label_spacing() + max_item_width)
        .max(f64::from(label_size.width));
    let width = metrics
        .horizontal_inset
        .mul_add(2.0, content_width)
        .max(metrics.min_width);
    // The minimum height floors the rows block alone: the heading adds on top
    // of it so the rows keep their unlabelled height.
    let rows_height = metrics
        .vertical_inset
        .mul_add(2.0, total_height)
        .max(metrics.min_height);
    let height = rows_height
        + if label_size.height > 0.0 {
            f64::from(label_size.height) + metrics.label_spacing
        } else {
            0.0
        };
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    )
}

/// Measures a picker's intrinsic size from a precomputed label size. The
/// dispatch path passes the label measured via [`measure_picker_intrinsic`];
/// the retained-node path passes the label measured from its built
/// [`crate::renderer::RetainedSubview`], so layout and the in-field label
/// render agree on the label height.
pub fn measure_picker_intrinsic_with_label_size<E: crate::text::TextEngine>(
    picker: &PickerConfig,
    label_size: LayoutSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let items = state.measure_signal(picker.items());
    if items.is_empty() {
        contract_violation!(
            "hydrolysis picker requires at least one item; the picker measures as zero"
        );
        return LayoutSize::zero();
    }
    let item_count = items.len();

    match picker.style() {
        PickerStyle::Automatic | PickerStyle::Menu => {
            let look = theme.menu_picker(
                &crate::widgets::controls::picker::picker_spec(picker, env),
                env,
            );
            measure_menu_picker_intrinsic(&look, &items, label_size, state, env)
        }
        PickerStyle::Radio => {
            let look = theme.radio_group(
                &crate::widgets::controls::picker::picker_spec(picker, env),
                env,
            );
            measure_radio_picker_intrinsic(&look, &items, label_size, state, env)
        }
        PickerStyle::Segmented => {
            let spec = crate::widgets::controls::picker::segmented_spec(picker, item_count, env);
            let look = theme.segmented_picker(&spec, env);
            measure_segmented_intrinsic(&look, &items, label_size, state, env)
        }
        other => {
            contract_violation!(
                "hydrolysis PickerStyle variant {other:?} is not implemented; the picker measures as zero"
            );
            LayoutSize::zero()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{measure_view_dimensions, measured_input_field_height};
    use crate::renderer::tests::{MinimalTestTheme, test_environment};
    use crate::renderer::{HydroState, normalize_layout_view};
    use crate::text::{FontFamilyResolution, ParleyEngine};
    use crate::theme::InputFieldMetrics;
    use crate::theme::WidgetTheme;
    use std::rc::Rc;
    use waterui::theme::install_font_signal;
    use waterui::{Binding, Computed};
    use waterui_controls::text_field::field;
    use waterui_core::{AnyView, Str};
    use waterui_form::secure::{Secure, secure};
    use waterui_text::font::{Body, FontWeight, ResolvedFont};

    #[test]
    fn labeled_input_field_height_reserves_space_for_tall_text() {
        let metrics = InputFieldMetrics::new(18.0, 72.0, 56.0, 16.0, 8.0);

        approx::assert_relative_eq!(measured_input_field_height(22.0, 18.0, metrics), 56.0);
        approx::assert_relative_eq!(measured_input_field_height(34.0, 18.0, metrics), 68.0);
    }

    /// A labeled field reserves exactly the theme's `label_height` for its
    /// floating label: the label's unscaled line must not grow the band
    /// (water-rs/waterui#2387). A 16 pt band, two 8 pt insets and a 24 pt
    /// body line compose 56 pt — the Material 3 filled field height — and
    /// the secure field composes identically. The theme's `min_height`
    /// stays below the composition so the assertion cannot be met by the
    /// floor.
    #[test]
    fn labeled_fields_reserve_the_theme_label_height() {
        let mut env = test_environment();
        install_font_signal::<Body>(
            &mut env,
            Computed::constant(
                ResolvedFont::new(16.0, FontWeight::Normal).with_typography_metrics(24.0, 0.0),
            ),
        );
        let theme: Rc<dyn WidgetTheme> = Rc::new(
            MinimalTestTheme::default()
                .with_input_metrics(InputFieldMetrics::new(16.0, 280.0, 40.0, 16.0, 8.0)),
        );
        let mut state = HydroState::new(ParleyEngine::system(FontFamilyResolution::Strict));

        let value = Binding::container(Str::from("x"));
        let view = normalize_layout_view(AnyView::new(field("Label", &value)), &env);
        let size = measure_view_dimensions(&view, &mut state, &env, &theme).size;
        approx::assert_relative_eq!(f64::from(size.height), 56.0);

        let secret = Binding::container(Secure::new(String::from("x")));
        let view = normalize_layout_view(AnyView::new(secure("Label", &secret)), &env);
        let size = measure_view_dimensions(&view, &mut state, &env, &theme).size;
        approx::assert_relative_eq!(f64::from(size.height), 56.0);
    }

    #[test]
    fn unlabeled_input_field_height_uses_minimum_until_text_needs_more() {
        let metrics = InputFieldMetrics::new(18.0, 72.0, 56.0, 16.0, 8.0);

        approx::assert_relative_eq!(measured_input_field_height(34.0, 0.0, metrics), 56.0);
        approx::assert_relative_eq!(measured_input_field_height(48.0, 0.0, metrics), 64.0);
    }
}

#[cfg(test)]
mod background_tests {
    use super::*;
    use crate::renderer::tests::test_environment;
    use waterui_graphics::color::Color;
    use waterui_text::styled::{Style as TextStyle, StyledStr};

    const BACKGROUND: [u8; 4] = [0, 128, 0, 255];

    /// The premultiplied solid colours a recording draws — glyph brushes and
    /// background fills alike — read off the recorded ops.
    fn solid_fill_colours(scene: &Recording) -> Vec<u32> {
        scene.solid_fill_colours()
    }

    fn rendered_fill_colours(styled: &StyledStr, width: f64) -> Vec<u32> {
        let env = test_environment();
        let mut state = HydroState::new(ParleyEngine::system(FontFamilyResolution::Strict));
        let input = resolve_text_layout_input(styled, HorizontalAlignment::Leading, &env);
        let scene = state.text.record(
            &input,
            Some(crate::num_cast::f64_as_f32(width)),
            TailMark::None,
            &mut state.counters,
            &mut state.text_scenes,
        );
        solid_fill_colours(&scene)
    }

    /// A span's `TextStyle::background` must reach the encoded scene: one fill
    /// per contiguous backgrounded run on each line it covers.
    #[test]
    fn styled_backgrounds_paint_fills_under_their_runs() {
        let expected = u32::from_ne_bytes(BACKGROUND);

        let mut single = StyledStr::empty();
        single.push("before ", TextStyle::new());
        single.push(
            "spoiler",
            TextStyle::new().background(Color::srgb(0, 128, 0)),
        );
        single.push(" after", TextStyle::new());
        let colours = rendered_fill_colours(&single, 300.0);
        assert!(
            colours.contains(&expected),
            "a mid-line span paints its background fill"
        );

        let mut wrapped = StyledStr::empty();
        wrapped.push(
            "the hidden words run long enough to wrap the line ",
            TextStyle::new(),
        );
        wrapped.push(
            "across its own boundary here",
            TextStyle::new().background(Color::srgb(0, 128, 0)),
        );
        wrapped.push(" and out", TextStyle::new());
        let colours = rendered_fill_colours(&wrapped, 120.0);
        assert!(
            colours.iter().filter(|colour| **colour == expected).count() >= 2,
            "a span wrapped over two lines paints one fill per line"
        );
    }
}
