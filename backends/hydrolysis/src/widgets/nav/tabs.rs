use crate::renderer::bounded_proposal;
use std::cell::RefCell;
use std::rc::Rc;

#[cfg(feature = "accessibility")]
use crate::renderer::{AccessibilityActionTarget, RenderContext};
use crate::renderer::{
    Edge, HydroNativeView, HydroState, RetainedSubview, WidgetRenderContext, measure_tabs_layout,
    tabs_bar_and_content_rect, tabs_button_rect, tabs_content_proposal,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use nami::{Binding, Signal};
use waterui::navigation::tab::{NativeTabStyle, TabIcon, TabsLayout};
use waterui_backend_core::widget::TabItemLayout;
use waterui_controls::label::LabelDisplayMode;
use waterui_core::id::Id;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};

#[cfg(feature = "accessibility")]
use crate::widgets::util::widget_disabled;

/// The retained render state of one tab. Its `label` is a move-only `AnyView`, so
/// it is held as a [`RetainedSubview`] built once and re-flushed each frame; its
/// `content` is a cloneable `Rc`-backed builder rebuilt fresh each frame; `tag`
/// drives selection.
struct TabRenderState {
    tag: Id,
    label: RetainedSubview,
    /// The label's icon, lifted out so the tab item can place it above the
    /// label (vertical) or beside it (horizontal).
    icon: Option<RetainedSubview>,
    content: RetainedSubview,
    enabled: nami::Computed<bool>,
}

/// The retained render state of a `TabsLayout` container. The selection `Binding` and tab
/// native style are kept by value; each tab is a [`TabRenderState`].
pub struct TabsRenderState {
    selection: Binding<Id>,
    style: NativeTabStyle,
    tabs: Vec<TabRenderState>,
}

impl TabsRenderState {
    pub(crate) fn from_tabs(tabs: TabsLayout) -> Self {
        assert!(
            !(tabs.tabs.is_empty()),
            "hydrolysis Tabs requires at least one tab"
        );
        // `TabsLayout` is `#[non_exhaustive]`, so access fields rather than destructuring.
        let selection = tabs.selection;
        let style = tabs.style;
        let tabs = tabs
            .tabs
            .into_iter()
            .map(|tab| TabRenderState {
                tag: tab.id,
                label: RetainedSubview::new(tab.label),
                icon: tab.icon.map(|icon| {
                    RetainedSubview::new(match icon {
                        TabIcon::System(icon) => AnyView::new(icon),
                        TabIcon::View(builder) => builder.build(),
                    })
                }),
                content: RetainedSubview::new(AnyView::new(tab.content.build())),
                enabled: tab.enabled,
            })
            .collect();
        Self {
            selection,
            style,
            tabs,
        }
    }

    /// Eagerly build the tab-label and icon sub-views (the measure path has
    /// no renderer to build on). The label builds title-only: the bar places
    /// the icon itself, so the label must not draw it a second time.
    pub(crate) fn prebuild_labels(
        &mut self,
        renderer: &mut crate::renderer::SemanticCore,
        env: &Environment,
    ) {
        let label_env = tab_label_env(env);
        for tab in &mut self.tabs {
            tab.label.ensure_built(renderer, &label_env);
            if let Some(icon) = &mut tab.icon {
                icon.ensure_built(renderer, env);
            }
            tab.content.ensure_built(renderer, env);
        }
    }

    fn selected_index(&self, selected_id: Id) -> usize {
        self.tabs
            .iter()
            .position(|tab| tab.tag == selected_id)
            .unwrap_or_else(|| panic!("hydrolysis Tabs selection is not present in tabs"))
    }
}

impl HydroNativeView for Native<TabsLayout> {
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_tabs_layout(
            view.as_inner(),
            ProposalSize::UNSPECIFIED,
            state,
            env,
            theme,
        )
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        ViewDimensions::new(measure_tabs_layout(
            view.as_inner(),
            proposal,
            state,
            env,
            theme,
        ))
    }
}

