use super::*;
use waterui_core::handler::BoxedAction;
use waterui_form::picker::PickerStyle;
use waterui_form::picker::date::DatePickerConfig;

pub(crate) struct MeasuredTableMetrics {
    pub(crate) column_widths: Vec<f64>,
    pub(crate) table_width: f64,
    pub(crate) table_height: f64,
}

pub(crate) fn table_header_cell_rect(
    origin_x: f64,
    origin_y: f64,
    x_offset: f64,
    width: f64,
    metrics: waterui_backend_core::widget::TableMetrics,
) -> vello::kurbo::Rect {
    vello::kurbo::Rect::new(
        origin_x + x_offset,
        origin_y,
        origin_x + x_offset + width,
        origin_y + metrics.header_height,
    )
}

pub(crate) fn table_data_cell_rect(
    origin_x: f64,
    origin_y: f64,
    x_offset: f64,
    width: f64,
    row_index: usize,
    metrics: waterui_backend_core::widget::TableMetrics,
) -> vello::kurbo::Rect {
    let y0 = origin_y + metrics.header_height + metrics.row_height * row_index as f64;
    vello::kurbo::Rect::new(
        origin_x + x_offset,
        y0,
        origin_x + x_offset + width,
        y0 + metrics.row_height,
    )
}

fn navigation_bar_height(view: &NavigationView, env: &Environment) -> f64 {
    if view.bar.hidden.get() {
        0.0
    } else {
        let metrics = widget_theme(env).navigation_metrics();
        let base =
            navigation_base_bar_height_for_display_mode_metrics(view.bar.display_mode, metrics);
        let search_extra = if view.bar.search.is_some() {
            metrics.search_height + metrics.search_vertical_inset * 2.0
        } else {
            0.0
        };
        base + search_extra
    }
}

pub(crate) fn navigation_base_bar_height_for_display_mode(
    display_mode: waterui::navigation::NavigationTitleDisplayMode,
    env: &Environment,
) -> f64 {
    navigation_base_bar_height_for_display_mode_metrics(
        display_mode,
        widget_theme(env).navigation_metrics(),
    )
}

fn navigation_base_bar_height_for_display_mode_metrics(
    display_mode: waterui::navigation::NavigationTitleDisplayMode,
    metrics: waterui_backend_core::widget::NavigationMetrics,
) -> f64 {
    match display_mode {
        waterui::navigation::NavigationTitleDisplayMode::Automatic => metrics.automatic_bar_height,
        waterui::navigation::NavigationTitleDisplayMode::Inline => metrics.inline_bar_height,
        waterui::navigation::NavigationTitleDisplayMode::Large => metrics.large_bar_height,
    }
}

pub(crate) fn split_compact_threshold(sidebar_width: f64) -> f64 {
    sidebar_width + 360.0
}

pub(crate) fn measure_view_intrinsic(
    view: &AnyView,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    measure_view_dimensions(view, state, env).size
}

/// Measures the intrinsic visual size of a control's [`Label`].
///
/// Hydrolysis caches intrinsic measurements by `AnyView` identity, so the
/// label is type-erased at this single boundary rather than at every call
/// site. The semantic identity of the label remains typed inside the
/// control's config.
pub(crate) fn measure_label_intrinsic(
    label: &waterui_controls::label::Label,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    measure_view_intrinsic(&AnyView::new(label.clone()), state, env)
}

pub(crate) fn measure_view_dimensions(
    view: &AnyView,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    measure_view_dimensions_with_proposal(view, ProposalSize::UNSPECIFIED, state, env)
}

pub(crate) fn measure_view_dimensions_with_proposal(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
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
        measure_view_dimensions_with_proposal_with_budget(view, proposal, state, env, 256);
    state
        .measurement
        .store_view_dimensions(identity, env_identity, proposal, dimensions.clone());
    dimensions
}

