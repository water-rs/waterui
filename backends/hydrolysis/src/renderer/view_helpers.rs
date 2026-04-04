use super::*;

pub(crate) fn gesture_group_identity(view: &AnyView) -> usize {
    gesture_group_identity_with_budget(view, 64)
}

pub(crate) fn flatten_environment_metadata_ref<'a>(
    mut view: &'a AnyView,
    env: &Environment,
) -> (&'a AnyView, Environment) {
    let mut scoped_env = env.clone();
    while let Some(metadata) = view.downcast_ref::<Metadata<Environment>>() {
        scoped_env = local_state_overlay_env(&metadata.value, &scoped_env);
        view = &metadata.content;
    }
    (view, scoped_env)
}

pub(crate) fn flatten_environment_metadata_owned(
    mut view: AnyView,
    env: &Environment,
) -> (AnyView, Environment) {
    let mut scoped_env = env.clone();
    while view.is::<Metadata<Environment>>() {
        let Metadata { content, value } = *view
            .downcast::<Metadata<Environment>>()
            .expect("environment metadata flattening downcast must succeed");
        scoped_env = local_state_overlay_env(&value, &scoped_env);
        view = content;
    }
    (view, scoped_env)
}

fn gesture_group_identity_with_budget(view: &AnyView, remaining: usize) -> usize {
    assert!(
        !(remaining == 0),
        "hydrolysis gesture group identity extraction exceeded recursion budget for {}",
        view.name()
    );
    if let Some(metadata) = view.downcast_ref::<Metadata<Environment>>() {
        return gesture_group_identity_with_budget(&metadata.content, remaining - 1);
    }
    if let Some(content) = passthrough_content(view) {
        return gesture_group_identity_with_budget(content, remaining - 1);
    }
    view.stable_ptr() as usize
}

pub(crate) fn passthrough_content<'a>(view: &'a AnyView) -> Option<&'a AnyView> {
    macro_rules! passthrough_metadata_content {
        ($($ty:ty),+ $(,)?) => {
            $(
                if let Some(metadata) = view.downcast_ref::<Metadata<$ty>>() {
                    return Some(&metadata.content);
                }
            )+
        };
    }

    macro_rules! passthrough_ignorable_metadata_content {
        ($($ty:ty),+ $(,)?) => {
            $(
                if let Some(metadata) = view.downcast_ref::<IgnorableMetadata<$ty>>() {
                    return Some(&metadata.content);
                }
            )+
        };
    }

    passthrough_metadata_content!(
        Environment,
        Retain,
        Opacity,
        AppliedFilter,
        Scale,
        Rotation,
        Offset,
        ClipShape,
        Border,
        Shadow,
        Focused,
        Hittable,
        GestureObserver,
        LifeCycleHook,
        OnEvent,
        Secure,
        StandardDynamicRange,
        HighDynamicRange,
        Cursor,
        IgnoreSafeArea,
        ContextMenu,
        ResolvedContextMenu,
        Draggable,
        DropDestination,
        Background
    );
    passthrough_ignorable_metadata_content!(
        MaterialBackground,
        AccessibilityLabel,
        AccessibilityRole,
        AccessibilityHidden,
        AccessibilityChildren,
        AccessibilityState,
        AccessibilityStateSignal
    );

    None
}

pub(crate) fn effective_stretch_axis(view: &AnyView) -> StretchAxis {
    if let Some(content) = passthrough_content(view) {
        return effective_stretch_axis(content);
    }
    view.stretch_axis()
}

fn is_layout_terminal(view: &AnyView) -> bool {
    if view.downcast_ref::<Str>().is_some() || view.downcast_ref::<Divider>().is_some() {
        return true;
    }
    super::is_hydro_native_view(view)
}

pub(crate) fn normalize_view_for_render(view: AnyView, env: &Environment) -> AnyView {
    normalize_layout_view(view, env)
}

pub(crate) fn normalize_layout_view(view: AnyView, env: &Environment) -> AnyView {
    normalize_layout_view_with_budget(view, env, 64)
}

