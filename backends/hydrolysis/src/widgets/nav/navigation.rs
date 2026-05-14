use crate::engine::Brush;
#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
#[cfg(feature = "accessibility")]
use crate::renderer::accessibility_activation_point;
use crate::renderer::navigation_state::{HydroNavigationController, NavigationTransitionDirection};
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    measure_navigation_view_intrinsic, measure_view_intrinsic, navigation_back_button_rect,
    navigation_base_bar_height_for_display_mode, normalize_layout_view, resolved_color_to_peniko,
    split_compact_threshold, transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use nami::Signal;
use std::rc::Rc;
use waterui::navigation::{
    NavigationController, NavigationSplitLayout, NavigationStack, NavigationTransition,
    NavigationView,
};
use waterui_controls::text_field::TextField;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};

use crate::widgets::widget_theme;

#[derive(Clone, Copy)]
struct NavigationLeadingReserve(f64);

fn navigation_leading_reserve(env: &Environment) -> f64 {
    env.get::<NavigationLeadingReserve>()
        .map_or(0.0, |reserve| reserve.0)
}

fn back_button_title_reserve(env: &Environment) -> f64 {
    let metrics = widget_theme(env).navigation_metrics();
    metrics.back_button_size + metrics.title_leading_inset
}

impl HydroNativeView for Native<NavigationView> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_navigation_view(ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_navigation_view_intrinsic(view.as_inner(), state, env)
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        if let (Some(width), Some(height)) = (proposal.width, proposal.height) {
            return ViewDimensions::new(LayoutSize::new(width, height));
        }
        ViewDimensions::new(Self::intrinsic(state, view, env))
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let navigation = view.as_inner();
            let bar = &navigation.bar;
            let bar_hidden = renderer.read_signal(&bar.hidden);
            if bar_hidden {
                return;
            }
            let metrics = widget_theme(env).navigation_metrics();
            let bar_height = navigation_base_bar_height_for_display_mode(bar.display_mode, env);
            let bar_rect = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                (ctx.bounds.y0 + bar_height).min(ctx.bounds.y1),
            );
            let mut bar_node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Navigation),
            );
            let bar_label = renderer.resolve_accessibility_label(env, None);
            if let Some(label) = bar_label {
                bar_node.set_label(label);
            }
            let title_height = if matches!(
                bar.display_mode,
                waterui::navigation::NavigationTitleDisplayMode::Large
            ) {
                metrics.large_title_height
            } else {
                metrics.inline_title_height
            };
            let title_y0 = if matches!(
                bar.display_mode,
                waterui::navigation::NavigationTitleDisplayMode::Large
            ) {
                bar_rect.y1 - metrics.large_title_bottom_inset - title_height
            } else {
                bar_rect.y0 + (bar_height - title_height) * 0.5
            };
            let title_leading = navigation_leading_reserve(env);
            let title_rect = vello::kurbo::Rect::new(
                if title_leading > 0.0 {
                    bar_rect.x0 + metrics.horizontal_inset + title_leading
                } else {
                    bar_rect.x0 + metrics.title_leading_inset
                },
                title_y0,
                bar_rect.x1 - metrics.title_trailing_inset,
                title_y0 + title_height,
            );
            if title_rect.width() > 0.0 && title_rect.height() > 0.0 {
                let mut title_node = AccessibilityNode::new(
                    renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Header),
                );
                let default_title_label = renderer.accessibility_label_from_view(&bar.title, env);
                let title_label = renderer.resolve_accessibility_label(env, default_title_label);
                if let Some(label) = title_label {
                    title_node.set_label(label);
                }
                if let Some(title_node_id) = renderer.register_accessibility_child_node(
                    title_node,
                    transformed_rect(ctx.hit_transform, title_rect),
                    env,
                    None,
                ) {
                    bar_node.push_child(title_node_id);
                }
            }
            let _ = renderer.register_accessibility_node(
                bar_node,
                transformed_rect(ctx.hit_transform, bar_rect),
                env,
                None,
            );
        }
    }
}

impl HydroNativeView for Native<NavigationSplitLayout> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_navigation_split_layout(ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let split = view.as_inner();
        let sidebar = {
            let sidebar_view = normalize_layout_view(split.sidebar().build(), env);
            measure_view_intrinsic(&sidebar_view, state, env)
        };
        let detail = if let Some(selected) = split.selection().get() {
            measure_navigation_view_intrinsic(&split.detail_builder().build(selected), state, env)
        } else {
            let placeholder_view = normalize_layout_view(split.placeholder().build(), env);
            measure_view_intrinsic(&placeholder_view, state, env)
        };
        LayoutSize::new(
            (f64::from(split.sidebar_width()) + f64::from(detail.width)) as f32,
            f64::from(sidebar.height.max(detail.height)) as f32,
        )
    }

    fn accessibility(
        _renderer: &mut HydrolysisRenderer,
        _ctx: RenderContext,
        _view: &Self,
        _env: &Environment,
    ) {
    }
}