/// Emits a tab list's accessibility tree from per-tab `(tag, interaction_key,
/// default_label, is_selected)` tuples. Shared by the dispatch path and the
/// retained `Widget`-node path (which extracts each default label from its
/// tab's [`RetainedSubview`]).
#[cfg(feature = "accessibility")]
pub fn tabs_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    theme: Option<&Rc<dyn crate::engine::WidgetTheme>>,
    selection: &Binding<Id>,
    style: NativeTabStyle,
    labels: &[(Id, crate::renderer::InteractionKey, Option<String>, bool)],
    env: &Environment,
) {
    let disabled = renderer.read_signal(&widget_disabled(env));
    // Bar/button rects exist only in the rendered frame; the semantic walk
    // emits the same TabList/Tab structure with no bounds.
    let bar_rect = ctx.zip(theme).map(|(ctx, theme)| {
        let layout = theme.tabs_item_layout(
            tabs_bar_item_extent(ctx.bounds.width(), style, theme),
            labels.len(),
        );
        let metrics = theme.tabs_metrics(layout);
        tabs_bar_and_content_rect(ctx.bounds, style, metrics.bar_height).0
    });
    let mut tab_list =
        AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
            env,
            AccessibilityNodeRole::TabList,
        ));
    let tab_list_label = renderer.resolve_accessibility_label(env, None);
    if let Some(label) = tab_list_label {
        tab_list.set_label(label);
    }
    if let Some(value) = renderer.resolve_accessibility_value(env, None) {
        tab_list.set_value(value);
    }
    for (index, (tag, interaction_key, default_label, is_selected)) in labels.iter().enumerate() {
        let mut tab_node =
            AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::Tab,
            ));
        let label = renderer.resolve_accessibility_label(env, default_label.clone());
        if let Some(label) = label {
            tab_node.set_label(label);
        }
        tab_node.set_selected(*is_selected);
        tab_node.add_action(AccessibilityAction::Focus);
        if disabled {
            tab_node.set_disabled();
        } else {
            tab_node.add_action(AccessibilityAction::Click);
        }
        let key = i64::from(i32::from(*tag));
        let target = (!disabled).then(|| AccessibilityActionTarget::PickerSelect {
            selection: selection.clone(),
            target: *tag,
        });
        let tab_node_id = match ctx.zip(bar_rect) {
            Some((_ctx, bar_rect)) => renderer.register_accessibility_child_node_with_key(
                key,
                tab_node,
                tabs_button_rect(bar_rect, labels.len(), index, style),
                env,
                target,
            ),
            None => renderer
                .register_accessibility_child_node_with_key_semantic(key, tab_node, env, target),
        };
        if let Some(tab_node_id) = tab_node_id {
            tab_list.push_child(tab_node_id);
            renderer.register_accessibility_focus_link(interaction_key, tab_node_id);
        }
    }
    match ctx.zip(bar_rect) {
        Some((_ctx, bar_rect)) => {
            let _ = renderer.register_accessibility_node(tab_list, bar_rect, env, None);
        }
        None => {
            let _ = renderer.register_accessibility_node_semantic(tab_list, env, None);
        }
    }
}