fn normalize_layout_view_with_budget(
    view: AnyView,
    env: &Environment,
    remaining: usize,
) -> AnyView {
    assert!(
        !(remaining == 0),
        "hydrolysis layout normalization exceeded recursion budget for {}",
        view.name()
    );
    let next_remaining = remaining - 1;
    let mut view = view;

    if view.is::<Metadata<Environment>>() {
        let Metadata { content, value } = *view
            .downcast::<Metadata<Environment>>()
            .expect("layout normalization failed to downcast Metadata<Environment>");
        let scoped_env = local_state_overlay_env(&value, env);
        let normalized_content =
            normalize_layout_view_with_budget(content, &scoped_env, next_remaining);
        return AnyView::new(Metadata {
            content: normalized_content,
            value,
        });
    }
    macro_rules! normalize_passthrough_metadata {
        ($($ty:ty),+ $(,)?) => {
            $(
                if view.is::<Metadata<$ty>>() {
                    let Metadata { content, value } = *view
                        .downcast::<Metadata<$ty>>()
                        .expect("layout normalization failed to downcast metadata");
                    let normalized_content =
                        normalize_layout_view_with_budget(content, env, next_remaining);
                    return AnyView::new(Metadata {
                        content: normalized_content,
                        value,
                    });
                }
            )+
        };
    }
    macro_rules! normalize_passthrough_ignorable_metadata {
        ($($ty:ty),+ $(,)?) => {
            $(
                if view.is::<IgnorableMetadata<$ty>>() {
                    let IgnorableMetadata { content, value } = *view
                        .downcast::<IgnorableMetadata<$ty>>()
                        .expect("layout normalization failed to downcast ignorable metadata");
                    let normalized_content =
                        normalize_layout_view_with_budget(content, env, next_remaining);
                    return AnyView::new(IgnorableMetadata {
                        content: normalized_content,
                        value,
                    });
                }
            )+
        };
    }

    normalize_passthrough_metadata!(
        Retain,
        Opacity,
        AppliedFilter,
        Scale,
        Rotation,
        Offset,
        ClipShape,
        Border,
        Shadow,
        Focused,
        Hittable,
        GestureObserver,
        LifeCycleHook,
        OnEvent,
        Secure,
        StandardDynamicRange,
        HighDynamicRange,
        Cursor,
        IgnoreSafeArea,
        ContextMenu,
        ResolvedContextMenu,
        Draggable,
        DropDestination,
        Background
    );
    normalize_passthrough_ignorable_metadata!(
        MaterialBackground,
        AccessibilityLabel,
        AccessibilityRole,
        AccessibilityHidden,
        AccessibilityChildren,
        AccessibilityState,
        AccessibilityStateSignal
    );

    if view.is::<Native<FixedContainer>>() {
        let native = *view
            .downcast::<Native<FixedContainer>>()
            .expect("layout normalization failed to downcast Native<FixedContainer>");
        let (layout, children) = native.into_inner().into_inner();
        let mut normalized_children = Vec::with_capacity(children.len());
        for (index, child) in children.into_iter().enumerate() {
            let child_env = local_state_child_env(env, index);
            normalized_children.push(normalize_layout_view_with_budget(
                child,
                &child_env,
                next_remaining,
            ));
        }
        return AnyView::new(Native::new(FixedContainer::from_parts(
            layout,
            normalized_children,
        )));
    }

    if view.is::<Native<LazyContainer>>() {
        return view;
    }

    if view.is::<Native<ScrollView>>() {
        let native = *view
            .downcast::<Native<ScrollView>>()
            .expect("layout normalization failed to downcast Native<ScrollView>");
        let (axis, content) = native.into_inner().into_inner();
        let child_env = local_state_child_env(env, 0);
        let normalized_content =
            normalize_layout_view_with_budget(content, &child_env, next_remaining);
        return AnyView::new(Native::new(ScrollView::new(axis, normalized_content)));
    }

    if is_layout_terminal(&view) {
        return view;
    }

    let body_env = local_state_body_env(env);
    let body_content_env = local_state_body_content_env(env);
    view = AnyView::new(view.body(&body_env));
    normalize_layout_view_with_budget(view, &body_content_env, next_remaining)
}

