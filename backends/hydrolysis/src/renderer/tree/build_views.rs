//! Builders for structured and visual views (list, table, navigation, tabs,
//! icon, gradient, shapes, map, webview, spacer, divider, plain text).

use super::*;

impl RenderNode {
    /// Build a persistent list node: retain the config (its `contents` collection,
    /// `editing` signal, and `on_delete`/`on_move` handlers stay owned by the cell).
    /// Each flush re-resolves the visible row window from the enclosing scroll's
    /// pushed viewport and re-dispatches only those rows, so reactive row content and
    /// a changing collection length stay live; the `LazyListController` row-extent
    /// cache (keyed by body-order cursor slot) persists across frames. Stretches to
    /// fill the proposal (`StretchAxis::Both`, read from the config).
    pub(super) fn build_list(config: ListConfig, env: &Environment) -> RenderNode {
        use crate::widgets::layout::list::{ListRenderState, measure_list_node, render_list_node};
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let state = Rc::new(RefCell::new(ListRenderState::from_config(config)));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_list_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_list_node(&state.borrow().config, proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent table node: retain the config (its `columns` signal and
    /// per-column reactive `rows` stay owned by the cell). Each flush re-reads the
    /// columns, re-resolves the visible row/column windows from the enclosing
    /// scroll's pushed viewport, and re-dispatches only those header/data cells, so
    /// reactive cell content and a changing column/row set stay live; the
    /// `LazyTableController` column-width/row-count cache (keyed by body-order cursor
    /// slot) persists across frames. Stretch is read from the config.
    pub(super) fn build_table(config: TableConfig, env: &Environment) -> RenderNode {
        use crate::widgets::layout::table::{
            TableRenderState, measure_table_node, render_table_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let state = Rc::new(RefCell::new(TableRenderState::from_config(config)));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_table_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_table_node(&state.borrow().config, proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent navigation-view node: its bar `title`/`leading`/`trailing`
    /// and screen `content` are move-only `AnyView`s pre-built into
    /// [`RetainedSubview`]s; the bar's `color`/`hidden` signals are read through
    /// `read_signal` each frame so an appearance change schedules a frame and the
    /// bar re-renders. Stretch is `Both` (read from the config).
    pub(super) fn build_navigation_view(
        navigation: NavigationView,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::nav::navigation::{
            NavigationViewRenderState, measure_navigation_view_node, render_navigation_view_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&navigation);
        let mut state = NavigationViewRenderState::from_view(navigation, env);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_navigation_view_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        // The navigation view's measure reads its un-retained config; it is
        // intrinsic-sized (or fills a concrete proposal), independent of the
        // retained sub-views, so the measure closure holds the original config.
        let measure_config = Rc::clone(&state);
        let measure = Box::new(
            move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                measure_navigation_view_node(&measure_config.borrow(), proposal, hydro, env)
            },
        )
            as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>;
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent navigation-split node: the layout's sidebar/detail/
    /// placeholder are `Rc`-backed builders rebuilt fresh and re-dispatched each
    /// frame, and the `selection` binding is read through `read_signal` so a
    /// selection change schedules a frame and the detail re-resolves. Stretch is
    /// `Both` (read from the config).
    pub(super) fn build_navigation_split(
        split: NavigationSplitLayout,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::nav::navigation::{
            NavigationSplitRenderState, measure_navigation_split_node, render_navigation_split_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&split);
        let mut state = NavigationSplitRenderState::from_layout(split);
        // Pre-build the sidebar + placeholder sub-views (the measure path has no renderer).
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_navigation_split_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_navigation_split_node(&state.borrow().split, proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent navigation-stack node: the move-only stack root is held in
    /// a [`RetainedSubview`] (re-rendered into a fresh scene each frame for the
    /// transition cross-fade), and pushed destinations are rebuilt from their
    /// `Rc`-backed builders each flush; the navigation controller and transition
    /// slot persist across frames so push/pop transitions keep animating. Stretch is
    /// `Both` (read from the config).
    pub(super) fn build_navigation_stack(
        stack: NavigationStack<(), ()>,
        env: &Environment,
    ) -> RenderNode {
        use crate::widgets::nav::navigation::{
            NavigationStackRenderState, measure_navigation_stack_node, render_navigation_stack_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&stack);
        let state = Rc::new(RefCell::new(NavigationStackRenderState::from_stack(stack)));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_navigation_stack_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_navigation_stack_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent tabs node: the tab labels are move-only `AnyView`s
    /// pre-built into [`RetainedSubview`]s, the selected tab's content is rebuilt
    /// from its `Rc`-backed builder each flush, and the `selection` binding is read
    /// through `read_signal` so a tab change schedules a frame and the active
    /// content/indicator updates. Stretch is `Both` (read from the config).
    pub(super) fn build_tabs(
        tabs: Tabs,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::nav::tabs::{TabsRenderState, measure_tabs_node, render_tabs_node};
        let stretch = waterui_core::NativeView::stretch_axis(&tabs);
        let mut state = TabsRenderState::from_tabs(tabs);
        state.prebuild_labels(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_tabs_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_tabs_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent system-icon node: the placeholder marker is a move-only
    /// `AnyView` pre-built into a [`RetainedSubview`] (Hydrolysis has no OS icon
    /// catalog, so it draws a neutral missing-glyph marker). Static, content-sized
    /// (`StretchAxis::None`, read from the config).
    pub(super) fn build_icon(
        icon: SystemIcon,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::visual::icon::{IconRenderState, measure_icon_node, render_icon_node};
        let stretch = waterui_core::NativeView::stretch_axis(&icon);
        let mut state = IconRenderState::from_icon(&icon, env);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_icon_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_icon_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent gradient node: retain the fully-resolved gradient payload
    /// behind an `Rc<RefCell<…>>` and re-fill it every flush at the current bounds.
    /// The payload carries no signal, so nothing is watched; the gradient stretches
    /// to fill the proposal (`StretchAxis::Both`, read from the payload).
    pub(super) fn build_gradient(gradient: ResolvedGradient, env: &Environment) -> RenderNode {
        use crate::renderer::{measure_gradient_node, render_gradient_node};
        let stretch = waterui_core::NativeView::stretch_axis(&gradient);
        let gradient = Rc::new(RefCell::new(gradient));
        let render = {
            let gradient = Rc::clone(&gradient);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_gradient_node(&mut widget_ctx, &gradient, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let gradient = Rc::clone(&gradient);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_gradient_node(&gradient.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent shape node: retain the resolved shape payload and re-fill
    /// its bounds-aware path every flush. The fill signal is observed by the render
    /// path so theme and binding changes patch this node precisely. The shape
    /// stretches to fill the proposal (`StretchAxis::Both`, read from the payload).
    pub(super) fn build_shape(shape: ResolvedShape, env: &Environment) -> RenderNode {
        use crate::renderer::{measure_shape_node, render_shape_node};
        let stretch = waterui_core::NativeView::stretch_axis(&shape);
        let shape = Rc::new(RefCell::new(shape));
        let render = {
            let shape = Rc::clone(&shape);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_shape_node(&mut widget_ctx, &shape, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let shape = Rc::clone(&shape);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_shape_node(&shape.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent morph-shape node: retain the resolved morph payload and
    /// re-fill its interpolated path every flush. The morph progress is resolved
    /// through the animation controller each frame — an explicit `progress` signal
    /// is watched via `read_signal`/`resolve_animated_scalar_with_discriminator`, and
    /// a time-based animation drives continuous frames via `sample_morph_progress` —
    /// so the morph stays live. Stretches to fill the proposal (`StretchAxis::Both`).
    pub(super) fn build_morph_shape(shape: ResolvedMorphShape, env: &Environment) -> RenderNode {
        use crate::renderer::{measure_morph_shape_node, render_morph_shape_node};
        let stretch = waterui_core::NativeView::stretch_axis(&shape);
        let shape = Rc::new(RefCell::new(shape));
        let render = {
            let shape = Rc::clone(&shape);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_morph_shape_node(&mut widget_ctx, &shape, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let shape = Rc::clone(&shape);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_morph_shape_node(&shape.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent map node: the map is a Rust-side composer whose content
    /// (`vstack` of a gradient surface + reactive region/annotation `Text`s) is
    /// built once into a [`RetainedSubview`] and re-flushed at the node's bounds each
    /// frame. Region/annotation reactivity is carried by the inner `Text` nodes
    /// (`config.region.map(...)`), which become live `Dynamic`/`Text` nodes inside the
    /// sub-view, so a region or annotation change updates without rebuilding the node.
    /// A11y is render-driven (the inner content's own dispatch emits it). Stretches to
    /// fill the proposal (`StretchAxis::Both`, read from the config).
    pub(super) fn build_map(
        config: MapConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::visual::map::{MapRenderState, measure_map_node, render_map_node};
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = MapRenderState::from_config(config, env);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_map_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_map_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent webview node: the webview is a Rust-side composer whose
    /// content (`vstack` of a gradient surface + reactive status/navigation `Text`s)
    /// is built once into a [`RetainedSubview`] and re-flushed each frame. The
    /// `event`/`can_go_back`/`can_go_forward` reactivity is carried by the inner
    /// `Text` nodes, which become live `Dynamic`/`Text` nodes, so a navigation or load
    /// event updates without rebuilding the node. A11y is render-driven (the inner
    /// content's own dispatch emits it). Stretches to fill the proposal.
    pub(super) fn build_webview(
        webview: WebView,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::platform::webview::{
            WebViewRenderState, measure_webview_node, render_webview_node,
        };
        let stretch = waterui_core::View::stretch_axis(&webview);
        let mut state = WebViewRenderState::from_view(webview, env);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_webview_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_webview_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent spacer node: a no-op render with zero intrinsic; it
    /// expands during placement, not from its intrinsic size. Stretch is its
    /// main-axis fill (`StretchAxis::MainAxis`, read from the config).
    pub(super) fn build_spacer(spacer: Spacer, env: &Environment) -> RenderNode {
        use crate::widgets::layout::spacer::{measure_spacer_node, render_spacer_node};
        let stretch = waterui_core::NativeView::stretch_axis(&spacer);
        let spacer = Rc::new(RefCell::new(spacer));
        let render = {
            let spacer = Rc::clone(&spacer);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_spacer_node(&mut widget_ctx, &spacer, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let spacer = Rc::clone(&spacer);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_spacer_node(&spacer.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent empty (`()`) node: a no-op render with zero intrinsic and
    /// no accessibility. Never stretches (`StretchAxis::None`, read from the unit
    /// view).
    pub(super) fn build_empty(env: &Environment) -> RenderNode {
        use crate::widgets::layout::spacer::{measure_empty_node, render_empty_node};
        let stretch = waterui_core::NativeView::stretch_axis(&());
        let empty = Rc::new(RefCell::new(()));
        let render = {
            let empty = Rc::clone(&empty);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_empty_node(&mut widget_ctx, &empty, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let empty = Rc::clone(&empty);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_empty_node(&empty.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent divider node: a static separator line drawn from theme
    /// metrics, oriented by the enclosing stack axis (read from env every flush).
    /// Stretches on its cross axis (`StretchAxis::CrossAxis`, matching the dispatch
    /// path's [`effective_stretch_axis`] for `Divider`).
    pub(super) fn build_divider(divider: Divider, env: &Environment) -> RenderNode {
        use crate::widgets::divider::{measure_divider_node, render_divider_node};
        let divider = Rc::new(RefCell::new(divider));
        let render = {
            let divider = Rc::clone(&divider);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_divider_node(&mut widget_ctx, &divider, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let divider = Rc::clone(&divider);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_divider_node(&divider.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, StretchAxis::CrossAxis, env)
    }

    /// Build a persistent string node: an immutable `Str` rendered as plain styled
    /// text each flush (no signal — its content never changes for a given node).
    /// Never stretches (`StretchAxis::None`, like text).
    pub(super) fn build_str(text: Str, env: &Environment) -> RenderNode {
        use crate::renderer::views::{measure_str_node, render_str_node};
        let text = Rc::new(RefCell::new(text));
        let render = {
            let text = Rc::clone(&text);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_str_node(&mut widget_ctx, &text, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let text = Rc::clone(&text);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_str_node(&text.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, StretchAxis::None, env)
    }
}