/// Measures a retained tabs leaf from its [`TabsRenderState`]: each tab's
/// retained content answers the proposal its rendered content rect hands it —
/// the pane minus the tab bar (see [`tabs_content_proposal`]) — mirroring
/// `measure_tabs_layout`.
pub fn measure_tabs_node(
    state: &TabsRenderState,
    proposal: ProposalSize,
    hydro: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let label_env = tab_label_env(env);
    let item_sizes: Vec<(LayoutSize, Option<LayoutSize>)> = state
        .tabs
        .iter()
        .map(|tab| {
            let label_size = tab.label.measure_built(hydro, &label_env, theme);
            let icon_size = tab
                .icon
                .as_ref()
                .map(|icon| icon.measure_built(hydro, env, theme));
            (label_size, icon_size)
        })
        .collect();
    // Decide the layout once from the bar's own extent so the measured bar
    // and the drawn bar answer the same layout (see `tabs_decide_layout`).
    let (layout, metrics) = tabs_decide_layout(
        theme,
        state.style,
        proposal.width.map(f64::from),
        &item_sizes,
    );
    let content_proposal = tabs_content_proposal(proposal, state.style, metrics.bar_height);
    let mut max_content_width: f64 = 0.0;
    let mut max_content_height: f64 = 0.0;
    let mut bar_width = 0.0;
    for (tab, (label_size, icon_size)) in state.tabs.iter().zip(item_sizes.iter()) {
        bar_width += tabs_item_natural_width(*label_size, *icon_size, &metrics, layout);

        let content_size =
            tab.content
                .measure_built_with_proposal(hydro, env, theme, content_proposal);
        max_content_width = max_content_width.max(f64::from(content_size.width));
        max_content_height = max_content_height.max(f64::from(content_size.height));
    }
    let (width, height) = match state.style {
        NativeTabStyle::Automatic | NativeTabStyle::TabBar => (
            max_content_width.max(bar_width),
            max_content_height + metrics.bar_height,
        ),
        NativeTabStyle::Sidebar => (
            max_content_width + metrics.bar_height,
            max_content_height
                .max(metrics.button_min_width * crate::num_cast::usize_as_f64(state.tabs.len())),
        ),
    };
    ViewDimensions::new(LayoutSize::new(
        proposal
            .width
            .unwrap_or_else(|| crate::num_cast::f64_as_f32(width)),
        proposal
            .height
            .unwrap_or_else(|| crate::num_cast::f64_as_f32(height)),
    ))
}

/// Renders a retained tabs leaf every flush: emits the tab-list a11y (unless
/// hidden) then the bar + selected content, reading the selection signal live.
pub fn render_tabs_node(
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
            let (selection, style, labels) = {
                let st = state.borrow();
                let selected_index = st.selected_index(selected_id);
                let labels: Vec<(Id, crate::renderer::InteractionKey, Option<String>, bool)> = st
                    .tabs
                    .iter()
                    .enumerate()
                    .map(|(index, tab)| {
                        (
                            tab.tag,
                            crate::renderer::InteractionKey::for_rc(
                                state,
                                crate::num_cast::i32_as_u32(i32::from(tab.tag)) as usize,
                            ),
                            tab.label.default_a11y_label(),
                            index == selected_index,
                        )
                    })
                    .collect();
                (st.selection.clone(), st.style, labels)
            };
            let render_ctx = ctx.render_context();
            let theme = ctx.theme();
            tabs_accessibility(
                ctx.renderer_mut(),
                Some(render_ctx),
                Some(&theme),
                &selection,
                style,
                &labels,
                env,
            );
        }
    }
    render_tabs_parts(ctx, state, env);
}