pub(crate) fn estimate_layout_intrinsic<'a>(
    layout: &dyn Layout,
    children: impl IntoIterator<Item = &'a AnyView>,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let state = RefCell::new(state);
    let children: Vec<&AnyView> = children.into_iter().collect();
    let child_envs: Vec<Environment> = children
        .iter()
        .enumerate()
        .map(|(index, _)| local_state_child_env(env, index))
        .collect();
    let mut subviews = Vec::new();
    for (child, child_env) in children.into_iter().zip(&child_envs) {
        subviews.push(HydroSubview::from_view(child, &state, child_env));
    }
    let refs: Vec<&dyn SubView> = subviews.iter().map(|view| view as &dyn SubView).collect();
    layout.size_that_fits(ProposalSize::UNSPECIFIED, &refs)
}

pub(crate) fn resolved_color_to_peniko(color: ResolvedColor) -> vello::peniko::Color {
    let srgb = color.to_srgb_with_headroom();
    vello::peniko::Color::new([srgb.red, srgb.green, srgb.blue, color.opacity])
}

pub(crate) fn resolved_gradient_to_brush(
    gradient: &ResolvedGradient,
    bounds: vello::kurbo::Rect,
) -> vello::peniko::Brush {
    let mut stops: Vec<vello::peniko::ColorStop> =
        gradient.stops.iter().map(to_peniko_stop).collect();

    let brush = match gradient.gradient_type {
        GradientType::Linear => {
            let start = resolved_point_to_kurbo(gradient.start_point, bounds);
            let end = resolved_point_to_kurbo(gradient.end_point, bounds);
            vello::peniko::Gradient::new_linear(start, end).with_stops(&*stops)
        }
        GradientType::Radial => {
            let center = resolved_point_to_kurbo(gradient.start_point, bounds);
            let radius_scale = bounds.width().min(bounds.height()) as f32;
            let start_radius = gradient.start_value * radius_scale;
            let end_radius = gradient.end_value * radius_scale;
            vello::peniko::Gradient::new_two_point_radial(center, start_radius, center, end_radius)
                .with_stops(&*stops)
        }
        GradientType::Angular => {
            let sweep = gradient.end_value - gradient.start_value;
            let sweep_fraction = f64::from(sweep) / TAU;
            if sweep_fraction < 1.0 {
                let last_color = stops
                    .last()
                    .expect("resolved gradient must contain at least one stop")
                    .color;
                for stop in &mut stops {
                    stop.offset = (f64::from(stop.offset) * sweep_fraction) as f32;
                }
                stops.push(vello::peniko::ColorStop {
                    offset: sweep_fraction as f32,
                    color: last_color,
                });
                stops.push(vello::peniko::ColorStop {
                    offset: 1.0,
                    color: last_color,
                });
            }
            let center = resolved_point_to_kurbo(gradient.start_point, bounds);
            vello::peniko::Gradient::new_sweep(center, gradient.start_value, 0.0)
                .with_stops(&*stops)
        }
        GradientType::Mesh => {
            panic!("resolved mesh gradient must not be dispatched through ResolvedGradient")
        }
    };

    vello::peniko::Brush::Gradient(brush)
}

fn resolved_point_to_kurbo(point: [f32; 2], bounds: vello::kurbo::Rect) -> vello::kurbo::Point {
    vello::kurbo::Point::new(
        f64::from(point[0]) * bounds.width(),
        f64::from(point[1]) * bounds.height(),
    )
}

fn to_peniko_stop(stop: &ResolvedGradientStop) -> vello::peniko::ColorStop {
    vello::peniko::ColorStop {
        offset: stop.position,
        color: resolved_color_to_peniko(stop.color).into(),
    }
}

pub(crate) fn resolved_shape_to_path(
    shape: &ResolvedShape,
    bounds: vello::kurbo::Rect,
) -> vello::kurbo::BezPath {
    path_commands_to_path(&shape.commands, bounds)
}

