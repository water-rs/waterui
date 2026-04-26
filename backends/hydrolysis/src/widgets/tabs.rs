#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, TABS_BUTTON_HORIZONTAL_INSET,
    WidgetRenderContext, tabs_bar_and_content_rect, tabs_button_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use waterui::navigation::tab::{TabPosition, Tabs};
use waterui_core::layout::Size as LayoutSize;
use waterui_core::{AnyView, Environment, Native};

use super::{inset_rect, widget_theme};

impl HydroNativeView for Native<Tabs> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_tabs(ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        crate::renderer::measure_tabs_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let tabs = view.as_inner();
            assert!(
                !(tabs.tabs.is_empty()),
                "hydrolysis Tabs requires at least one tab"
            );
            let selected_id = renderer.read_signal(&tabs.selection);
            let selected_index = tabs
                .tabs
                .iter()
                .position(|tab| tab.label.tag == selected_id)
                .unwrap_or_else(|| panic!("hydrolysis Tabs selection is not present in tabs"));
            let (bar_rect, _content_rect) = tabs_bar_and_content_rect(ctx.bounds, tabs.position);
            let mut tab_list = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::TabList),
            );
            let tab_list_label = renderer.resolve_accessibility_label(env, None);
            if let Some(label) = tab_list_label {
                tab_list.set_label(label);
            }
            for (index, tab) in tabs.tabs.iter().enumerate() {
                let mut tab_node = AccessibilityNode::new(
                    renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Tab),
                );
                let default_label = renderer.accessibility_label_from_view(&tab.label.content, env);
                let label = renderer.resolve_accessibility_label(env, default_label);
                if let Some(label) = label {
                    tab_node.set_label(label);
                }
                let is_selected = index == selected_index;
                tab_node.set_selected(is_selected);
                tab_node.add_action(AccessibilityAction::Focus);
                tab_node.add_action(AccessibilityAction::Click);
                let tab_bounds = crate::renderer::transformed_rect(
                    ctx.hit_transform,
                    tabs_button_rect(bar_rect, tabs.tabs.len(), index),
                );
                if let Some(tab_node_id) = renderer.register_accessibility_child_node(
                    tab_node,
                    tab_bounds,
                    env,
                    Some(AccessibilityActionTarget::PickerSelect {
                        selection: tabs.selection.clone(),
                        target: tab.label.tag,
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
    }
}

pub(crate) fn render_tabs(
    ctx: &mut WidgetRenderContext<'_>,
    tabs: Native<Tabs>,
    env: &Environment,
) {
    let tabs = tabs.into_inner();
    assert!(
        !(tabs.tabs.is_empty()),
        "hydrolysis Tabs requires at least one tab"
    );

    let tab_count = tabs.tabs.len();
    let position = tabs.position;
    let selection = tabs.selection;
    let selected_id = ctx.renderer_mut().read_signal(&selection);
    let selected_index = tabs
        .tabs
        .iter()
        .position(|tab| tab.label.tag == selected_id)
        .unwrap_or_else(|| panic!("hydrolysis Tabs selection is not present in tabs"));

    let (bar_rect, content_rect) = tabs_bar_and_content_rect(ctx.bounds, position);

    {
        let theme = widget_theme(env);
        let mut draw = ctx.draw_context();
        theme.draw_tabs_bar(&mut draw, bar_rect, matches!(position, TabPosition::Top));
    }

    let mut selected_content = None;
    for (index, tab) in tabs.tabs.into_iter().enumerate() {
        if index == selected_index {
            selected_content = Some(AnyView::new(tab.content.build()));
        }

        let button_rect = tabs_button_rect(bar_rect, tab_count, index);
        {
            if index == selected_index {
                let highlight = inset_rect(button_rect, 4.0, 6.0);
                let theme = widget_theme(env);
                let mut draw = ctx.draw_context();
                theme.draw_tabs_highlight(&mut draw, highlight);
            }
        }
        let label_rect = inset_rect(button_rect, TABS_BUTTON_HORIZONTAL_INSET, 8.0);
        let tab_id = tab.label.tag;
        if label_rect.width() > 0.0 && label_rect.height() > 0.0 {
            ctx.dispatch_in_rect_without_accessibility(env, tab.label.content, label_rect);
        }

        let selection_binding = selection.clone();
        let hit_transform = ctx.hit_transform;
        ctx.renderer_mut().register_pointer_target(
            crate::renderer::transformed_rect(hit_transform, button_rect),
            move |_renderer, _point, _env| {
                if selection_binding.get() != tab_id {
                    selection_binding.set(tab_id);
                }
                true
            },
        );
    }

    if let Some(content) = selected_content
        && content_rect.width() > 0.0
        && content_rect.height() > 0.0
    {
        ctx.dispatch_in_rect(env, content, content_rect);
    }
}