impl HydroNativeView for Native<NavigationStack<(), ()>> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_navigation_stack(ctx, view, env);
    }

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        _view: &Self,
        env: &Environment,
    ) {
        let entries = {
            let (slot_index, entries) = renderer.bind_navigation_entries();
            renderer.push_pending_navigation_entries(slot_index, Rc::clone(&entries));
            entries
        };
        let depth = entries.borrow().len();
        #[cfg(feature = "accessibility")]
        {
            if depth == 0 {
                return;
            }
            let mut back_node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Button),
            );
            back_node.set_label("Back".to_owned());
            back_node.add_action(AccessibilityAction::Focus);
            back_node.add_action(AccessibilityAction::Click);
            let back_bounds = transformed_rect(
                ctx.hit_transform,
                navigation_back_button_rect(ctx.bounds, widget_theme(env).navigation_metrics()),
            );
            let _ = renderer.register_accessibility_node(
                back_node,
                back_bounds,
                env,
                Some(AccessibilityActionTarget::PointerPrimaryClick {
                    point: accessibility_activation_point(back_bounds),
                }),
            );
        }
    }
}

pub(crate) fn render_navigation_view(
    ctx: &mut WidgetRenderContext<'_>,
    navigation: Native<NavigationView>,
    env: &Environment,
) {
    let navigation = navigation.into_inner();
    let NavigationView { bar, content } = navigation;
    let waterui::navigation::Bar {
        title,
        leading,
        trailing,
        search,
        color,
        hidden,
        display_mode,
    } = bar;
    let bar_height = if ctx.renderer_mut().read_signal(&hidden) {
        0.0
    } else {
        let metrics = widget_theme(env).navigation_metrics();
        let base = navigation_base_bar_height_for_display_mode(display_mode, env);
        let search_extra = if search.is_some() {
            metrics.search_height + metrics.search_vertical_inset * 2.0
        } else {
            0.0
        };
        base + search_extra
    };

    if bar_height > 0.0 {
        let metrics = widget_theme(env).navigation_metrics();
        let base_bar_height = navigation_base_bar_height_for_display_mode(display_mode, env);
        let bar_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x1,
            (ctx.bounds.y0 + bar_height).min(ctx.bounds.y1),
        );
        let bar_color = {
            let color = ctx.renderer_mut().read_signal(&color);
            resolved_color_to_peniko(color.resolve(env).get())
        };
        {
            let theme = widget_theme(env);
            let mut draw = ctx.draw_context();
            theme.draw_navigation_bar(&mut draw, bar_rect, &Brush::from(bar_color));
            let separator = vello::kurbo::Rect::new(
                bar_rect.x0,
                (bar_rect.y1 - 1.0).max(bar_rect.y0),
                bar_rect.x1,
                bar_rect.y1,
            );
            theme.draw_navigation_bar_separator(&mut draw, separator);
        }

        let leading_width = if !leading.is::<()>() {
            f64::from(crate::renderer::measure_view_intrinsic(&leading, ctx.state_mut(), env).width)
        } else {
            0.0
        };
        let trailing_width = if !trailing.is::<()>() {
            f64::from(
                crate::renderer::measure_view_intrinsic(&trailing, ctx.state_mut(), env).width,
            )
        } else {
            0.0
        };
        let leading_rect = vello::kurbo::Rect::new(
            bar_rect.x0 + metrics.horizontal_inset,
            bar_rect.y0 + (base_bar_height - metrics.inline_title_height) * 0.5,
            (bar_rect.x0 + metrics.horizontal_inset + leading_width).min(bar_rect.x1),
            bar_rect.y0 + (base_bar_height + metrics.inline_title_height) * 0.5,
        );
        let trailing_rect = vello::kurbo::Rect::new(
            (bar_rect.x1 - metrics.horizontal_inset - trailing_width).max(bar_rect.x0),
            bar_rect.y0 + (base_bar_height - metrics.inline_title_height) * 0.5,
            bar_rect.x1 - metrics.horizontal_inset,
            bar_rect.y0 + (base_bar_height + metrics.inline_title_height) * 0.5,
        );
        if !leading.is::<()>() && leading_rect.width() > 0.0 && leading_rect.height() > 0.0 {
            ctx.dispatch_in_rect(env, leading, leading_rect);
        }
        if !trailing.is::<()>() && trailing_rect.width() > 0.0 && trailing_rect.height() > 0.0 {
            ctx.dispatch_in_rect(env, trailing, trailing_rect);
        }

        let title_height = if matches!(
            display_mode,
            waterui::navigation::NavigationTitleDisplayMode::Large
        ) {
            metrics.large_title_height
        } else {
            metrics.inline_title_height
        };
        let title_y0 = if matches!(
            display_mode,
            waterui::navigation::NavigationTitleDisplayMode::Large
        ) {
            bar_rect.y0 + base_bar_height - metrics.large_title_bottom_inset - title_height
        } else {
            bar_rect.y0 + (base_bar_height - title_height) * 0.5
        };
        let effective_leading_width = leading_width.max(navigation_leading_reserve(env));
        let title_x0 = if effective_leading_width > 0.0 {
            bar_rect.x0 + metrics.horizontal_inset + effective_leading_width + metrics.item_spacing
        } else {
            bar_rect.x0 + metrics.title_leading_inset
        };
        let title_x1 = if trailing_width > 0.0 {
            bar_rect.x1 - metrics.horizontal_inset - trailing_width - metrics.item_spacing
        } else {
            bar_rect.x1 - metrics.title_trailing_inset
        };
        let title_rect = vello::kurbo::Rect::new(
            title_x0.min(bar_rect.x1),
            title_y0,
            title_x1.max(bar_rect.x0),
            title_y0 + title_height,
        );
        if title_rect.width() > 0.0 && title_rect.height() > 0.0 {
            ctx.dispatch_in_rect_without_accessibility(env, title, title_rect);
        }

        if let Some(search) = search.as_ref() {
            let search_rect = vello::kurbo::Rect::new(
                bar_rect.x0 + metrics.horizontal_inset,
                bar_rect.y0 + base_bar_height + metrics.search_vertical_inset,
                bar_rect.x1 - metrics.horizontal_inset,
                (bar_rect.y0
                    + base_bar_height
                    + metrics.search_vertical_inset
                    + metrics.search_height)
                    .min(bar_rect.y1 - metrics.search_vertical_inset),
            );
            if search_rect.width() > 0.0 && search_rect.height() > 0.0 {
                ctx.dispatch_in_rect(
                    env,
                    AnyView::new(TextField::new(&search.text).prompt(search.prompt.clone())),
                    search_rect,
                );
            }
        }
    }

    let content_rect = vello::kurbo::Rect::new(
        ctx.bounds.x0,
        (ctx.bounds.y0 + bar_height).min(ctx.bounds.y1),
        ctx.bounds.x1,
        ctx.bounds.y1,
    );
    if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
        ctx.dispatch_in_rect(env, content, content_rect);
    }
}

