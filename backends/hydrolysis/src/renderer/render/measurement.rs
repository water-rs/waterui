// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::engine::WidgetTheme;
use crate::widgets::nav::tabs::{tabs_decide_layout, tabs_item_natural_width};
use std::rc::Rc;
use waterui_core::views::ViewSnapshot;
use std::sync::Arc;
use waterui::navigation::tab::TabIcon;
use waterui_core::handler::BoxedAction;
use waterui_form::picker::PickerStyle;
use waterui_form::picker::date::DatePickerConfig;

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
    metrics: waterui_backend_core::widget::TableMetrics,
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
    metrics: waterui_backend_core::widget::TableMetrics,
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

fn navigation_bar_height(view: &NavigationView, theme: &Rc<dyn WidgetTheme>) -> f64 {
    if view.bar.hidden.snapshot() {
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
        let bottom_extra = if view.bar.toolbar.items.iter().any(|item| {
            matches!(
                item.placement,
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
    metrics: waterui_backend_core::widget::NavigationMetrics,
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

pub fn measure_view_intrinsic(
    view: &AnyView,
    state: &mut HydroState,
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
pub fn measure_transient_view_intrinsic(
    view: &AnyView,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    measure_transient_view_with_proposal(view, ProposalSize::UNSPECIFIED, state, env, theme)
}

/// Measures a view this measurement materialized rather than one the retained
/// tree owns, under `proposal`. Same contract as
/// [`measure_transient_view_intrinsic`]; the proposal is the rect the layout
/// hands the content, so bounded axes answer the proposal.
pub fn measure_transient_view_with_proposal(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState,
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
pub fn measure_label_intrinsic(
    label: &waterui_controls::label::Label,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    measure_transient_view_intrinsic(&AnyView::new(label.clone()), state, env, theme)
}

pub fn measure_view_dimensions(
    view: &AnyView,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> ViewDimensions {
    measure_view_dimensions_with_proposal(view, ProposalSize::UNSPECIFIED, state, env, theme)
}

pub fn measure_view_dimensions_with_proposal(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState,
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
fn measure_view_dimensions_with_proposal_with_budget(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
    remaining: usize,
) -> ViewDimensions {
    assert!(
        (remaining != 0),
        "hydrolysis view measurement exceeded recursion budget for {}",
        view.name()
    );
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
        return HydrolysisRenderer::measure_text_dimensions(
            state,
            resolved.content.snapshot(),
            resolved.paragraph_alignment.snapshot(),
            &scoped_env,
            proposal.width,
            resolved.line_limit.map(core::num::NonZeroUsize::get),
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
    if let Some(dimensions) =
        dimensions_for_known_native_views(view, proposal, state, &scoped_env, theme)
    {
        return dimensions;
    }

    if view.downcast_ref::<Divider>().is_some() {
        return ViewDimensions::new(LayoutSize::new(1.0, 1.0));
    }

    panic!(
        "hydrolysis dimensions estimation encountered unsupported view type {}",
        view.name()
    );
}

pub fn measure_layout_dimensions<'a>(
    layout: &dyn Layout,
    children: impl IntoIterator<Item = &'a AnyView>,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
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

fn can_skip_layout_alignment_measurement(
    layout: &dyn Layout,
    children: &[HydroSubview<'_>],
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
    if let Some(container) = view.downcast_ref::<Native<FixedContainer>>() {
        let (layout, children) = container.as_inner().as_parts();
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

impl HydrolysisRenderer {
    pub(crate) fn render_styled_text(
        state: &mut HydroState,
        scene: &mut Recording,
        ctx: RenderContext,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
    ) {
        Self::render_styled_text_limited(state, scene, ctx, styled, alignment, env, TailMark::None);
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    pub(crate) fn render_styled_text_limited(
        state: &mut HydroState,
        scene: &mut Recording,
        ctx: RenderContext,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        tail: TailMark,
    ) {
        let input = resolve_text_layout_input(&styled, alignment, env);
        let fragment = state.text.glyph_scene_with(
            &input,
            Some(crate::num_cast::f64_as_f32(ctx.bounds.width())),
            tail,
            |layout, effective, fragment| {
                Self::encode_text_layout(
                    state.text.as_ref(),
                    &mut state.counters,
                    fragment,
                    layout,
                    effective,
                    tail.parts().0,
                );
            },
        );
        scene.append(
            &fragment,
            ctx.transform * kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0)),
        );
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    pub(crate) fn render_styled_text_single_line_centered(
        state: &mut HydroState,
        scene: &mut Recording,
        ctx: RenderContext,
        styled: StyledStr,
        env: &Environment,
    ) {
        let input = resolve_text_layout_input(&styled, HorizontalAlignment::Leading, env);
        let layout = state.text.shape(&input, None);
        let Some(line) = layout.lines().next() else {
            return;
        };
        let metrics = line.metrics();
        // Center the measured frame — advance widened to cover overhanging
        // ink — matching what `text_dimensions_from_layout` reports for it.
        let width = layout_ink_extent(state.text.as_ref(), &layout, Some(1)).map_or_else(
            || f64::from(metrics.advance),
            |(ink_min, ink_max)| f64::from(metrics.advance.max(ink_max) - ink_min.min(0.0)),
        );
        let height = f64::from(metrics.line_height);
        let x = ((ctx.bounds.width() - width) * 0.5).max(0.0);
        let y = ((ctx.bounds.height() - height) * 0.5).max(0.0);
        let fragment = state.text.glyph_scene_with(
            &input,
            None,
            TailMark::Clip(1),
            |layout, effective, fragment| {
                Self::encode_text_layout(
                    state.text.as_ref(),
                    &mut state.counters,
                    fragment,
                    layout,
                    effective,
                    Some(1),
                );
            },
        );
        scene.append(&fragment, ctx.transform * kurbo::Affine::translate((x, y)));
    }

    /// Encode `layout`'s glyph runs into `scene` at the local origin. The
    /// caller positions the result by appending it under a transform, which is
    /// what makes the encoded fragment reusable across frames.
    fn encode_text_layout(
        service: &TextMeasureService,
        counters: &mut FrameWorkCounters,
        scene: &mut Recording,
        layout: &Arc<parley::Layout<[u8; 4]>>,
        input: &ResolvedTextLayoutInput,
        max_lines: Option<usize>,
    ) {
        if layout.is_empty() {
            return;
        }
        // `text_dimensions_from_layout` widens the measured frame so it covers
        // glyph ink that overhangs the pen advance (`layout_ink_extent`); the
        // same left-edge correction shifts the encoded glyphs so that ink
        // starts at the frame origin instead of painting left of it.
        let ink_shift = layout_ink_extent(service, layout, max_lines)
            .map_or(0.0, |(ink_min, _)| -ink_min.min(0.0));
        let paint_backgrounds = input.has_background();
        for (index, line) in layout.lines().enumerate() {
            if max_lines.is_some_and(|limit| index >= limit) {
                break;
            }
            if paint_backgrounds {
                Self::encode_line_backgrounds(scene, &line, input, ink_shift);
            }
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    let run = glyph_run.run();
                    let style = glyph_run.style();
                    let brush = rgba8_to_peniko(style.brush);
                    let normalized_coords = run.normalized_coords();

                    let mut run_x = glyph_run.offset() + ink_shift;
                    let run_y = glyph_run.baseline();
                    let glyphs: Vec<crate::renderer::Glyph> = glyph_run
                        .glyphs()
                        .map(move |glyph| {
                            let x = run_x + glyph.x;
                            let y = run_y - glyph.y;
                            run_x += glyph.advance;
                            crate::renderer::Glyph { id: glyph.id, x, y }
                        })
                        .collect();

                    counters.font_registrations += 1;
                    scene.glyphs(&crate::renderer::GlyphRun {
                        font: run.font(),
                        font_size: run.font_size(),
                        normalized_coords,
                        transform: kurbo::Affine::IDENTITY,
                        brush: &peniko::Brush::Solid(brush),
                        brush_alpha: 1.0,
                        style: peniko::StyleRef::Fill(peniko::Fill::NonZero),
                        glyphs: &glyphs,
                    });
                }
            }
        }
    }

    /// Fill each backgrounded span's glyph extent on `line` — the full line
    /// box (`block_min_coord..block_max_coord`) tall — under the text.
    ///
    /// The horizontal cursor accumulates cluster advances over `runs()` in
    /// display order, the same sequence parley's own glyph-run iterator places
    /// left-to-right (both are driven by `Run::visual_clusters`). These layouts
    /// come from a ranged builder, which emits no inline boxes, so runs are
    /// the whole item sequence.
    fn encode_line_backgrounds(
        scene: &mut Recording,
        line: &parley::Line<'_, [u8; 4]>,
        input: &ResolvedTextLayoutInput,
        ink_shift: f32,
    ) {
        let metrics = line.metrics();
        let (top, bottom) = (
            f64::from(metrics.block_min_coord),
            f64::from(metrics.block_max_coord),
        );
        let mut cursor = metrics.inline_min_coord + metrics.offset + ink_shift;
        // Adjacent clusters with the same background merge into one fill.
        let mut open: Option<(f32, [u8; 4])> = None;
        for run in line.runs() {
            for cluster in run.visual_clusters() {
                let end = cursor + cluster.advance();
                let background = input.span_background(cluster.text_range().start);
                let extends = matches!(
                    (open, background),
                    (Some((_, open_colour)), Some(colour)) if open_colour == colour
                );
                if !extends {
                    if let Some((start, colour)) = open.take() {
                        Self::fill_span_background(scene, start, cursor, top, bottom, colour);
                    }
                    open = background.map(|colour| (cursor, colour));
                }
                cursor = end;
            }
        }
        if let Some((start, colour)) = open {
            Self::fill_span_background(scene, start, cursor, top, bottom, colour);
        }
    }

    fn fill_span_background(
        scene: &mut Recording,
        start: f32,
        end: f32,
        top: f64,
        bottom: f64,
        colour: [u8; 4],
    ) {
        scene.fill(
            peniko::Fill::NonZero,
            kurbo::Affine::IDENTITY,
            &peniko::Brush::Solid(rgba8_to_peniko(colour)),
            None,
            &kurbo::Rect::new(f64::from(start), top, f64::from(end), bottom),
        );
    }

    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the mutable borrow is required by the shared signature even though this implementation does not mutate it"
    )]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    pub(crate) fn build_text_layout(
        state: &mut HydroState,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        max_width: Option<f32>,
    ) -> Arc<parley::Layout<[u8; 4]>> {
        let input = resolve_text_layout_input(&styled, alignment, env);
        state.text.shape(&input, max_width)
    }

    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the mutable borrow is required by the shared signature even though this implementation does not mutate it"
    )]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    pub(crate) fn measure_text_dimensions(
        state: &mut HydroState,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        max_width: Option<f32>,
        max_lines: Option<usize>,
    ) -> ViewDimensions {
        let input = resolve_text_layout_input(&styled, alignment, env);
        let layout = state.text.shape_limited(&input, max_width, max_lines);
        text_dimensions_from_layout(state.text.as_ref(), &layout, max_lines)
    }

    pub(crate) fn measure_text_intrinsic_size(
        state: &mut HydroState,
        styled: StyledStr,
        env: &Environment,
    ) -> LayoutSize {
        Self::measure_text_dimensions(state, styled, HorizontalAlignment::Leading, env, None, None)
            .size
    }

    pub(crate) fn measure_text_intrinsic_size_with_line_limit(
        state: &mut HydroState,
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
}

#[expect(
    clippy::option_if_let_else,
    reason = "the if-let/else mirrors the control flow more clearly than the combinator chain here"
)]
pub fn measure_navigation_view_intrinsic(
    navigation: &NavigationView,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let bar_height = navigation_bar_height(navigation, theme);
    let mut principal_width = 0.0_f64;
    let mut principal_height = 0.0_f64;
    let mut leading_width = 0.0_f64;
    let mut leading_height = 0.0_f64;
    let mut trailing_width = 0.0_f64;
    let mut trailing_height = 0.0_f64;
    let mut bottom_width = 0.0_f64;
    let mut bottom_height = 0.0_f64;
    let metrics = theme.navigation_metrics();
    for item in &navigation.bar.toolbar.items {
        let size = measure_view_intrinsic(&item.content, state, env, theme);
        let (width, height) = match item.placement {
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
        let search_field = TextField::new(search.prompt.clone(), &search.text)
            .hide_label()
            .prompt(search.prompt.clone());
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
pub fn measure_owned_navigation_view_with_proposal(
    navigation: NavigationView,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let navigation = normalize_layout_view(AnyView::new(navigation), env);
    measure_transient_view_with_proposal(&navigation, proposal, state, env, theme)
}

/// Measures a `TabsLayout` under `proposal`: each tab's content answers the
/// proposal its rendered content rect hands it — the pane minus the tab bar —
/// and the layout echoes bounded axes. `intrinsic` is the `UNSPECIFIED` call.
pub fn measure_tabs_layout(
    tabs: &TabsLayout,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    assert!(
        !(tabs.tabs.is_empty()),
        "hydrolysis Tabs requires at least one tab"
    );

    // Labels measure title-only: the bar places the icon itself, so the label
    // must not count it a second time.
    let label_env = env.extending(waterui_controls::label::LabelDisplayMode::TitleOnly);
    let item_sizes: Vec<(LayoutSize, Option<LayoutSize>)> = tabs
        .tabs
        .iter()
        .map(|tab| {
            let label_size = measure_view_intrinsic(&tab.label, state, &label_env, theme);
            let icon_size = tab.icon.as_ref().map(|icon| {
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
        tabs.style,
        proposal.width.map(f64::from),
        &item_sizes,
    );
    let content_proposal = tabs_content_proposal(proposal, tabs.style, metrics.bar_height);
    let mut max_content_width: f64 = 0.0;
    let mut max_content_height: f64 = 0.0;
    let mut bar_width = 0.0;
    for (tab, (label_size, icon_size)) in tabs.tabs.iter().zip(item_sizes.iter()) {
        bar_width += tabs_item_natural_width(*label_size, *icon_size, &metrics, layout);

        let content = normalize_layout_view(AnyView::new(tab.content.build()), env);
        let content_size =
            measure_transient_view_with_proposal(&content, content_proposal, state, env, theme);
        max_content_width = max_content_width.max(f64::from(content_size.width));
        max_content_height = max_content_height.max(f64::from(content_size.height));
    }

    let (width, height) = match tabs.style {
        NativeTabStyle::Automatic | NativeTabStyle::TabBar => (
            max_content_width.max(bar_width),
            max_content_height + metrics.bar_height,
        ),
        NativeTabStyle::Sidebar => (
            max_content_width + metrics.bar_height,
            max_content_height
                .max(metrics.button_min_width * crate::num_cast::usize_as_f64(tabs.tabs.len())),
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

/// The proposal the rendered content rect hands a tab's content: the pane
/// minus the tab bar — a bottom strip for `Automatic`/`TabBar`, a leading
/// strip for `Sidebar` (see [`tabs_bar_and_content_rect`]). Bounded axes echo
/// the offer; an axis the container left open stays open.
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

pub fn tabs_bar_and_content_rect(
    bounds: kurbo::Rect,
    style: NativeTabStyle,
    bar_extent: f64,
) -> (kurbo::Rect, kurbo::Rect) {
    match style {
        NativeTabStyle::Automatic | NativeTabStyle::TabBar => {
            let bar_height = bar_extent.min(bounds.height());
            (
                kurbo::Rect::new(
                    bounds.x0,
                    (bounds.y1 - bar_height).max(bounds.y0),
                    bounds.x1,
                    bounds.y1,
                ),
                kurbo::Rect::new(
                    bounds.x0,
                    bounds.y0,
                    bounds.x1,
                    (bounds.y1 - bar_height).max(bounds.y0),
                ),
            )
        }
        NativeTabStyle::Sidebar => {
            let bar_width = bar_extent.min(bounds.width());
            (
                kurbo::Rect::new(bounds.x0, bounds.y0, bounds.x0 + bar_width, bounds.y1),
                kurbo::Rect::new(bounds.x0 + bar_width, bounds.y0, bounds.x1, bounds.y1),
            )
        }
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
    metrics: waterui_backend_core::widget::NavigationMetrics,
) -> kurbo::Rect {
    kurbo::Rect::new(
        bounds.x0 + metrics.back_button_leading_inset,
        bounds.y0 + metrics.back_button_top_inset,
        bounds.x0 + metrics.back_button_leading_inset + metrics.back_button_size,
        bounds.y0 + metrics.back_button_top_inset + metrics.back_button_size,
    )
}

pub fn measure_list_intrinsic(
    list: &ListConfig,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let row_count = list.contents.len().snapshot();
    if row_count == 0 {
        return LayoutSize::zero();
    }
    let editing = list.editing.snapshot();
    let mut first_item = list
        .contents
        .snapshot().get_view(0)
        .unwrap_or_else(|| panic!("ListConfig failed to materialize item at index 0"));
    first_item.content = normalize_layout_view(first_item.content, env);
    let content_size = measure_transient_view_intrinsic(&first_item.content, state, env, theme);
    let metrics = theme.list_metrics();
    let row_height = metrics
        .vertical_inset
        .mul_add(2.0, f64::from(content_size.height))
        .max(metrics.one_line_row_height);

    let mut row_width = metrics
        .horizontal_inset
        .mul_add(2.0, f64::from(content_size.width));
    if editing && list.on_move.is_some() {
        row_width += metrics.move_control_width + metrics.trailing_control_spacing;
    }
    if editing && list.on_delete.is_some() {
        row_width += metrics.delete_control_width + metrics.trailing_control_spacing;
    }
    // Section chrome is part of the list's own height. Walking every marker is
    // bounded here because only static section content sets `uses_sections`; a
    // virtualized `List::for_each` never does.
    let mut section_height = 0.0;
    if list.uses_sections {
        for index in 0..row_count {
            let Some(section) = list.contents.snapshot().get_view(index).and_then(|item| item.section) else {
                continue;
            };
            if section.label.is_some() {
                section_height += metrics.section_header_height;
            }
            if section.footer.is_some() {
                section_height += metrics.section_footer_height;
            }
        }
    }

    let total_height = row_height.mul_add(crate::num_cast::usize_as_f64(row_count), section_height);
    let max_width = row_width.max(metrics.horizontal_inset * 2.0);

    LayoutSize::new(
        crate::num_cast::f64_as_f32(max_width),
        crate::num_cast::f64_as_f32(total_height),
    )
}

pub fn materialize_list_item(
    contents: &impl Views<View = ListItem>,
    index: usize,
    env: &Environment,
) -> ListItem {
    let mut item = contents
        .snapshot().get_view(index)
        .unwrap_or_else(|| panic!("ListConfig failed to materialize item at index {index}"));
    item.content = normalize_layout_view(item.content, env);
    item
}

/// A list row's extent from its content's measured height: the content plus
/// the row's vertical insets, floored at the configured minimum. `insets` is
/// `ListItem::insets` — `None` uses the theme's `vertical_inset` — and
/// `min_height` is `ListConfig::min_row_height` — `None` uses the theme's
/// `one_line_row_height`, so both unset reproduces the theme metrics exactly.
/// The caller measures the content (a [`measure_transient_view_intrinsic`] on
/// the materialized `ListItem`) and the section chrome height is added on top
/// of what this returns.
pub fn list_row_height_for_content(
    content_height: f64,
    insets: Option<&waterui_layout::padding::EdgeInsets>,
    min_height: Option<f32>,
    metrics: waterui_backend_core::widget::ListMetrics,
) -> f64 {
    let vertical_insets = insets.map_or(metrics.vertical_inset * 2.0, |insets| {
        f64::from(insets.top() + insets.bottom())
    });
    let floor = min_height.map_or(metrics.one_line_row_height, f64::from);
    (content_height + vertical_insets).max(floor)
}

pub fn measure_progress_intrinsic(
    progress: &ProgressConfig,
    _state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    match progress.style {
        ProgressStyle::Linear => {
            let metrics = theme
                .progress_metrics(waterui_backend_core::widget::ProgressIndicatorStyle::Linear);
            let label_height = f64::from(
                waterui_text::font::Font::default()
                    .resolve(env)
                    .snapshot()
                    .size,
            )
            .max(metrics.label_height);
            let value_label_height = if progress.value.snapshot().is_finite() {
                metrics.value_label_top_spacing + label_height
            } else {
                0.0
            };
            let width = metrics
                .bar_horizontal_inset
                .mul_add(2.0, metrics.min_track_width);
            let height =
                label_height + metrics.bar_top_offset + metrics.bar_height + value_label_height;
            LayoutSize::new(
                crate::num_cast::f64_as_f32(width),
                crate::num_cast::f64_as_f32(height),
            )
        }
        ProgressStyle::Circular => {
            let metrics = theme
                .progress_metrics(waterui_backend_core::widget::ProgressIndicatorStyle::Circular);
            LayoutSize::new(
                crate::num_cast::f64_as_f32(metrics.circular_diameter),
                crate::num_cast::f64_as_f32(metrics.circular_diameter),
            )
        }
        _ => panic!("hydrolysis ProgressStyle variant is not implemented"),
    }
}

pub fn measure_text_field_intrinsic(
    text_field: &ResolvedTextFieldConfig,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let label_size = measure_label_intrinsic(&text_field.label, state, env, theme);
    measure_text_field_intrinsic_with_label_size(text_field, label_size, state, env, theme)
}

/// Measures a text field's intrinsic size from a precomputed label size. The
/// dispatch path passes the label measured via `measure_label_intrinsic`; the
/// retained-node path passes the label measured from its built `RetainedSubview`,
/// so layout and the floating-label render agree on the label height.
pub fn measure_text_field_intrinsic_with_label_size(
    text_field: &ResolvedTextFieldConfig,
    label_size: LayoutSize,
    state: &mut HydroState,
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
/// label size. The field is a `Horizontal` leaf: a finite width proposal is
/// answered with that width, a `0` probe with the content minimum (no ideal
/// floor), and `None` with the intrinsic width — where the theme's
/// `min_width` applies as the ideal. Height is always the intrinsic height.
pub fn measure_text_field_size_with_label_size(
    text_field: &ResolvedTextFieldConfig,
    label_size: LayoutSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
    proposal: ProposalSize,
) -> LayoutSize {
    let metrics = theme.input_field_metrics();
    let line_limit = text_field.line_limit.map(NonZeroUsize::get);
    let prompt = text_field.prompt.content.snapshot();
    let value = text_field.value.snapshot();
    let prompt_size = HydrolysisRenderer::measure_text_intrinsic_size_with_line_limit(
        state, prompt, env, line_limit,
    );
    let value_size = HydrolysisRenderer::measure_text_intrinsic_size_with_line_limit(
        state, value, env, line_limit,
    );
    let label_height = measured_input_label_height(label_size, metrics.label_height);
    let text_height = prompt_size.height.max(value_size.height);
    let content_width = metrics
        .horizontal_inset
        .mul_add(2.0, f64::from(prompt_size.width.max(value_size.width)));
    let label_width = metrics
        .horizontal_inset
        .mul_add(2.0, f64::from(label_size.width));

    let field_height = measured_input_field_height(text_height, label_height, metrics);
    let width = input_field_width(
        proposal.width,
        label_width.max(content_width.max(metrics.min_width)),
        label_width.max(content_width),
    );
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(field_height),
    )
}

/// Resolves an input field's width from a proposal: a finite proposal is
/// answered exactly, a `0` probe answers the content minimum, and `None` or
/// an unbounded probe answers the ideal (theme `min_width` floor applied).
fn input_field_width(proposal: Option<f32>, ideal: f64, minimum: f64) -> f64 {
    match proposal {
        Some(0.0) => minimum,
        Some(width) if width.is_finite() => f64::from(width.max(0.0)),
        _ => ideal,
    }
}

pub fn measure_secure_field_intrinsic(
    secure_field: &SecureFieldConfig,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let label_size = measure_label_intrinsic(&secure_field.label, state, env, theme);
    measure_secure_field_intrinsic_with_label_size(secure_field, label_size, state, env, theme)
}

/// Measures a secure field's intrinsic size from a precomputed label size. The
/// dispatch path passes the label measured via `measure_label_intrinsic`; the
/// retained-node path passes the label measured from its built `RetainedSubview`,
/// so layout and the floating-label render agree on the label height.
pub fn measure_secure_field_intrinsic_with_label_size(
    secure_field: &SecureFieldConfig,
    label_size: LayoutSize,
    state: &mut HydroState,
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
/// a finite width proposal is answered exactly, `0` probes the content
/// minimum, `None` the intrinsic width with the theme's `min_width` ideal.
pub fn measure_secure_field_size_with_label_size(
    secure_field: &SecureFieldConfig,
    label_size: LayoutSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
    proposal: ProposalSize,
) -> LayoutSize {
    let metrics = theme.input_field_metrics();
    let secure_len = secure_field.value.snapshot().expose().chars().count();
    let masked = if secure_len == 0 {
        StyledStr::plain("")
    } else {
        StyledStr::plain("*".repeat(secure_len))
    };
    let value_size = HydrolysisRenderer::measure_text_intrinsic_size(state, masked, env);
    let label_height = measured_input_label_height(label_size, metrics.label_height);
    let content_width = metrics
        .horizontal_inset
        .mul_add(2.0, f64::from(value_size.width));
    let label_width = metrics
        .horizontal_inset
        .mul_add(2.0, f64::from(label_size.width));
    let field_height = measured_input_field_height(value_size.height, label_height, metrics);
    let width = input_field_width(
        proposal.width,
        label_width.max(content_width.max(metrics.min_width)),
        label_width.max(content_width),
    );
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(field_height),
    )
}

fn measured_input_label_height(label_size: LayoutSize, min_label_height: f64) -> f64 {
    if label_size.width > 0.0 || label_size.height > 0.0 {
        f64::from(label_size.height).max(min_label_height)
    } else {
        0.0
    }
}

fn measured_input_field_height(
    text_height: f32,
    label_height: f64,
    metrics: waterui_backend_core::widget::InputFieldMetrics,
) -> f64 {
    let text_height = f64::from(text_height);
    let measured_height = if label_height > 0.0 {
        label_height + metrics.vertical_inset + text_height + metrics.vertical_inset
    } else {
        metrics.vertical_inset.mul_add(2.0, text_height)
    };
    measured_height.max(metrics.min_height)
}

pub fn measure_table_metrics(
    columns: &[TableColumn],
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> MeasuredTableMetrics {
    let metrics = theme.table_metrics();
    let mut column_widths = Vec::with_capacity(columns.len());
    let mut max_rows = 0usize;
    for column in columns {
        let mut width = metrics.min_column_width;
        let label_view = normalize_layout_view(AnyView::new(column.label()), env);
        let label_size = measure_transient_view_intrinsic(&label_view, state, env, theme);
        width = width.max(f64::from(label_size.width) + metrics.cell_horizontal_padding);

        let rows = column.rows();
        max_rows = max_rows.max(rows.len().snapshot());
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

pub fn refresh_table_slot_baseline(
    columns: &[TableColumn],
    slot: &mut LazyTableSlot,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) {
    let metrics = theme.table_metrics();
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

pub fn update_table_slot_visible_cell_widths(
    columns: &[TableColumn],
    slot: &mut LazyTableSlot,
    row_window: VisibleIndexWindow,
    col_window: VisibleColumnWindow,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) {
    let metrics = theme.table_metrics();
    for (column_index, column) in columns
        .iter()
        .enumerate()
        .take(col_window.end)
        .skip(col_window.start)
    {
        let rows = column.rows();
        for row_index in row_window.start..row_window.end {
            if let Some(cell) = rows.snapshot().get_view(row_index) {
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

pub fn measure_slider_intrinsic(
    slider: &SliderConfig,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let metrics = theme.slider_metrics(slider.size);
    let label_size = measure_label_intrinsic(&slider.label, state, env, theme);
    let min_label_size = measure_view_intrinsic(&slider.min_value_label, state, env, theme);
    let max_label_size = measure_view_intrinsic(&slider.max_value_label, state, env, theme);

    let control_row_height = metrics
        .handle_height
        .max(f64::from(min_label_size.height))
        .max(f64::from(max_label_size.height));
    let label_height = f64::from(label_size.height);
    let intrinsic_height = if label_height > 0.0 {
        label_height + metrics.vertical_spacing + control_row_height
    } else {
        control_row_height
    };

    let min_width = f64::from(label_size.width).max(metrics.horizontal_inset.mul_add(
        2.0,
        f64::from(min_label_size.width)
            + metrics.horizontal_spacing
            + metrics.min_track_width
            + metrics.horizontal_spacing
            + f64::from(max_label_size.width),
    ));
    LayoutSize::new(
        crate::num_cast::f64_as_f32(min_width),
        crate::num_cast::f64_as_f32(intrinsic_height),
    )
}

fn resolved_text_styled(text: &Text, env: &Environment) -> StyledStr {
    text.resolve(env).content.snapshot()
}

pub fn measure_date_picker_intrinsic(
    date_picker: &DatePickerConfig,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let metrics = theme.picker_metrics(PickerStyle::Menu);
    let input_metrics = theme.input_field_metrics();
    let label_size = measure_label_intrinsic(&date_picker.label, state, env, theme);
    let has_label = label_size.width > 0.0 || label_size.height > 0.0;
    let label_height = if has_label {
        f64::from(label_size.height).max(input_metrics.label_height)
    } else {
        0.0
    };
    let current = date_picker
        .value
        .snapshot()
        .clamp(*date_picker.range.start(), *date_picker.range.end());
    let candidates = [
        date_picker.ty.format_value(*date_picker.range.start()),
        date_picker.ty.format_value(current),
        date_picker.ty.format_value(*date_picker.range.end()),
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

pub fn measure_button_view_intrinsic(
    button: &Button<BoxedAction<()>>,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let metrics = if crate::widgets::controls::button::label_resolves_icon_only(button.label(), env)
    {
        theme.icon_button_metrics(button.button_style(), button.button_size())
    } else {
        theme.button_metrics(button.button_style(), button.button_size())
    };
    let label_size = measure_label_intrinsic(button.label(), state, env, theme);
    let content_width = f64::mul_add(metrics.padding_x, 2.0, f64::from(label_size.width));
    let content_height = f64::mul_add(metrics.padding_y, 2.0, f64::from(label_size.height));
    LayoutSize::new(
        crate::num_cast::f64_as_f32(content_width.max(metrics.min_width)),
        crate::num_cast::f64_as_f32(content_height.max(metrics.min_height)),
    )
}

pub fn measure_picker_intrinsic(
    picker: &PickerConfig,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    // The label view is materialized only for this measurement, so it goes
    // through the transient path — see `measure_transient_view_intrinsic`.
    let label_size = measure_transient_view_intrinsic(
        &crate::widgets::controls::picker::menu_picker_label_view(&picker.label),
        state,
        env,
        theme,
    );
    measure_picker_intrinsic_with_label_size(picker, label_size, state, env, theme)
}

/// Measures a picker's intrinsic size from a precomputed label size. The
/// dispatch path passes the label measured via [`measure_picker_intrinsic`];
/// the retained-node path passes the label measured from its built
/// [`crate::renderer::RetainedSubview`], so layout and the in-field label
/// render agree on the label height.
// one continuous intrinsic-measurement pass per widget kind; splitting it would only mirror the match arms artificially
#[expect(
    clippy::too_many_lines,
    reason = "the measurement walks each picker's structure end to end; the length is the enumeration, not logic"
)]
pub fn measure_picker_intrinsic_with_label_size(
    picker: &PickerConfig,
    label_size: LayoutSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let items = picker.items.snapshot();
    assert!(
        !(items.is_empty()),
        "hydrolysis picker requires at least one item"
    );
    let item_count = items.len();

    match picker.style {
        PickerStyle::Automatic | PickerStyle::Menu => {
            let metrics = theme.picker_metrics(PickerStyle::Menu);
            let mut max_item_width: f64 = 0.0;
            let mut max_item_height: f64 = 0.0;
            for item in &items {
                let styled = resolved_text_styled(&item.content, env);
                let size = HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env);
                max_item_width = max_item_width.max(f64::from(size.width));
                max_item_height = max_item_height.max(f64::from(size.height));
            }

            // The field label sits inside the field above the value: it adds
            // its own height plus the metrics' label spacing to the field
            // height, and its width competes with the items for the field's
            // content width. A label measuring zero height (a hidden one)
            // adds nothing — the field keeps its unlabelled size. The render
            // path decides presence the same way, on `label_size.height > 0`.
            let has_label = label_size.height > 0.0;
            let content_width = max_item_width.max(f64::from(label_size.width));
            let width = (metrics.horizontal_inset.mul_add(2.0, content_width)
                + metrics.indicator_space)
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
        PickerStyle::Radio => {
            let metrics = theme.picker_metrics(PickerStyle::Radio);
            let mut max_item_width: f64 = 0.0;
            let mut total_height = 0.0;
            for (index, item) in items.iter().enumerate() {
                let styled = resolved_text_styled(&item.content, env);
                let size = HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env);
                max_item_width = max_item_width.max(f64::from(size.width));
                total_height += f64::from(size.height).max(metrics.radio_indicator_size);
                if index + 1 < item_count {
                    total_height += metrics.radio_row_spacing;
                }
            }
            // A visible group label heads the picker above the option rows:
            // inside the horizontal insets, its width competes with a row's
            // content width, and its height plus the metrics' label spacing
            // stacks on top of the rows. A zero-height (hidden) label adds
            // nothing — same presence rule as the menu field label.
            let content_width =
                (metrics.radio_indicator_size + metrics.radio_label_spacing + max_item_width)
                    .max(f64::from(label_size.width));
            let width = metrics
                .horizontal_inset
                .mul_add(2.0, content_width)
                .max(metrics.min_width);
            // The minimum height floors the rows block alone: the heading
            // adds on top of it so the rows keep their unlabelled height.
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
        PickerStyle::Segmented => {
            let metrics = theme.picker_metrics(PickerStyle::Segmented);
            let mut total_width: f64 = 0.0;
            let mut max_item_height: f64 = 0.0;
            for item in &items {
                let styled = resolved_text_styled(&item.content, env);
                let size = HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env);
                total_width += metrics
                    .horizontal_inset
                    .mul_add(2.0, f64::from(size.width))
                    .max(metrics.segment_min_width);
                max_item_height = max_item_height.max(f64::from(size.height));
            }
            // A visible group label heads the segment row edge to edge: its
            // width competes with the row's total width with no inset term,
            // and it adds its height plus the metrics' label spacing on top,
            // so the row keeps its unlabelled height. A zero-height (hidden)
            // label adds nothing — same presence rule as the menu field label.
            let width = total_width
                .max(f64::from(label_size.width))
                .max(metrics.min_width);
            // The minimum height floors the row alone: the heading adds on
            // top of it so the row keeps its unlabelled height.
            let row_height = metrics
                .vertical_inset
                .mul_add(2.0, max_item_height)
                .max(metrics.min_height);
            let height = row_height
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
        _ => panic!("hydrolysis PickerStyle variant is not implemented"),
    }
}

#[cfg(test)]
mod tests {
    use super::measured_input_field_height;
    use waterui_backend_core::widget::InputFieldMetrics;

    #[test]
    fn labeled_input_field_height_reserves_space_for_tall_text() {
        let metrics = InputFieldMetrics::new(18.0, 72.0, 56.0, 16.0, 8.0);

        assert_eq!(measured_input_field_height(22.0, 18.0, metrics), 56.0);
        assert_eq!(measured_input_field_height(34.0, 18.0, metrics), 68.0);
    }

    #[test]
    fn unlabeled_input_field_height_uses_minimum_until_text_needs_more() {
        let metrics = InputFieldMetrics::new(18.0, 72.0, 56.0, 16.0, 8.0);

        assert_eq!(measured_input_field_height(34.0, 0.0, metrics), 56.0);
        assert_eq!(measured_input_field_height(48.0, 0.0, metrics), 64.0);
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

    fn rendered_fill_colours(styled: StyledStr, width: f64) -> Vec<u32> {
        let env = test_environment();
        let mut state = HydroState::default();
        let mut scene = Recording::new();
        let ctx = RenderContext::with_transforms(
            kurbo::Rect::new(0.0, 0.0, width, 200.0),
            kurbo::Affine::IDENTITY,
            kurbo::Affine::IDENTITY,
        );
        HydrolysisRenderer::render_styled_text_limited(
            &mut state,
            &mut scene,
            ctx,
            styled,
            HorizontalAlignment::Leading,
            &env,
            TailMark::None,
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
        let colours = rendered_fill_colours(single, 300.0);
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
        let colours = rendered_fill_colours(wrapped, 120.0);
        assert!(
            colours.iter().filter(|colour| **colour == expected).count() >= 2,
            "a span wrapped over two lines paints one fill per line"
        );
    }
}