#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn render_tabs_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<TabsRenderState>>,
    env: &Environment,
) {
    let (selection, style, tab_count) = {
        let st = state.borrow();
        assert!(
            !st.tabs.is_empty(),
            "hydrolysis Tabs requires at least one tab"
        );
        (st.selection.clone(), st.style, st.tabs.len())
    };
    let selected_id = ctx.renderer_mut().read_signal(&selection);
    let selected_index = state.borrow().selected_index(selected_id);

    let extent = {
        let theme = ctx.theme();
        tabs_bar_item_extent(ctx.bounds.width(), style, &theme)
    };
    let layout = ctx.theme().tabs_item_layout(extent, tab_count);
    let theme_metrics = ctx.theme().tabs_metrics(layout);
    let (bar_rect, content_rect) =
        tabs_bar_and_content_rect(ctx.bounds, style, theme_metrics.bar_height);
    let label_env = tab_label_env(env);

    // §7.1 "Chrome": the bar's surface extends through the regions of the
    // edges it touches to the window edge. A leading-docked strip keeps
    // only its top edge on its own frame — hydrolysis-m3 draws the
    // sidebar's divider at the surface's *top* edge under `top_edge:
    // false`, a boundary the strip touches, so extending the surface there
    // would move the divider under the status band; no `top_edge` argument
    // yields the vertical inner-edge rule a sidebar needs. Extending the
    // strip's top edge and placing its divider is a `WidgetTheme` contract
    // change tracked as a follow-up issue.
    let bar_surface = match style {
        NativeTabStyle::Sidebar => {
            ctx.chrome_surface_except(bar_rect, &[Edge::Top, Edge::Trailing])
        }
        NativeTabStyle::Automatic | NativeTabStyle::TabBar => {
            ctx.chrome_surface(bar_rect, Edge::Bottom)
        }
    };
    {
        let theme = ctx.theme();
        ctx.draw_context(|draw| {
            theme.draw_tabs_bar(&mut *draw, bar_surface, false);
        });
    }

    for index in 0..tab_count {
        let button_rect = tabs_button_rect(bar_rect, tab_count, index, style);
        let tab_id = state.borrow().tabs[index].tag;
        let interaction_key = crate::renderer::InteractionKey::for_rc(
            state,
            crate::num_cast::i32_as_u32(i32::from(tab_id)) as usize,
        );
        // The label and icon sub-views are prebuilt (node path:
        // `prebuild_labels`; dispatch path: `render` calls `prebuild_labels`),
        // so measure them directly for placement.
        let (label_size, icon_size) = {
            let cell = state.borrow();
            let theme = ctx.theme();
            let label_size =
                cell.tabs[index]
                    .label
                    .measure_built(ctx.state_mut(), &label_env, &theme);
            let icon_size = cell.tabs[index]
                .icon
                .as_ref()
                .map(|icon| icon.measure_built(ctx.state_mut(), env, &theme));
            (label_size, icon_size)
        };
        let (icon_rect, label_rect) =
            tabs_item_content_rects(button_rect, icon_size, label_size, &theme_metrics, layout);
        {
            let hit_transform = ctx.renderer_mut().current_hit_transform();
            let hit_bounds = button_rect;
            let (interaction, press_slot, _) =
                ctx.renderer_mut()
                    .bind_interaction_target(interaction_key, hit_bounds, env);
            let interaction = crate::renderer::local_interaction_state(interaction, hit_transform);
            let is_selected = index == selected_index;
            // A horizontal item's indicator and state layer hug the icon+label
            // content grown by the button inset, not the whole button share;
            // vertical items keep the button-wide bounds. The hit target stays
            // the full button rect either way.
            let chrome_bounds = match layout {
                TabItemLayout::Horizontal => {
                    let content_x0 =
                        icon_rect.map_or(label_rect.x0, |rect| rect.x0.min(label_rect.x0));
                    let content_x1 =
                        icon_rect.map_or(label_rect.x1, |rect| rect.x1.max(label_rect.x1));
                    let inset = theme_metrics.button_horizontal_inset;
                    kurbo::Rect::new(
                        (content_x0 - inset).max(button_rect.x0),
                        button_rect.y0,
                        (content_x1 + inset).min(button_rect.x1),
                        button_rect.y1,
                    )
                }
                TabItemLayout::Vertical => button_rect,
            };
            {
                let theme = ctx.theme();
                ctx.draw_context(|draw| {
                    if is_selected {
                        let highlight = tabs_highlight_rect(
                            chrome_bounds,
                            style,
                            theme_metrics.active_indicator_height,
                            if matches!(style, NativeTabStyle::Sidebar) {
                                f64::from(label_size.height)
                            } else {
                                f64::from(label_size.width)
                            },
                            layout,
                        );
                        theme.draw_tabs_highlight(&mut *draw, highlight, layout);
                    }
                    theme.draw_tabs_button_state_layer(
                        &mut *draw,
                        chrome_bounds,
                        is_selected,
                        interaction,
                        layout,
                    );
                });
            }
            let selection_binding = selection.clone();
            let enabled = {
                let st = state.borrow();
                ctx.renderer_mut().read_signal(&st.tabs[index].enabled)
            };
            if enabled {
                ctx.renderer_mut().register_interactive_pointer_target(
                    hit_bounds,
                    press_slot,
                    move |_renderer, _point, _env| {
                        if selection_binding.snapshot() != tab_id {
                            selection_binding.set(tab_id);
                        }
                        true
                    },
                );
            }
        }
        let has_label = label_rect.width() > 0.0 && label_rect.height() > 0.0;
        if icon_rect.is_some() || has_label {
            // The tab item's a11y is emitted by `tabs_accessibility`, so suppress
            // the sub-views' own a11y (matching the dispatch path's
            // `dispatch_in_rect_without_accessibility`).
            #[cfg(feature = "accessibility")]
            ctx.renderer_mut().push_accessibility_suppression();
            // A tab gets an equal share of the bar and no more. Without this a
            // long label drew straight over its neighbour and off the edge of
            // the bar, since the label lays out at its natural width.
            ctx.with_scope(
                crate::renderer::mount::ScopeKey {
                    role: "tab",
                    item: index as u64,
                },
                1.0,
                button_rect,
                |ctx| {
                    let render_ctx = ctx.render_context();
                    let mut st = state.borrow_mut();
                    // The icon draws whether or not the label has text to show.
                    if let (Some(icon), Some(icon_rect)) = (&mut st.tabs[index].icon, icon_rect) {
                        let icon_area = ctx.safe_area_for(icon_rect);
                        icon.place(
                            ctx.renderer_mut(),
                            render_ctx,
                            env,
                            ProposalSize::UNSPECIFIED,
                            icon_rect,
                            icon_area,
                        );
                    }
                    if has_label {
                        let label_area = ctx.safe_area_for(label_rect);
                        st.tabs[index].label.place(
                            ctx.renderer_mut(),
                            render_ctx,
                            &label_env,
                            ProposalSize::UNSPECIFIED,
                            label_rect,
                            label_area,
                        );
                    }
                    drop(st);
                },
            );
            #[cfg(feature = "accessibility")]
            ctx.renderer_mut().pop_accessibility_suppression();
        }
    }

    if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
        let mut st = state.borrow_mut();
        let render_ctx = ctx.render_context();
        // §7.1: tab content is chrome-hosted — it inherits the widget's
        // boundaries on the edges the tab bar leaves reachable.
        let content_area = ctx.content_area_for(content_rect);
        st.tabs[selected_index].content.place(
            ctx.renderer_mut(),
            render_ctx,
            env,
            bounded_proposal(content_rect),
            content_rect,
            content_area,
        );
    }
}

