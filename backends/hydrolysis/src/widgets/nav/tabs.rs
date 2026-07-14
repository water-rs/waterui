use std::cell::RefCell;
use std::rc::Rc;

#[cfg(feature = "accessibility")]
use crate::renderer::{AccessibilityActionTarget, RenderContext};
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RetainedSubview, WidgetRenderContext,
    measure_tabs_intrinsic, measure_view_intrinsic, tabs_bar_and_content_rect, tabs_button_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use nami::Binding;
use waterui::navigation::tab::{TabPosition, Tabs};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::Id;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};

use crate::widgets::widget_theme;

/// The retained render state of one tab. Its `label` is a move-only `AnyView`, so
/// it is held as a [`RetainedSubview`] built once and re-flushed each frame; its
/// `content` is a clonable `Rc`-backed builder rebuilt fresh each frame; `tag`
/// drives selection.
struct TabRenderState {
    tag: Id,
    label: RetainedSubview,
    content: AnyViewBuilder<waterui::navigation::NavigationView>,
}

/// The retained render state of a `Tabs` container. The selection `Binding` and tab
/// `position` are kept by value; each tab is a [`TabRenderState`].
pub(crate) struct TabsRenderState {
    selection: Binding<Id>,
    position: TabPosition,
    tabs: Vec<TabRenderState>,
    /// The currently-shown tab content as a retained node sub-view, paired with the
    /// tab index it was built for. Rebuilt only when the selection changes (so the
    /// active tab's content keeps its state while selected); re-flushed each frame.
    selected_content: Option<(usize, RetainedSubview)>,
}

impl TabsRenderState {
    pub(crate) fn from_tabs(tabs: Tabs) -> Self {
        assert!(
            !(tabs.tabs.is_empty()),
            "hydrolysis Tabs requires at least one tab"
        );
        // `Tabs` is `#[non_exhaustive]`, so access fields rather than destructuring.
        let selection = tabs.selection;
        let position = tabs.position;
        let tabs = tabs
            .tabs
            .into_iter()
            .map(|tab| TabRenderState {
                tag: tab.label.tag,
                label: RetainedSubview::new(tab.label.content),
                content: tab.content,
            })
            .collect();
        Self {
            selection,
            position,
            tabs,
            selected_content: None,
        }
    }

    /// Eagerly build the tab-label sub-views (the measure path has no renderer to
    /// build on).
    pub(crate) fn prebuild_labels(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        for tab in &mut self.tabs {
            tab.label.ensure_built(renderer, env);
        }
    }

    fn selected_index(&self, selected_id: Id) -> usize {
        self.tabs
            .iter()
            .position(|tab| tab.tag == selected_id)
            .unwrap_or_else(|| panic!("hydrolysis Tabs selection is not present in tabs"))
    }
}

impl HydroNativeView for Native<Tabs> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_tabs_intrinsic(view.as_inner(), state, env)
    }
}

/// Emits a tab list's accessibility tree from per-tab `(tag, default_label,
/// is_selected)` triples. Shared by the dispatch path and the retained `Widget`-node
/// path (which extracts each default label from its tab's [`RetainedSubview`]).
#[cfg(feature = "accessibility")]
pub(crate) fn tabs_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    selection: &Binding<Id>,
    position: TabPosition,
    labels: &[(Id, Option<String>, bool)],
    env: &Environment,
) {
    let metrics = widget_theme(env).tabs_metrics();
    let (bar_rect, _content_rect) =
        tabs_bar_and_content_rect(ctx.bounds, position, metrics.bar_height);
    let mut tab_list = AccessibilityNode::new(
        renderer.resolve_accessibility_role(env, AccessibilityNodeRole::TabList),
    );
    let tab_list_label = renderer.resolve_accessibility_label(env, None);
    if let Some(label) = tab_list_label {
        tab_list.set_label(label);
    }
    for (index, (tag, default_label, is_selected)) in labels.iter().enumerate() {
        let mut tab_node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Tab),
        );
        let label = renderer.resolve_accessibility_label(env, default_label.clone());
        if let Some(label) = label {
            tab_node.set_label(label);
        }
        tab_node.set_selected(*is_selected);
        tab_node.add_action(AccessibilityAction::Focus);
        tab_node.add_action(AccessibilityAction::Click);
        let tab_bounds = crate::renderer::transformed_rect(
            ctx.hit_transform,
            tabs_button_rect(bar_rect, labels.len(), index),
        );
        if let Some(tab_node_id) = renderer.register_accessibility_child_node(
            tab_node,
            tab_bounds,
            env,
            Some(AccessibilityActionTarget::PickerSelect {
                selection: selection.clone(),
                target: *tag,
            }),
        ) {
            tab_list.push_child(tab_node_id);
        }
    }
    let _ = renderer.register_accessibility_node(
        tab_list,
        crate::renderer::transformed_rect(ctx.hit_transform, bar_rect),
        env,
        None,
    );
}

