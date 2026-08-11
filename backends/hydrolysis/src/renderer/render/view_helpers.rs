use super::*;
use waterui_core::Computed;
use waterui_core::metadata::MetadataKey;

pub(crate) fn gesture_group_identity(view: &AnyView) -> usize {
    gesture_group_identity_with_budget(view, 64)
}

pub(crate) fn flatten_environment_metadata_ref<'a>(
    mut view: &'a AnyView,
    env: &Environment,
) -> (&'a AnyView, Environment) {
    let mut scoped_env = env.clone();
    while let Some(metadata) = view.downcast_ref::<Metadata<Environment>>() {
        scoped_env = metadata.value.clone();
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
        scoped_env = value.clone();
        view = content;
    }
    (view, scoped_env)
}

/// Extends `env` with an accessibility metadata value exactly like the retained
/// build's `Env`-scoping arm does. Layout normalization pre-resolves composite
/// bodies (`view.body(env)`), and a body may snapshot its environment into a
/// `Metadata<Environment>` override (the `With`-style env wrappers behind
/// `.foreground()` and friends), so normalization must resolve content under
/// the same scoped environment the build-time `Env` node installs — otherwise
/// the snapshot, which replaces the environment wholesale when flattened,
/// silently drops the accessibility scoping above it.
pub(crate) fn a11y_scoped_env<T: MetadataKey + Clone + 'static>(
    env: &Environment,
    value: &T,
) -> Environment {
    let mut scoped = env.clone();
    scoped.insert(value.clone());
    scoped
}

/// The `Env` scoping for a static [`AccessibilityState`]: a hidden state
/// suppresses the whole subtree's emission (a constant can never un-hide), and
/// the state is stored as a constant [`AccessibilityStateSignal`] so emission
/// resolves one code path for static and reactive state alike.
pub(crate) fn a11y_scoped_env_for_state(
    env: &Environment,
    value: &AccessibilityState,
) -> Environment {
    let mut scoped = env.clone();
    if value.is_hidden() {
        scoped.insert(AccessibilityHidden::new(true));
    }
    scoped.insert(AccessibilityStateSignal::new(Computed::constant(
        value.clone(),
    )));
    scoped
}