fn measure_view_dimensions_with_proposal_with_budget(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
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
            remaining - 1,
        );
    }

    if view.downcast_ref::<()>().is_some() || view.downcast_ref::<Canvas>().is_some() {
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
            remaining - 1,
        );
    }
    if let Some(text) = view.downcast_ref::<Text>() {
        return HydrolysisRenderer::measure_text_dimensions(
            state,
            text.resolve(&scoped_env).content.get(),
            text.resolve(&scoped_env).paragraph_alignment.get(),
            &scoped_env,
            proposal.width,
            None,
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
            remaining - 1,
        );
    }
    if let Some(button) = view.downcast_ref::<Button<BoxedAction<()>>>() {
        return ViewDimensions::new(measure_button_view_intrinsic(button, state, &scoped_env));
    }
    if let Some(dimensions) = dimensions_for_known_native_views(view, proposal, state, &scoped_env)
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

pub(crate) fn measure_layout_dimensions<'a>(
    layout: &dyn Layout,
    children: impl IntoIterator<Item = &'a AnyView>,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    let state = RefCell::new(state);
    let children: Vec<&AnyView> = children.into_iter().collect();
    let mut subviews = Vec::new();
    for child in children {
        subviews.push(HydroSubview::from_view(child, &state, env));
    }
    let refs: Vec<&dyn SubView> = subviews.iter().map(|view| view as &dyn SubView).collect();
    let size = layout.size_that_fits(proposal, &refs);
    if can_skip_layout_alignment_measurement(layout, &subviews) {
        return ViewDimensions::new(size);
    }

    let bounds = LayoutRect::from_size(size);
    let child_rects = layout.place(bounds, &refs);
    let placed_subviews: Vec<PlacedSubview<'_>> = subviews
        .iter()
        .zip(child_rects.iter().copied())
        .map(|(view, frame)| PlacedSubview::new(view as &dyn SubView, frame))
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
    pub(crate) fn render_text_config(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        text: Native<TextConfig>,
        env: &Environment,
    ) {
        let text = text.into_inner();
        let styled = renderer.read_signal(&text.content);
        let alignment = renderer.read_signal(&text.paragraph_alignment);
        Self::render_styled_text(
            &mut renderer.state,
            &mut renderer.scene,
            ctx,
            styled,
            alignment,
            env,
        );
    }

    pub(crate) fn render_styled_text(
        state: &mut HydroState,
        scene: &mut vello::Scene,
        ctx: RenderContext,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
    ) {
        Self::render_styled_text_limited(state, scene, ctx, styled, alignment, env, None);
    }

    pub(crate) fn render_styled_text_limited(
        state: &mut HydroState,
        scene: &mut vello::Scene,
        ctx: RenderContext,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        max_lines: Option<usize>,
    ) {
        let layout = Self::build_text_layout(
            state,
            styled,
            alignment,
            env,
            Some(ctx.bounds.width() as f32),
        );
        Self::draw_text_layout(scene, ctx, &layout, max_lines);
    }

    pub(crate) fn render_styled_text_single_line_centered(
        state: &mut HydroState,
        scene: &mut vello::Scene,
        ctx: RenderContext,
        styled: StyledStr,
        env: &Environment,
    ) {
        let layout =
            Self::build_text_layout(state, styled, HorizontalAlignment::Leading, env, None);
        let Some(line) = layout.lines().next() else {
            return;
        };
        let metrics = line.metrics();
        let width = f64::from(metrics.advance);
        let height = f64::from(metrics.line_height);
        let x = ((ctx.bounds.width() - width) * 0.5).max(0.0);
        let y = ((ctx.bounds.height() - height) * 0.5).max(0.0);
        let child_ctx = ctx.child(
            vello::kurbo::Affine::translate((x, y)),
            vello::kurbo::Rect::new(0.0, 0.0, width, height),
        );
        Self::draw_text_layout(scene, child_ctx, &layout, Some(1));
    }

    fn draw_text_layout(
        scene: &mut vello::Scene,
        ctx: RenderContext,
        layout: &parley::Layout<[u8; 4]>,
        max_lines: Option<usize>,
    ) {
        if layout.is_empty() {
            return;
        }
        let text_transform =
            ctx.transform * vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0));
        for (index, line) in layout.lines().enumerate() {
            if max_lines.is_some_and(|limit| index >= limit) {
                break;
            }
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    let run = glyph_run.run();
                    let style = glyph_run.style();
                    let brush = rgba8_to_peniko(style.brush);
                    let normalized_coords = run.normalized_coords();

                    let mut run_x = glyph_run.offset();
                    let run_y = glyph_run.baseline();
                    let glyphs = glyph_run.glyphs().map(move |glyph| {
                        let x = run_x + glyph.x;
                        let y = run_y - glyph.y;
                        run_x += glyph.advance;
                        vello::Glyph { id: glyph.id, x, y }
                    });

                    let glyph_run_builder = scene
                        .draw_glyphs(run.font())
                        .brush(brush)
                        .transform(text_transform)
                        .font_size(run.font_size());
                    if normalized_coords.is_empty() {
                        glyph_run_builder.draw(vello::peniko::Fill::NonZero, glyphs);
                    } else {
                        let normalized_coords: Vec<vello::NormalizedCoord> =
                            normalized_coords.to_vec();
                        glyph_run_builder
                            .normalized_coords(&normalized_coords)
                            .draw(vello::peniko::Fill::NonZero, glyphs);
                    }
                }
            }
        }
    }

    pub(crate) fn build_text_layout(
        state: &mut HydroState,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        max_width: Option<f32>,
    ) -> parley::Layout<[u8; 4]> {
        let input = resolve_text_layout_input(&styled, alignment, env);
        state.text.shape(&input, max_width)
    }

    pub(crate) fn measure_text_dimensions(
        state: &mut HydroState,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        max_width: Option<f32>,
        max_lines: Option<usize>,
    ) -> ViewDimensions {
        let layout = Self::build_text_layout(state, styled, alignment, env, max_width);
        text_dimensions_from_layout(&layout, max_lines)
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

pub(crate) fn measure_navigation_view_intrinsic(
    navigation: &NavigationView,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let bar_height = navigation_bar_height(navigation, env);
    let title_size = if bar_height > 0.0 {
        measure_view_intrinsic(&navigation.bar.title, state, env)
    } else {
        LayoutSize::zero()
    };
    let leading_size = if !navigation.bar.leading.is::<()>() {
        measure_view_intrinsic(&navigation.bar.leading, state, env)
    } else {
        LayoutSize::zero()
    };
    let trailing_size = if !navigation.bar.trailing.is::<()>() {
        measure_view_intrinsic(&navigation.bar.trailing, state, env)
    } else {
        LayoutSize::zero()
    };
    let search_size = if let Some(search) = navigation.bar.search.as_ref() {
        let body_env = env.clone();
        let search_field = TextField::new(&search.text).prompt(search.prompt.clone());
        let search_body =
            normalize_layout_view(AnyView::new(search_field.body(&body_env)), &body_env);
        measure_view_intrinsic(&search_body, state, &body_env)
    } else {
        LayoutSize::zero()
    };
    let content_size = measure_view_intrinsic(&navigation.content, state, env);
    let metrics = widget_theme(env).navigation_metrics();
    let width = f64::from(content_size.width)
        .max(
            f64::from(leading_size.width)
                + f64::from(title_size.width)
                + f64::from(trailing_size.width)
                + metrics.horizontal_inset * 2.0
                + metrics.item_spacing * 2.0,
        )
        .max(f64::from(search_size.width) + metrics.horizontal_inset * 2.0);
    let height = f64::from(content_size.height) + bar_height;
    LayoutSize::new(width as f32, height as f32)
}

pub(crate) fn measure_owned_navigation_view_intrinsic(
    navigation: NavigationView,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let mut navigation = navigation;
    navigation.bar.title = normalize_layout_view(navigation.bar.title, env);
    navigation.bar.leading = normalize_layout_view(navigation.bar.leading, env);
    navigation.bar.trailing = normalize_layout_view(navigation.bar.trailing, env);
    navigation.content = normalize_layout_view(navigation.content, env);
    measure_navigation_view_intrinsic(&navigation, state, env)
}

pub(crate) fn measure_tabs_intrinsic(
    tabs: &Tabs,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    assert!(
        !(tabs.tabs.is_empty()),
        "hydrolysis Tabs requires at least one tab"
    );

    let mut max_content_width: f64 = 0.0;
    let mut max_content_height: f64 = 0.0;
    let mut bar_width = 0.0;
    let metrics = widget_theme(env).tabs_metrics();
    for tab in &tabs.tabs {
        let label_size = measure_view_intrinsic(&tab.label.content, state, env);
        bar_width += (f64::from(label_size.width) + metrics.button_horizontal_inset * 2.0)
            .max(metrics.button_min_width);

        let content = normalize_layout_view(AnyView::new(tab.content.build()), env);
        let content_size = measure_view_intrinsic(&content, state, env);
        max_content_width = max_content_width.max(f64::from(content_size.width));
        max_content_height = max_content_height.max(f64::from(content_size.height));
    }

    let width = max_content_width.max(bar_width);
    let height = max_content_height + metrics.bar_height;
    LayoutSize::new(width as f32, height as f32)
}

pub(crate) fn tabs_bar_and_content_rect(
    bounds: vello::kurbo::Rect,
    position: TabPosition,
    bar_height: f64,
) -> (vello::kurbo::Rect, vello::kurbo::Rect) {
    let bar_height = bar_height.min(bounds.height());
    match position {
        TabPosition::Top => (
            vello::kurbo::Rect::new(
                bounds.x0,
                bounds.y0,
                bounds.x1,
                (bounds.y0 + bar_height).min(bounds.y1),
            ),
            vello::kurbo::Rect::new(
                bounds.x0,
                (bounds.y0 + bar_height).min(bounds.y1),
                bounds.x1,
                bounds.y1,
            ),
        ),
        TabPosition::Bottom => (
            vello::kurbo::Rect::new(
                bounds.x0,
                (bounds.y1 - bar_height).max(bounds.y0),
                bounds.x1,
                bounds.y1,
            ),
            vello::kurbo::Rect::new(
                bounds.x0,
                bounds.y0,
                bounds.x1,
                (bounds.y1 - bar_height).max(bounds.y0),
            ),
        ),
    }
}

pub(crate) fn tabs_button_rect(
    bar_rect: vello::kurbo::Rect,
    tab_count: usize,
    index: usize,
) -> vello::kurbo::Rect {
    let button_width = bar_rect.width() / tab_count as f64;
    let x0 = bar_rect.x0 + button_width * index as f64;
    vello::kurbo::Rect::new(x0, bar_rect.y0, x0 + button_width, bar_rect.y1)
}

pub(crate) fn navigation_back_button_rect(
    bounds: vello::kurbo::Rect,
    metrics: waterui_backend_core::widget::NavigationMetrics,
) -> vello::kurbo::Rect {
    vello::kurbo::Rect::new(
        bounds.x0 + metrics.back_button_leading_inset,
        bounds.y0 + metrics.back_button_top_inset,
        bounds.x0 + metrics.back_button_leading_inset + metrics.back_button_size,
        bounds.y0 + metrics.back_button_top_inset + metrics.back_button_size,
    )
}

pub(crate) fn measure_list_intrinsic(
    list: &ListConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let row_count = list.contents.len().get();
    if row_count == 0 {
        return LayoutSize::zero();
    }
    let editing = list.editing.get();
    let mut first_item = list
        .contents
        .get_view(0)
        .unwrap_or_else(|| panic!("ListConfig failed to materialize item at index 0"));
    first_item.content = normalize_layout_view(first_item.content, env);
    let content_size = measure_view_intrinsic(&first_item.content, state, env);
    let metrics = widget_theme(env).list_metrics();
    let row_height = (f64::from(content_size.height) + metrics.vertical_inset * 2.0)
        .max(metrics.one_line_row_height);

    let mut row_width = f64::from(content_size.width) + metrics.horizontal_inset * 2.0;
    if editing && list.on_move.is_some() {
        row_width += metrics.move_control_width + metrics.trailing_control_spacing;
    }
    if editing && list.on_delete.is_some() {
        row_width += metrics.delete_control_width + metrics.trailing_control_spacing;
    }
    let total_height = row_height * row_count as f64;
    let max_width = row_width.max(metrics.horizontal_inset * 2.0);

    LayoutSize::new(max_width as f32, total_height as f32)
}

pub(crate) fn materialize_list_item(
    contents: &impl Views<View = ListItem>,
    index: usize,
    env: &Environment,
) -> ListItem {
    let mut item = contents
        .get_view(index)
        .unwrap_or_else(|| panic!("ListConfig failed to materialize item at index {index}"));
    item.content = normalize_layout_view(item.content, env);
    item
}

pub(crate) fn measure_list_item_row_height(
    item: &ListItem,
    state: &mut HydroState,
    env: &Environment,
) -> f64 {
    let intrinsic = measure_view_intrinsic(&item.content, state, env);
    let metrics = widget_theme(env).list_metrics();
    (f64::from(intrinsic.height) + metrics.vertical_inset * 2.0).max(metrics.one_line_row_height)
}

pub(crate) fn materialize_list_row(
    contents: &impl Views<View = ListItem>,
    index: usize,
    state: &mut HydroState,
    env: &Environment,
) -> (ListItem, f64) {
    let item = materialize_list_item(contents, index, env);
    let row_height = measure_list_item_row_height(&item, state, env);
    (item, row_height)
}

pub(crate) fn measure_progress_intrinsic(
    progress: &ProgressConfig,
    _state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    match progress.style {
        ProgressStyle::Linear => {
            let metrics = theme
                .progress_metrics(waterui_backend_core::widget::ProgressIndicatorStyle::Linear);
            let label_height =
                f64::from(waterui_text::font::Font::default().resolve(env).get().size)
                    .max(metrics.label_height);
            let value_label_height = if progress.value.get().is_finite() {
                metrics.value_label_top_spacing + label_height
            } else {
                0.0
            };
            let width = metrics.min_track_width + metrics.bar_horizontal_inset * 2.0;
            let height =
                label_height + metrics.bar_top_offset + metrics.bar_height + value_label_height;
            LayoutSize::new(width as f32, height as f32)
        }
        ProgressStyle::Circular => {
            let metrics = theme
                .progress_metrics(waterui_backend_core::widget::ProgressIndicatorStyle::Circular);
            LayoutSize::new(
                metrics.circular_diameter as f32,
                metrics.circular_diameter as f32,
            )
        }
        _ => panic!("hydrolysis ProgressStyle variant is not implemented"),
    }
}

pub(crate) fn measure_text_field_intrinsic(
    text_field: &ResolvedTextFieldConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let metrics = theme.input_field_metrics();
    let label_size = measure_label_intrinsic(&text_field.label, state, env);
    let line_limit = text_field.line_limit.map(NonZeroUsize::get);
    let prompt = text_field.prompt.content.get();
    let value = text_field.value.get();
    let prompt_size = HydrolysisRenderer::measure_text_intrinsic_size_with_line_limit(
        state, prompt, env, line_limit,
    );
    let value_size = HydrolysisRenderer::measure_text_intrinsic_size_with_line_limit(
        state, value, env, line_limit,
    );
    let label_height = measured_input_label_height(label_size, metrics.label_height);
    let text_height = prompt_size.height.max(value_size.height);
    let content_width =
        f64::from(prompt_size.width.max(value_size.width)) + metrics.horizontal_inset * 2.0;

    let field_width = content_width.max(metrics.min_width);
    let field_height = measured_input_field_height(text_height, label_height, metrics);
    let width = (f64::from(label_size.width) + metrics.horizontal_inset * 2.0).max(field_width);
    LayoutSize::new(width as f32, field_height as f32)
}

pub(crate) fn measure_secure_field_intrinsic(
    secure_field: &SecureFieldConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let metrics = theme.input_field_metrics();
    let label_size = measure_label_intrinsic(&secure_field.label, state, env);
    let secure_len = secure_field.value.get().expose().chars().count();
    let masked = if secure_len == 0 {
        StyledStr::plain("")
    } else {
        StyledStr::plain("*".repeat(secure_len))
    };
    let value_size = HydrolysisRenderer::measure_text_intrinsic_size(state, masked, env);
    let label_height = measured_input_label_height(label_size, metrics.label_height);
    let field_width =
        (f64::from(value_size.width) + metrics.horizontal_inset * 2.0).max(metrics.min_width);
    let field_height = measured_input_field_height(value_size.height, label_height, metrics);
    let width = (f64::from(label_size.width) + metrics.horizontal_inset * 2.0).max(field_width);
    LayoutSize::new(width as f32, field_height as f32)
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
        text_height + metrics.vertical_inset * 2.0
    };
    measured_height.max(metrics.min_height)
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

pub(crate) fn measure_table_metrics(
    columns: &[TableColumn],
    state: &mut HydroState,
    env: &Environment,
) -> MeasuredTableMetrics {
    let metrics = widget_theme(env).table_metrics();
    let mut column_widths = Vec::with_capacity(columns.len());
    let mut max_rows = 0usize;
    for column in columns {
        let mut width = metrics.min_column_width;
        let label_view = normalize_layout_view(AnyView::new(column.label()), env);
        let label_size = measure_view_intrinsic(&label_view, state, env);
        width = width.max(f64::from(label_size.width) + metrics.cell_horizontal_padding);

        let rows = column.rows();
        max_rows = max_rows.max(rows.len().get());
        column_widths.push(width);
    }

    let table_width: f64 = column_widths.iter().sum();
    let table_height = metrics.header_height + metrics.row_height * max_rows as f64;
    MeasuredTableMetrics {
        column_widths,
        table_width,
        table_height,
    }
}

pub(crate) fn refresh_table_slot_baseline(
    columns: &[TableColumn],
    slot: &mut LazyTableSlot,
    state: &mut HydroState,
    env: &Environment,
) {
    let metrics = widget_theme(env).table_metrics();
    slot.prepare_columns(columns.len(), metrics);
    slot.max_rows = 0;
    for (index, column) in columns.iter().enumerate() {
        let label_view = normalize_layout_view(AnyView::new(column.label()), env);
        let label_size = measure_view_intrinsic(&label_view, state, env);
        let width = (f64::from(label_size.width) + metrics.cell_horizontal_padding)
            .max(metrics.min_column_width);
        if slot.column_widths[index] < width {
            slot.column_widths[index] = width;
        }
        slot.max_rows = slot.max_rows.max(column.rows().len().get());
    }
}

pub(crate) fn update_table_slot_visible_cell_widths(
    columns: &[TableColumn],
    slot: &mut LazyTableSlot,
    row_window: VisibleIndexWindow,
    col_window: VisibleColumnWindow,
    state: &mut HydroState,
    env: &Environment,
) {
    let metrics = widget_theme(env).table_metrics();
    for (column_index, column) in columns
        .iter()
        .enumerate()
        .take(col_window.end)
        .skip(col_window.start)
    {
        let rows = column.rows();
        for row_index in row_window.start..row_window.end {
            if let Some(cell) = rows.get_view(row_index) {
                let cell_view = normalize_layout_view(AnyView::new(cell), env);
                let size = measure_view_intrinsic(&cell_view, state, env);
                let width = (f64::from(size.width) + metrics.cell_horizontal_padding)
                    .max(metrics.min_column_width);
                if slot.column_widths[column_index] < width {
                    slot.column_widths[column_index] = width;
                }
            }
        }
    }
}

pub(crate) fn measure_slider_intrinsic(
    slider: &SliderConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let metrics = theme.slider_metrics();
    let label_size = measure_label_intrinsic(&slider.label, state, env);
    let min_label_size = measure_view_intrinsic(&slider.min_value_label, state, env);
    let max_label_size = measure_view_intrinsic(&slider.max_value_label, state, env);

    let control_row_height = (metrics.thumb_radius * 2.0)
        .max(f64::from(min_label_size.height))
        .max(f64::from(max_label_size.height));
    let label_height = f64::from(label_size.height);
    let intrinsic_height = if label_height > 0.0 {
        label_height + metrics.vertical_spacing + control_row_height
    } else {
        control_row_height
    };

    let min_width = f64::from(label_size.width).max(
        f64::from(min_label_size.width)
            + metrics.horizontal_spacing
            + metrics.min_track_width
            + metrics.horizontal_spacing
            + f64::from(max_label_size.width)
            + metrics.horizontal_inset * 2.0,
    );
    LayoutSize::new(min_width as f32, intrinsic_height as f32)
}

fn resolved_text_styled(text: &Text, env: &Environment) -> StyledStr {
    text.resolve(env).content.get()
}

pub(crate) fn measure_date_picker_intrinsic(
    date_picker: &DatePickerConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let metrics = theme.picker_metrics(PickerStyle::Menu);
    let input_metrics = theme.input_field_metrics();
    let label_size = measure_label_intrinsic(&date_picker.label, state, env);
    let has_label = label_size.width > 0.0 || label_size.height > 0.0;
    let label_height = if has_label {
        f64::from(label_size.height).max(input_metrics.label_height)
    } else {
        0.0
    };
    let current = date_picker
        .value
        .get()
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
    let field_width =
        (field_text_width + input_metrics.horizontal_inset * 2.0 + metrics.indicator_space)
            .max(input_metrics.min_width);
    let field_height =
        (field_text_height + input_metrics.vertical_inset * 2.0).max(input_metrics.min_height);
    let width = f64::from(label_size.width).max(field_width);
    let height = label_height + field_height;
    LayoutSize::new(width as f32, height as f32)
}

pub(crate) fn measure_button_view_intrinsic(
    button: &Button<BoxedAction<()>>,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let metrics = theme.button_metrics(button.button_style());
    let label_size = measure_label_intrinsic(button.label(), state, env);
    let content_width = f64::from(label_size.width) + metrics.padding_x * 2.0;
    let content_height = f64::from(label_size.height) + metrics.padding_y * 2.0;
    LayoutSize::new(
        content_width.max(metrics.min_width) as f32,
        content_height.max(metrics.min_height) as f32,
    )
}

pub(crate) fn measure_picker_intrinsic(
    picker: &PickerConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let items = picker.items.get();
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

            let width = (max_item_width + metrics.horizontal_inset * 2.0 + metrics.indicator_space)
                .max(metrics.min_width);
            let height = (max_item_height + metrics.vertical_inset * 2.0).max(metrics.min_height);
            LayoutSize::new(width as f32, height as f32)
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
            let width = (metrics.horizontal_inset * 2.0
                + metrics.radio_indicator_size
                + metrics.radio_label_spacing
                + max_item_width)
                .max(metrics.min_width);
            let height = (metrics.vertical_inset * 2.0 + total_height).max(metrics.min_height);
            LayoutSize::new(width as f32, height as f32)
        }
        PickerStyle::Segmented => {
            let metrics = theme.picker_metrics(PickerStyle::Segmented);
            let mut total_width: f64 = 0.0;
            let mut max_item_height: f64 = 0.0;
            for item in &items {
                let styled = resolved_text_styled(&item.content, env);
                let size = HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env);
                total_width += f64::from(size.width) + metrics.horizontal_inset * 2.0;
                max_item_height = max_item_height.max(f64::from(size.height));
            }
            let width = total_width.max(metrics.min_width);
            let height = (max_item_height + metrics.vertical_inset * 2.0).max(metrics.min_height);
            LayoutSize::new(width as f32, height as f32)
        }
        _ => panic!("hydrolysis PickerStyle variant is not implemented"),
    }
}