pub(crate) fn path_commands_to_path(
    commands: &[PathCommand],
    bounds: vello::kurbo::Rect,
) -> vello::kurbo::BezPath {
    let width = bounds.width();
    let height = bounds.height();
    let mut path = vello::kurbo::BezPath::new();
    let mut has_current = false;

    for command in commands {
        match command {
            PathCommand::MoveTo { x, y } => {
                path.move_to(vello::kurbo::Point::new(
                    f64::from(*x) * width,
                    f64::from(*y) * height,
                ));
                has_current = true;
            }
            PathCommand::LineTo { x, y } => {
                assert!(
                    !(!has_current),
                    "PathCommand::LineTo requires an active current point"
                );
                path.line_to(vello::kurbo::Point::new(
                    f64::from(*x) * width,
                    f64::from(*y) * height,
                ));
            }
            PathCommand::QuadTo { cx, cy, x, y } => {
                assert!(
                    !(!has_current),
                    "PathCommand::QuadTo requires an active current point"
                );
                path.quad_to(
                    vello::kurbo::Point::new(f64::from(*cx) * width, f64::from(*cy) * height),
                    vello::kurbo::Point::new(f64::from(*x) * width, f64::from(*y) * height),
                );
            }
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => {
                assert!(
                    !(!has_current),
                    "PathCommand::CubicTo requires an active current point"
                );
                path.curve_to(
                    vello::kurbo::Point::new(f64::from(*c1x) * width, f64::from(*c1y) * height),
                    vello::kurbo::Point::new(f64::from(*c2x) * width, f64::from(*c2y) * height),
                    vello::kurbo::Point::new(f64::from(*x) * width, f64::from(*y) * height),
                );
            }
            PathCommand::Arc {
                cx,
                cy,
                rx,
                ry,
                start,
                sweep,
            } => {
                let center_x = f64::from(*cx) * width;
                let center_y = f64::from(*cy) * height;
                let radius_x = f64::from(*rx) * width;
                let radius_y = f64::from(*ry) * height;
                let start = f64::from(*start);
                let step = f64::from(*sweep) / 32.0;

                let start_point = vello::kurbo::Point::new(
                    center_x + radius_x * start.cos(),
                    center_y + radius_y * start.sin(),
                );
                if has_current {
                    path.line_to(start_point);
                } else {
                    path.move_to(start_point);
                    has_current = true;
                }

                let mut angle = start;
                for _ in 0..32 {
                    angle += step;
                    path.line_to(vello::kurbo::Point::new(
                        center_x + radius_x * angle.cos(),
                        center_y + radius_y * angle.sin(),
                    ));
                }
            }
            PathCommand::Close => {
                path.close_path();
                has_current = false;
            }
        }
    }

    path
}

pub(crate) fn anchor_point(
    bounds: vello::kurbo::Rect,
    anchor: waterui::style::Anchor,
) -> vello::kurbo::Point {
    vello::kurbo::Point::new(
        bounds.x0 + bounds.width() * f64::from(anchor.x),
        bounds.y0 + bounds.height() * f64::from(anchor.y),
    )
}