/// The environment a tab's label builds under: the bar owns the icon's
/// placement, so the label always renders its title alone.
fn tab_label_env(env: &Environment) -> Environment {
    env.extending(LabelDisplayMode::TitleOnly)
}

/// The extent a style's tab bar reports to `WidgetTheme::tabs_item_layout`
/// when the container is `container_width` wide: a bottom bar's extent is the
/// container's width; a sidebar strip reports its own thickness — the
/// vertical metrics' `bar_height` — which never widens into the pane.
pub fn tabs_bar_item_extent(
    container_width: f64,
    style: NativeTabStyle,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> f64 {
    match style {
        NativeTabStyle::Sidebar => theme.tabs_metrics(TabItemLayout::Vertical).bar_height,
        NativeTabStyle::Automatic | NativeTabStyle::TabBar => container_width,
    }
}

/// Decides the bar's item layout and metrics once for a measure pass: the
/// theme answers `tabs_item_layout` from the bar's own extent (see
/// [`tabs_bar_item_extent`]) computed from `item_sizes`, so the bar this pass
/// measures and the one the render pass draws under the same bounds agree.
#[expect(
    clippy::option_if_let_else,
    reason = "the if-let/else mirrors the control flow more clearly than the combinator chain here"
)]
pub fn tabs_decide_layout(
    theme: &Rc<dyn crate::engine::WidgetTheme>,
    style: NativeTabStyle,
    proposed_width: Option<f64>,
    item_sizes: &[(LayoutSize, Option<LayoutSize>)],
) -> (TabItemLayout, waterui_backend_core::widget::TabsMetrics) {
    let extent = match proposed_width {
        Some(width) => tabs_bar_item_extent(width, style, theme),
        None => {
            if style == NativeTabStyle::Sidebar {
                tabs_bar_item_extent(0.0, style, theme)
            } else {
                let vertical_metrics = theme.tabs_metrics(TabItemLayout::Vertical);
                item_sizes
                    .iter()
                    .map(|(label_size, icon_size)| {
                        tabs_item_natural_width(
                            *label_size,
                            *icon_size,
                            &vertical_metrics,
                            TabItemLayout::Vertical,
                        )
                    })
                    .sum()
            }
        }
    };
    let layout = theme.tabs_item_layout(extent, item_sizes.len());
    (layout, theme.tabs_metrics(layout))
}