/// Measures a retained tabs leaf from its [`TabsRenderState`] (intrinsic-sized,
/// mirroring `measure_tabs_intrinsic`).
pub(crate) fn measure_tabs_node(
    state: &TabsRenderState,
    _proposal: ProposalSize,
    hydro: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    let metrics = widget_theme(env).tabs_metrics();
    let mut max_content_width: f64 = 0.0;
    let mut max_content_height: f64 = 0.0;
    let mut bar_width = 0.0;
    for tab in &state.tabs {
        let label_size = tab.label.measure_built(hydro, env);
        bar_width += (f64::from(label_size.width) + metrics.button_horizontal_inset * 2.0)
            .max(metrics.button_min_width);

        let content =
            crate::renderer::normalize_layout_view(AnyView::new(tab.content.build()), env);
        let content_size = measure_view_intrinsic(&content, hydro, env);
        max_content_width = max_content_width.max(f64::from(content_size.width));
        max_content_height = max_content_height.max(f64::from(content_size.height));
    }
    let width = max_content_width.max(bar_width);
    let height = max_content_height + metrics.bar_height;
    ViewDimensions::new(LayoutSize::new(width as f32, height as f32))
}

/// Renders a retained tabs leaf every flush: emits the tab-list a11y (unless
/// hidden) then the bar + selected content, reading the selection signal live.
pub(crate) fn render_tabs_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<TabsRenderState>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let hidden = env
            .get::<waterui::accessibility::AccessibilityHidden>()
            .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
        if !hidden {
            let selected_id = ctx.renderer_mut().read_signal(&state.borrow().selection);
            let (selection, position, labels) = {
                let st = state.borrow();
                let selected_index = st.selected_index(selected_id);
                let labels: Vec<(Id, Option<String>, bool)> = st
                    .tabs
                    .iter()
                    .enumerate()
                    .map(|(index, tab)| {
                        (
                            tab.tag,
                            tab.label.default_a11y_label(),
                            index == selected_index,
                        )
                    })
                    .collect();
                (st.selection.clone(), st.position, labels)
            };
            let render_ctx = ctx.render_context();
            tabs_accessibility(
                ctx.renderer_mut(),
                render_ctx,
                &selection,
                position,
                &labels,
                env,
            );
        }
    }
    render_tabs_parts(ctx, state, env);
}