pub(crate) fn resolved_color_to_rgba8(color: ResolvedColor) -> [u8; 4] {
    let srgb = color.to_srgb_with_headroom();
    [
        (srgb.red.clamp(0.0, 1.0) * 255.0).round() as u8,
        (srgb.green.clamp(0.0, 1.0) * 255.0).round() as u8,
        (srgb.blue.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

pub(crate) fn rgba8_to_peniko(color: [u8; 4]) -> vello::peniko::Color {
    vello::peniko::Color::new([
        f32::from(color[0]) / 255.0,
        f32::from(color[1]) / 255.0,
        f32::from(color[2]) / 255.0,
        f32::from(color[3]) / 255.0,
    ])
}

pub(crate) fn parley_font_weight(weight: TextFontWeight) -> parley::FontWeight {
    let value = match weight {
        TextFontWeight::Thin => 100.0,
        TextFontWeight::UltraLight => 200.0,
        TextFontWeight::Light => 300.0,
        TextFontWeight::Normal => 400.0,
        TextFontWeight::Medium => 500.0,
        TextFontWeight::SemiBold => 600.0,
        TextFontWeight::Bold => 700.0,
        TextFontWeight::UltraBold => 800.0,
        TextFontWeight::Black => 900.0,
    };
    parley::FontWeight::new(value)
}

pub(crate) fn parley_alignment(alignment: HorizontalAlignment) -> parley::Alignment {
    if alignment == HorizontalAlignment::Leading {
        parley::Alignment::Start
    } else if alignment == HorizontalAlignment::Trailing {
        parley::Alignment::End
    } else {
        parley::Alignment::Center
    }
}

pub(crate) fn transformed_rect(
    transform: vello::kurbo::Affine,
    rect: vello::kurbo::Rect,
) -> vello::kurbo::Rect {
    let points = [
        transform * vello::kurbo::Point::new(rect.x0, rect.y0),
        transform * vello::kurbo::Point::new(rect.x1, rect.y0),
        transform * vello::kurbo::Point::new(rect.x0, rect.y1),
        transform * vello::kurbo::Point::new(rect.x1, rect.y1),
    ];
    let min_x = points
        .iter()
        .fold(f64::INFINITY, |acc, point| acc.min(point.x));
    let min_y = points
        .iter()
        .fold(f64::INFINITY, |acc, point| acc.min(point.y));
    let max_x = points
        .iter()
        .fold(f64::NEG_INFINITY, |acc, point| acc.max(point.x));
    let max_y = points
        .iter()
        .fold(f64::NEG_INFINITY, |acc, point| acc.max(point.y));
    vello::kurbo::Rect::new(min_x, min_y, max_x, max_y)
}

pub(crate) fn circle_arc_path(
    center: vello::kurbo::Point,
    radius: f64,
    start_angle: f64,
    sweep: f64,
) -> vello::kurbo::BezPath {
    let mut path = vello::kurbo::BezPath::new();
    if sweep == 0.0 {
        return path;
    }
    let segments = 64usize;
    let step = sweep / segments as f64;
    let mut angle = start_angle;
    path.move_to(vello::kurbo::Point::new(
        center.x + radius * angle.cos(),
        center.y + radius * angle.sin(),
    ));
    for _ in 0..segments {
        angle += step;
        path.line_to(vello::kurbo::Point::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        ));
    }
    path
}

fn text_cursor_area_from_layout(
    text_bounds: vello::kurbo::Rect,
    layout: &parley::Layout<[u8; 4]>,
    max_lines: Option<usize>,
    fallback_line_height: f64,
) -> vello::kurbo::Rect {
    let left = text_bounds.x0;
    let right = text_bounds.x1.max(left + 1.0);
    let top = text_bounds.y0;
    let bottom = text_bounds.y1.max(top + 1.0);
    if layout.is_empty() {
        let available_height = bottom - top;
        let line_height = fallback_line_height.clamp(1.0, available_height);
        let y0 = top + ((available_height - line_height) * 0.5).max(0.0);
        return vello::kurbo::Rect::new(left, y0, left + 1.0, (y0 + line_height).min(bottom));
    }

    let mut caret_x = left;
    let mut caret_top = top;
    let mut caret_bottom = (top + 1.0).min(bottom);
    for (index, line) in layout.lines().enumerate() {
        if max_lines.is_some_and(|limit| index >= limit) {
            break;
        }
        let metrics = line.metrics();
        caret_x = left + f64::from(metrics.offset + metrics.advance);
        let line_top = f64::from(metrics.baseline - metrics.ascent);
        let line_bottom = f64::from(metrics.baseline + metrics.descent);
        caret_top = top + line_top;
        caret_bottom = top + line_bottom.max(line_top + 1.0);
    }
    let caret_x = caret_x.clamp(left, right - 1.0);
    let caret_top = caret_top.clamp(top, bottom - 1.0);
    let caret_bottom = caret_bottom.clamp(caret_top + 1.0, bottom);
    vello::kurbo::Rect::new(caret_x, caret_top, caret_x + 1.0, caret_bottom)
}