/// A tab item's natural width for the bar's item layout: icon above the label
/// takes the wider of the two; icon beside the label adds them.
pub fn tabs_item_natural_width(
    label_size: LayoutSize,
    icon_size: Option<LayoutSize>,
    metrics: &waterui_backend_core::widget::TabsMetrics,
    layout: TabItemLayout,
) -> f64 {
    let content_width = match (layout, icon_size) {
        (TabItemLayout::Horizontal, Some(icon_size)) => {
            f64::from(icon_size.width) + metrics.icon_label_spacing + f64::from(label_size.width)
        }
        (_, Some(icon_size)) => f64::from(label_size.width).max(f64::from(icon_size.width)),
        (_, None) => f64::from(label_size.width),
    };
    metrics
        .button_horizontal_inset
        .mul_add(2.0, content_width)
        .max(metrics.button_min_width)
}

/// Places a tab item's icon and label inside its button rect. Vertical stacks
/// the icon above the label; horizontal puts the icon beside the label.
fn tabs_item_content_rects(
    button_rect: kurbo::Rect,
    icon_size: Option<LayoutSize>,
    label_size: LayoutSize,
    metrics: &waterui_backend_core::widget::TabsMetrics,
    layout: TabItemLayout,
) -> (Option<kurbo::Rect>, kurbo::Rect) {
    let Some(icon_size) = icon_size else {
        return (None, tabs_label_rect(button_rect, label_size, metrics));
    };
    let max_width = metrics
        .button_horizontal_inset
        .mul_add(-2.0, button_rect.width())
        .max(0.0);
    match layout {
        TabItemLayout::Vertical => {
            let icon_width = f64::from(icon_size.width).min(max_width);
            let icon_height = f64::from(icon_size.height).min(button_rect.height());
            let label_width = f64::from(label_size.width).min(max_width);
            let label_height = f64::from(label_size.height)
                .min((button_rect.height() - icon_height - metrics.icon_label_spacing).max(0.0));
            let total_height = icon_height + metrics.icon_label_spacing + label_height;
            let y0 = (button_rect.height() - total_height)
                .max(0.0)
                .mul_add(0.5, button_rect.y0);
            (
                Some(kurbo::Rect::new(
                    (button_rect.width() - icon_width).mul_add(0.5, button_rect.x0),
                    y0,
                    button_rect.x0 + f64::midpoint(button_rect.width(), icon_width),
                    y0 + icon_height,
                )),
                kurbo::Rect::new(
                    (button_rect.width() - label_width).mul_add(0.5, button_rect.x0),
                    y0 + icon_height + metrics.icon_label_spacing,
                    button_rect.x0 + f64::midpoint(button_rect.width(), label_width),
                    y0 + icon_height + metrics.icon_label_spacing + label_height,
                ),
            )
        }
        TabItemLayout::Horizontal => {
            let icon_width = f64::from(icon_size.width).min(max_width);
            let label_width = f64::from(label_size.width)
                .min((max_width - icon_width - metrics.icon_label_spacing).max(0.0));
            let icon_height = f64::from(icon_size.height).min(button_rect.height());
            let label_height = f64::from(label_size.height).min(button_rect.height());
            let total_width = icon_width + metrics.icon_label_spacing + label_width;
            let x0 = (button_rect.width() - total_width)
                .max(0.0)
                .mul_add(0.5, button_rect.x0);
            (
                Some(kurbo::Rect::new(
                    x0,
                    (button_rect.height() - icon_height).mul_add(0.5, button_rect.y0),
                    x0 + icon_width,
                    button_rect.y0 + f64::midpoint(button_rect.height(), icon_height),
                )),
                kurbo::Rect::new(
                    x0 + icon_width + metrics.icon_label_spacing,
                    (button_rect.height() - label_height).mul_add(0.5, button_rect.y0),
                    x0 + icon_width + metrics.icon_label_spacing + label_width,
                    button_rect.y0 + f64::midpoint(button_rect.height(), label_height),
                ),
            )
        }
    }
}