pub(crate) fn render_navigation_split_layout(
    ctx: &mut WidgetRenderContext<'_>,
    split: Native<NavigationSplitLayout>,
    env: &Environment,
) {
    let split = split.into_inner();
    let compact = ctx.bounds.width() < split_compact_threshold(f64::from(split.sidebar_width()));
    let selected = ctx.renderer_mut().read_signal(split.selection());

    if compact && let Some(selected) = selected {
        let detail = split.detail_builder().build(selected);
        let mut detail_env = env.clone();
        detail_env.insert(NavigationLeadingReserve(back_button_title_reserve(env)));
        ctx.dispatch_in_rect(&detail_env, AnyView::new(detail), ctx.bounds);
        let back_button_rect =
            navigation_back_button_rect(ctx.bounds, widget_theme(env).navigation_metrics());
        {
            let theme = widget_theme(env);
            let mut draw = ctx.draw_context();
            theme.draw_navigation_back_button(&mut draw, back_button_rect);
        }
        let selection = split.selection().clone();
        let hit_transform = ctx.hit_transform;
        ctx.renderer_mut().register_pointer_target(
            transformed_rect(hit_transform, back_button_rect),
            move |_renderer, _point, _| {
                selection.set(None);
                true
            },
        );
        return;
    }

    let sidebar_width = f64::from(split.sidebar_width()).min(ctx.bounds.width() * 0.5);
    let sidebar_rect = vello::kurbo::Rect::new(
        ctx.bounds.x0,
        ctx.bounds.y0,
        ctx.bounds.x0 + sidebar_width,
        ctx.bounds.y1,
    );
    let detail_rect =
        vello::kurbo::Rect::new(sidebar_rect.x1, ctx.bounds.y0, ctx.bounds.x1, ctx.bounds.y1);
    ctx.dispatch_in_rect(env, split.sidebar().build(), sidebar_rect);
    if let Some(selected) = selected {
        let detail = split.detail_builder().build(selected);
        ctx.dispatch_in_rect(env, AnyView::new(detail), detail_rect);
    } else {
        ctx.dispatch_in_rect(env, split.placeholder().build(), detail_rect);
    }
}