pub(crate) fn render_tabs_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<TabsRenderState>>,
    env: &Environment,
) {
    let (selection, position, tab_count) = {
        let st = state.borrow();
        assert!(
            !st.tabs.is_empty(),
            "hydrolysis Tabs requires at least one tab"
        );
        (st.selection.clone(), st.position, st.tabs.len())
    };
    let selected_id = ctx.renderer_mut().read_signal(&selection);
    let selected_index = state.borrow().selected_index(selected_id);

    let theme_metrics = widget_theme(env).tabs_metrics();
    let (bar_rect, content_rect) =
        tabs_bar_and_content_rect(ctx.bounds, position, theme_metrics.bar_height);

    {
        let theme = widget_theme(env);
        let mut draw = ctx.draw_context();
        theme.draw_tabs_bar(&mut draw, bar_rect, matches!(position, TabPosition::Top));
    }

    for index in 0..tab_count {
        let button_rect = tabs_button_rect(bar_rect, tab_count, index);
        // The label sub-view is prebuilt (node path: `prebuild_labels`; dispatch
        // path: `render` calls `prebuild_labels`), so measure it directly for
        // placement.
        let label_size = {
            let cell = state.borrow();
            cell.tabs[index].label.measure_built(ctx.state_mut(), env)
        };
        {
            let hit_bounds = crate::renderer::transformed_rect(ctx.hit_transform, button_rect);
            let (interaction, press_slot, _) =
                ctx.renderer_mut().bind_interaction_target(hit_bounds, env);
            let interaction =
                crate::renderer::local_interaction_state(interaction, ctx.hit_transform);
            let is_selected = index == selected_index;
            {
                let theme = widget_theme(env);
                let mut draw = ctx.draw_context();
                if is_selected {
                    let highlight = tabs_active_indicator_rect(
                        button_rect,
                        position,
                        theme_metrics.active_indicator_height,
                        f64::from(label_size.width),
                    );
                    theme.draw_tabs_highlight(&mut draw, highlight);
                }
                theme.draw_tabs_button_state_layer(
                    &mut draw,
                    button_rect,
                    is_selected,
                    interaction,
                );
            }
            let selection_binding = selection.clone();
            let tab_id = state.borrow().tabs[index].tag;
            ctx.renderer_mut().register_interactive_pointer_target(
                hit_bounds,
                press_slot,
                move |_renderer, _point, _env| {
                    if selection_binding.get() != tab_id {
                        selection_binding.set(tab_id);
                    }
                    true
                },
            );
        }
        let label_rect = tabs_label_rect(button_rect, label_size, theme_metrics);
        if label_rect.width() > 0.0 && label_rect.height() > 0.0 {
            // The tab label's a11y is emitted by `tabs_accessibility`, so suppress
            // the sub-view's own a11y (matching the dispatch path's
            // `dispatch_in_rect_without_accessibility`).
            #[cfg(feature = "accessibility")]
            ctx.renderer_mut().push_accessibility_suppression();
            let render_ctx = ctx.render_context();
            state.borrow_mut().tabs[index].label.flush_in_rect(
                ctx.renderer_mut(),
                render_ctx,
                env,
                label_rect,
            );
            #[cfg(feature = "accessibility")]
            ctx.renderer_mut().pop_accessibility_suppression();
        }
    }

    if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
        let mut st = state.borrow_mut();
        // Rebuild the content sub-view only when the selected tab changes; otherwise
        // re-flush the retained node so the active tab keeps its state across frames.
        let needs_rebuild = st
            .selected_content
            .as_ref()
            .is_none_or(|(index, _)| *index != selected_index);
        if needs_rebuild {
            let content = AnyView::new(st.tabs[selected_index].content.build());
            st.selected_content = Some((selected_index, RetainedSubview::new(content)));
        }
        let render_ctx = ctx.render_context();
        let (_, subview) = st
            .selected_content
            .as_mut()
            .expect("tabs selected content was just ensured present");
        subview.flush_in_rect(ctx.renderer_mut(), render_ctx, env, content_rect);
    }
}

fn tabs_label_rect(
    button_rect: vello::kurbo::Rect,
    label_size: waterui_core::layout::Size,
    metrics: waterui_backend_core::widget::TabsMetrics,
) -> vello::kurbo::Rect {
    let max_width = (button_rect.width() - metrics.button_horizontal_inset * 2.0).max(0.0);
    let width = f64::from(label_size.width).min(max_width);
    let height = f64::from(label_size.height).min(button_rect.height());
    let x0 = button_rect.x0 + (button_rect.width() - width) * 0.5;
    let y0 = button_rect.y0 + (button_rect.height() - height) * 0.5;
    vello::kurbo::Rect::new(x0, y0, x0 + width, y0 + height)
}

fn tabs_active_indicator_rect(
    button_rect: vello::kurbo::Rect,
    position: TabPosition,
    height: f64,
    width: f64,
) -> vello::kurbo::Rect {
    let width = width.clamp(0.0, button_rect.width());
    let x0 = button_rect.x0 + (button_rect.width() - width) * 0.5;
    let x1 = x0 + width;
    match position {
        TabPosition::Top => vello::kurbo::Rect::new(
            x0,
            (button_rect.y1 - height).max(button_rect.y0),
            x1,
            button_rect.y1,
        ),
        TabPosition::Bottom => vello::kurbo::Rect::new(
            x0,
            button_rect.y0,
            x1,
            (button_rect.y0 + height).min(button_rect.y1),
        ),
    }
}