fn gesture_group_identity_with_budget(view: &AnyView, remaining: usize) -> usize {
    assert!(
        (remaining != 0),
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

pub(crate) fn passthrough_content(view: &AnyView) -> Option<&AnyView> {
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
        Background,
        NavigationTransitionSource,
        NavigationTransitionDestination
    );
    passthrough_ignorable_metadata_content!(
        MaterialBackground,
        AccessibilityIdentifier,
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
    if view.downcast_ref::<Divider>().is_some() {
        return StretchAxis::CrossAxis;
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
        (remaining != 0),
        "hydrolysis layout normalization exceeded recursion budget for {}",
        view.name()
    );
    let next_remaining = remaining - 1;
    let mut view = view;

    if view.is::<Metadata<Environment>>() {
        let Metadata { content, value } = *view
            .downcast::<Metadata<Environment>>()
            .expect("layout normalization failed to downcast Metadata<Environment>");
        let normalized_content = normalize_layout_view_with_budget(content, &value, remaining);
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
                        normalize_layout_view_with_budget(content, env, remaining);
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
                        normalize_layout_view_with_budget(content, env, remaining);
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
        Background,
        NavigationTransitionSource,
        NavigationTransitionDestination
    );
    // Label/role/children stay plain passthrough here: they are
    // nearest-consumer metadata — the leaf that emits the semantic node reads
    // them from the build-time `Env` scope, and they must not be baked into the
    // environments that composite bodies snapshot below (a group label would
    // then relabel every descendant leaf).
    normalize_passthrough_ignorable_metadata!(
        MaterialBackground,
        AccessibilityIdentifier,
        AccessibilityLabel,
        AccessibilityRole,
        AccessibilityChildren
    );

    // Hidden and (reactive) state have *subtree* semantics: every emission
    // below resolves them from its environment. Their content must therefore be
    // normalized under the same scoped environment the build arm installs — a
    // composite body resolved below may snapshot its environment into a
    // `Metadata<Environment>` override, and a snapshot taken without the
    // scoping would drop the hidden/state scope wholesale at flush (an
    // `a11y_state_signal(hidden)` overlay stayed visible exactly this way).
    macro_rules! normalize_env_scoping_ignorable_metadata {
        ($($ty:ty, $scope:expr);+ $(;)?) => {
            $(
                if view.is::<IgnorableMetadata<$ty>>() {
                    let IgnorableMetadata { content, value } = *view
                        .downcast::<IgnorableMetadata<$ty>>()
                        .expect("layout normalization failed to downcast ignorable metadata");
                    let scoped = $scope(env, &value);
                    let normalized_content =
                        normalize_layout_view_with_budget(content, &scoped, remaining);
                    return AnyView::new(IgnorableMetadata {
                        content: normalized_content,
                        value,
                    });
                }
            )+
        };
    }
    normalize_env_scoping_ignorable_metadata!(
        AccessibilityHidden, a11y_scoped_env;
        AccessibilityStateSignal, a11y_scoped_env;
        AccessibilityState, a11y_scoped_env_for_state
    );

    if view.is::<Native<FixedContainer>>() {
        let native = *view
            .downcast::<Native<FixedContainer>>()
            .expect("layout normalization failed to downcast Native<FixedContainer>");
        let (layout, children) = native.into_inner().into_inner();
        let mut normalized_children = Vec::with_capacity(children.len());
        for child in children {
            normalized_children.push(normalize_layout_view_with_budget(child, env, remaining));
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
        let (axis, content, controller) = native.into_inner().into_inner();
        let normalized_content = normalize_layout_view_with_budget(content, env, remaining);
        let scroll = ScrollView::new(axis, normalized_content);
        let scroll = match controller {
            Some(controller) => scroll.scroll_controller(&controller),
            None => scroll,
        };
        return AnyView::new(Native::new(scroll));
    }

    if view.is::<Native<NavigationView>>() {
        let native = *view
            .downcast::<Native<NavigationView>>()
            .expect("layout normalization failed to downcast Native<NavigationView>");
        let mut navigation = native.into_inner();
        navigation.bar.title =
            normalize_layout_view_with_budget(navigation.bar.title, env, remaining);
        navigation.bar.subtitle =
            normalize_layout_view_with_budget(navigation.bar.subtitle, env, remaining);
        for item in &mut navigation.bar.toolbar.items {
            item.content = normalize_layout_view_with_budget(
                core::mem::take(&mut item.content),
                env,
                remaining,
            );
        }
        navigation.content = normalize_layout_view_with_budget(navigation.content, env, remaining);
        return AnyView::new(Native::new(navigation));
    }

    if is_layout_terminal(&view) {
        return view;
    }

    view = AnyView::new(view.body(env));
    normalize_layout_view_with_budget(view, env, next_remaining)
}

pub(crate) fn estimate_layout_intrinsic<'a>(
    layout: &dyn Layout,
    children: impl IntoIterator<Item = &'a AnyView>,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let state = RefCell::new(state);
    let children: Vec<&AnyView> = children.into_iter().collect();
    let mut subviews = Vec::new();
    for child in children {
        subviews.push(HydroSubview::from_view(child, &state, env));
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

/// Resolves a shape into a concrete path for `bounds`.
///
/// Structured shape kinds resolve their normalized corner radii against the
/// shorter side, producing circular corners — the same interpretation the
/// morph path and the Apple backend use. Scaling the unit-space path
/// non-uniformly instead (the `CustomPath` fallback) stretches corner arcs
/// into ellipse segments on wide containers, which violates the Material
/// corner shape (e.g. a 4dp snackbar radius smeared across a 1500px bar).
pub(crate) fn resolved_shape_to_path(
    shape: &ResolvedShape,
    bounds: vello::kurbo::Rect,
) -> vello::kurbo::BezPath {
    shape_kind_path(shape.kind, bounds)
        .unwrap_or_else(|| path_commands_to_path(&shape.commands, bounds))
}

/// Bounds-aware path for the structured shape kinds; `None` for custom paths,
/// which only exist as unit-space commands.
pub(crate) fn shape_kind_path(
    kind: ShapeKind,
    bounds: vello::kurbo::Rect,
) -> Option<vello::kurbo::BezPath> {
    use vello::kurbo::Shape as _;
    const PATH_TOLERANCE: f64 = 0.05;
    match kind {
        ShapeKind::Rect
        | ShapeKind::RoundedRect { .. }
        | ShapeKind::UnevenRoundedRect { .. }
        | ShapeKind::Capsule => Some(rounded_rect_path(bounds, shape_kind_radii(kind))),
        ShapeKind::Circle => {
            let radius = bounds.width().min(bounds.height()).max(0.0) / 2.0;
            Some(vello::kurbo::Circle::new(bounds.center(), radius).into_path(PATH_TOLERANCE))
        }
        ShapeKind::Ellipse => {
            Some(vello::kurbo::Ellipse::from_rect(bounds).into_path(PATH_TOLERANCE))
        }
        ShapeKind::CustomPath => None,
    }
}

pub(crate) fn resolved_morph_shape_to_path(
    shape: &ResolvedMorphShape,
    progress: f32,
    bounds: vello::kurbo::Rect,
) -> vello::kurbo::BezPath {
    let from = shape_kind_radii(shape.from);
    let to = shape_kind_radii(shape.to);
    let progress = progress.clamp(0.0, 1.0);
    let radii = [
        lerp(from[0], to[0], progress),
        lerp(from[1], to[1], progress),
        lerp(from[2], to[2], progress),
        lerp(from[3], to[3], progress),
    ];
    rounded_rect_path(bounds, radii)
}

fn shape_kind_radii(kind: ShapeKind) -> [f32; 4] {
    match kind {
        ShapeKind::Rect => [0.0; 4],
        ShapeKind::Circle | ShapeKind::Ellipse | ShapeKind::Capsule => [0.5; 4],
        ShapeKind::RoundedRect { corner_radius } => [corner_radius.clamp(0.0, 0.5); 4],
        ShapeKind::UnevenRoundedRect {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        } => [
            top_left.clamp(0.0, 0.5),
            top_right.clamp(0.0, 0.5),
            bottom_right.clamp(0.0, 0.5),
            bottom_left.clamp(0.0, 0.5),
        ],
        ShapeKind::CustomPath => {
            panic!("hydrolysis morph shape rendering requires built-in shape kinds")
        }
    }
}

fn rounded_rect_path(bounds: vello::kurbo::Rect, radii: [f32; 4]) -> vello::kurbo::BezPath {
    const KAPPA: f64 = 0.552_284_749_830_793_6;
    let min_side = bounds.width().min(bounds.height()).max(0.0);
    let [tl, tr, br, bl] = radii.map(|radius| f64::from(radius.clamp(0.0, 0.5)) * min_side);
    let mut path = vello::kurbo::BezPath::new();

    path.move_to((bounds.x0 + tl, bounds.y0));
    path.line_to((bounds.x1 - tr, bounds.y0));
    append_corner(
        &mut path,
        vello::kurbo::Point::new(bounds.x1 - tr, bounds.y0 + tr),
        tr,
        -core::f64::consts::FRAC_PI_2,
        0.0,
        KAPPA,
    );
    path.line_to((bounds.x1, bounds.y1 - br));
    append_corner(
        &mut path,
        vello::kurbo::Point::new(bounds.x1 - br, bounds.y1 - br),
        br,
        0.0,
        core::f64::consts::FRAC_PI_2,
        KAPPA,
    );
    path.line_to((bounds.x0 + bl, bounds.y1));
    append_corner(
        &mut path,
        vello::kurbo::Point::new(bounds.x0 + bl, bounds.y1 - bl),
        bl,
        core::f64::consts::FRAC_PI_2,
        core::f64::consts::PI,
        KAPPA,
    );
    path.line_to((bounds.x0, bounds.y0 + tl));
    append_corner(
        &mut path,
        vello::kurbo::Point::new(bounds.x0 + tl, bounds.y0 + tl),
        tl,
        core::f64::consts::PI,
        core::f64::consts::PI + core::f64::consts::FRAC_PI_2,
        KAPPA,
    );
    path.close_path();
    path
}

fn append_corner(
    path: &mut vello::kurbo::BezPath,
    center: vello::kurbo::Point,
    radius: f64,
    start: f64,
    end: f64,
    kappa: f64,
) {
    if radius <= 0.0 {
        return;
    }
    let start_point = vello::kurbo::Point::new(
        center.x + radius * start.cos(),
        center.y + radius * start.sin(),
    );
    let end_point =
        vello::kurbo::Point::new(center.x + radius * end.cos(), center.y + radius * end.sin());
    let c1 = vello::kurbo::Point::new(
        start_point.x - radius * kappa * start.sin(),
        start_point.y + radius * kappa * start.cos(),
    );
    let c2 = vello::kurbo::Point::new(
        end_point.x + radius * kappa * end.sin(),
        end_point.y - radius * kappa * end.cos(),
    );
    path.curve_to(c1, c2, end_point);
}

fn lerp(from: f32, to: f32, progress: f32) -> f32 {
    from + (to - from) * progress
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
                    has_current,
                    "PathCommand::LineTo requires an active current point"
                );
                path.line_to(vello::kurbo::Point::new(
                    f64::from(*x) * width,
                    f64::from(*y) * height,
                ));
            }
            PathCommand::QuadTo { cx, cy, x, y } => {
                assert!(
                    has_current,
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
                    has_current,
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