pub(crate) fn render_navigation_stack(
    ctx: &mut WidgetRenderContext<'_>,
    stack: Native<NavigationStack<(), ()>>,
    env: &Environment,
) {
    let stack = stack.into_inner();
    let transition_style = stack.transition_style();
    let transition_motion = widget_theme(env).navigation_motion();
    let root = stack.into_inner();
    let (slot_index, entries) = ctx
        .renderer_mut()
        .take_pending_navigation_entries("render_navigation_stack");

    let mut local_env = env.clone();
    local_env.insert(NavigationController::new(HydroNavigationController {
        entries: Rc::clone(&entries),
        rebuild_requested: Rc::clone(&ctx.renderer_mut().rebuild_requested),
    }));

    let (active, depth) = {
        let entries_ref = entries.borrow();
        let active = entries_ref
            .last()
            .map_or_else(|| root, |builder| AnyView::new(builder.build()));
        (active, entries_ref.len())
    };
    if depth > 0 {
        local_env.insert(NavigationLeadingReserve(back_button_title_reserve(env)));
    }
    let local_ctx = ctx.with_identity_transforms(ctx.bounds);
    let active_scene =
        HydrolysisRenderer::render_subtree_scene(ctx.renderer_mut(), local_ctx, &local_env, active);
    let now = ctx.renderer_mut().frame_instant();
    let transition_frame = {
        let slot = ctx
            .renderer_mut()
            .navigation
            .slots
            .get_mut(slot_index)
            .expect("hydrolysis navigation slot missing");
        if depth != slot.last_depth {
            if transition_style == NavigationTransition::None || slot.last_scene.is_none() {
                slot.transition = None;
            } else {
                let direction = if depth > slot.last_depth {
                    NavigationTransitionDirection::Push
                } else {
                    NavigationTransitionDirection::Pop
                };
                let from_scene = slot
                    .last_scene
                    .take()
                    .expect("hydrolysis navigation transition requires previous scene");
                slot.transition = Some(
                    crate::renderer::navigation_state::NavigationTransitionState::new(
                        transition_style,
                        direction,
                        from_scene,
                        active_scene.clone(),
                        now,
                        transition_motion.transition_duration,
                    ),
                );
            }
            slot.last_depth = depth;
        } else if let Some(transition) = slot.transition.as_ref()
            && !transition.is_active(now)
        {
            slot.transition = None;
        }
        let frame = slot.transition.as_ref().map(|transition| {
            (
                transition.style,
                transition.direction,
                transition.progress(now),
                transition_motion.pushpop_parallax_factor,
                transition.from_scene.clone(),
                transition.to_scene.clone(),
            )
        });
        slot.last_scene = Some(active_scene.clone());
        frame
    };

    if let Some((style, direction, progress, parallax_factor, from_scene, to_scene)) =
        transition_frame
    {
        ctx.draw_navigation_transition(
            style,
            direction,
            progress,
            parallax_factor,
            &from_scene,
            &to_scene,
        );
    } else {
        ctx.append_scene(&active_scene);
    }

    if depth == 0 {
        return;
    }

    let back_button_rect =
        navigation_back_button_rect(ctx.bounds, widget_theme(env).navigation_metrics());
    {
        let theme = widget_theme(env);
        let mut draw = ctx.draw_context();
        theme.draw_navigation_back_button(&mut draw, back_button_rect);
    }

    let entries_for_pop = Rc::clone(&entries);
    let rebuild_requested = Rc::clone(&ctx.renderer_mut().rebuild_requested);
    let hit_transform = ctx.hit_transform;
    ctx.renderer_mut().register_pointer_target(
        transformed_rect(hit_transform, back_button_rect),
        move |_renderer, _point, _env| {
            if entries_for_pop.borrow_mut().pop().is_some() {
                rebuild_requested.set(true);
                return true;
            }
            false
        },
    );
}