fn tabs_label_rect(
    button_rect: kurbo::Rect,
    label_size: waterui_core::layout::Size,
    metrics: &waterui_backend_core::widget::TabsMetrics,
) -> kurbo::Rect {
    let max_width = metrics
        .button_horizontal_inset
        .mul_add(-2.0, button_rect.width())
        .max(0.0);
    let width = f64::from(label_size.width).min(max_width);
    let height = f64::from(label_size.height).min(button_rect.height());
    let x0 = (button_rect.width() - width).mul_add(0.5, button_rect.x0);
    let y0 = (button_rect.height() - height).mul_add(0.5, button_rect.y0);
    kurbo::Rect::new(x0, y0, x0 + width, y0 + height)
}

fn tabs_highlight_rect(
    button_rect: kurbo::Rect,
    style: NativeTabStyle,
    thickness: f64,
    label_extent: f64,
    layout: TabItemLayout,
) -> kurbo::Rect {
    // Horizontal items highlight the whole item; the theme's metric supplies
    // the indicator's thickness centered on the item.
    if matches!(layout, TabItemLayout::Horizontal) {
        let height = thickness.min(button_rect.height());
        let y0 = (button_rect.height() - height).mul_add(0.5, button_rect.y0);
        return kurbo::Rect::new(button_rect.x0, y0, button_rect.x1, y0 + height);
    }
    match style {
        NativeTabStyle::Automatic | NativeTabStyle::TabBar => {
            let width = label_extent.clamp(0.0, button_rect.width());
            let x0 = (button_rect.width() - width).mul_add(0.5, button_rect.x0);
            let x1 = x0 + width;
            kurbo::Rect::new(
                x0,
                button_rect.y0,
                x1,
                (button_rect.y0 + thickness).min(button_rect.y1),
            )
        }
        NativeTabStyle::Sidebar => {
            let height = label_extent.clamp(0.0, button_rect.height());
            let y0 = (button_rect.height() - height).mul_add(0.5, button_rect.y0);
            kurbo::Rect::new(
                (button_rect.x1 - thickness).max(button_rect.x0),
                y0,
                button_rect.x1,
                y0 + height,
            )
        }
    }
}

/// Emits a retained tabs layout's accessibility tree for the semantic walk:
/// the tab bar and tab nodes `tabs_accessibility` registers (labels are
/// suppressed in the sub-view flush, so they emit nothing themselves), then
/// the selected tab's content subtree — it flushes unsuppressed in the
/// rendered path, so it emits its own nodes here too.
#[cfg(feature = "accessibility")]
pub fn emit_tabs_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<TabsRenderState>>,
    env: &Environment,
) {
    let owner = state;
    let mut state = state.borrow_mut();
    let selected_id = renderer.read_signal(&state.selection);
    let (selection, style, labels) = {
        let selected_index = state.selected_index(selected_id);
        let labels: Vec<(Id, crate::renderer::InteractionKey, Option<String>, bool)> = state
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                (
                    tab.tag,
                    crate::renderer::InteractionKey::for_rc(
                        owner,
                        crate::num_cast::i32_as_u32(i32::from(tab.tag)) as usize,
                    ),
                    tab.label.default_a11y_label(),
                    index == selected_index,
                )
            })
            .collect();
        (state.selection.clone(), state.style, labels)
    };
    tabs_accessibility(renderer, None, None, &selection, style, &labels, env);
    let selected_index = state.selected_index(selected_id);
    state.tabs[selected_index]
        .content
        .emit_accessibility(renderer, env);
}
