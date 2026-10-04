//! Builders for structured and visual views (list, table, navigation, tabs,
//! icon, gradient, shapes, webview, spacer, divider, plain text).

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

impl_widget_behavior!(
    crate::widgets::layout::list::ListRenderState,
    crate::widgets::layout::list::render_list_node,
    |state: &crate::widgets::layout::list::ListRenderState, proposal, hydro, env, theme| {
        crate::widgets::layout::list::measure_list_node(&state.config, proposal, hydro, env, theme)
    }
    ; a11y: crate::widgets::layout::list::emit_list_accessibility
);
impl_widget_behavior!(
    crate::widgets::layout::table::TableRenderState,
    crate::widgets::layout::table::render_table_node,
    |state: &crate::widgets::layout::table::TableRenderState, proposal, hydro, env, theme| {
        crate::widgets::layout::table::measure_table_node(&state.config, proposal, hydro, env, theme)
    }
    ; a11y: crate::widgets::layout::table::emit_table_accessibility
);
impl_widget_behavior!(
    crate::widgets::nav::navigation::NavigationViewRenderState,
    crate::widgets::nav::navigation::render_navigation_view_node,
    crate::widgets::nav::navigation::measure_navigation_view_node
    ; a11y: crate::widgets::nav::navigation::emit_navigation_view_accessibility
);
impl_widget_behavior!(
    crate::widgets::nav::navigation::NavigationSplitRenderState,
    crate::widgets::nav::navigation::render_navigation_split_node,
    crate::widgets::nav::navigation::measure_navigation_split_node
    ; prepare: prepare_columns
    ; a11y: crate::widgets::nav::navigation::emit_navigation_split_accessibility
);
impl_widget_behavior!(
    crate::widgets::nav::navigation::NavigationStackRenderState,
    crate::widgets::nav::navigation::render_navigation_stack_node,
    crate::widgets::nav::navigation::measure_navigation_stack_node
    ; a11y: crate::widgets::nav::navigation::emit_navigation_stack_accessibility
);
impl_widget_behavior!(
    crate::widgets::nav::tabs::TabsRenderState,
    crate::widgets::nav::tabs::render_tabs_node,
    crate::widgets::nav::tabs::measure_tabs_node
    ; a11y: crate::widgets::nav::tabs::emit_tabs_accessibility
);
impl_widget_behavior!(
    waterui_graphics::Gradient,
    crate::renderer::render_gradient_node,
    crate::renderer::measure_gradient_node
    ; a11y: crate::renderer::views::emit_graphics_leaf_accessibility
);
impl_widget_behavior!(
    ResolvedShape,
    crate::renderer::render_shape_node,
    crate::renderer::measure_shape_node
    ; a11y: crate::renderer::views::emit_graphics_leaf_accessibility
);
impl_widget_behavior!(
    ResolvedMorphShape,
    crate::renderer::render_morph_shape_node,
    crate::renderer::measure_morph_shape_node
    ; a11y: crate::renderer::views::emit_graphics_leaf_accessibility
);
#[cfg(hydrolysis_macos_system_webview)]
impl_widget_behavior!(
    crate::widgets::platform::webview::WebViewRenderState,
    crate::widgets::platform::webview::render_webview_node,
    crate::widgets::platform::webview::measure_webview_node
);
impl_widget_behavior!(
    Spacer,
    crate::widgets::layout::spacer::render_spacer_node,
    crate::widgets::layout::spacer::measure_spacer_node,
    Spacer::DEFAULT_LAYOUT_PRIORITY
);
impl_widget_behavior!(
    (),
    crate::widgets::layout::spacer::render_empty_node,
    crate::widgets::layout::spacer::measure_empty_node
    ; renders_nothing: true
);
impl_widget_behavior!(
    Divider,
    crate::widgets::divider::render_divider_node,
    crate::widgets::divider::measure_divider_node
);
impl_widget_behavior!(
    Str,
    crate::renderer::views::render_str_node,
    crate::renderer::views::measure_str_node
    ; a11y: crate::renderer::views::emit_str_accessibility
);

impl RenderNode {
    /// Build a persistent list node: retain the config (its `contents` collection,
    /// `editing` signal, and `on_delete`/`on_move` handlers stay owned by the cell).
    /// Each flush re-resolves the visible row window from the enclosing scroll's
    /// pushed viewport and re-dispatches only those rows, so reactive row content and
    /// a changing collection length stay live; the `LazyListController` row-extent
    /// cache (keyed by body-order cursor slot) persists across frames. Stretches to
    /// fill the proposal (`StretchAxis::Both`, read from the config).
    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the mutable borrow is required by the shared signature even though this implementation does not mutate it"
    )]
    pub(super) fn build_list(
        config: ListConfig,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        use crate::widgets::layout::list::ListRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let state = Rc::new(RefCell::new(ListRenderState::from_config(config, renderer)));
        Self::build_widget(state, stretch, env)
    }

    /// Build a persistent table node: retain the config (its `columns` signal and
    /// per-column reactive `rows` stay owned by the cell). Each flush re-reads the
    /// columns, re-resolves the visible row/column windows from the enclosing
    /// scroll's pushed viewport, and re-dispatches only those header/data cells, so
    /// reactive cell content and a changing column/row set stay live; the
    /// `LazyTableController` column-width/row-count cache (keyed by body-order cursor
    /// slot) persists across frames. Stretch is read from the config.
    pub(super) fn build_table(config: TableConfig, env: &Environment) -> Self {
        use crate::widgets::layout::table::TableRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let state = Rc::new(RefCell::new(TableRenderState::from_config(config)));
        Self::build_widget(state, stretch, env)
    }

    /// Build a persistent navigation-view node: its bar `title`/`leading`/`trailing`
    /// and screen `content` are move-only `AnyView`s pre-built into
    /// [`RetainedSubview`]s; the bar's `color`/`hidden` signals are read through
    /// `read_signal` each frame so an appearance change schedules a frame and the
    /// bar re-renders. Stretch is `Both` (read from the config).
    pub(super) fn build_navigation_view(
        navigation: NavigationView,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        use crate::widgets::nav::navigation::NavigationViewRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&navigation);
        let mut state = NavigationViewRenderState::from_view(navigation, env);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(state, stretch, env)
    }

    /// Build a persistent navigation-split node: the layout's sidebar/detail/
    /// placeholder are `Rc`-backed builders rebuilt fresh and re-dispatched each
    /// frame, and the `selection` binding is read through `read_signal` so a
    /// selection change schedules a frame and the detail re-resolves. Stretch is
    /// `Both` (read from the config).
    pub(super) fn build_navigation_split(
        split: NavigationSplitLayout,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        use crate::widgets::nav::navigation::NavigationSplitRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&split);
        let mut state = NavigationSplitRenderState::from_layout(split);
        // Pre-build the sidebar + placeholder sub-views (the measure path has no renderer).
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(state, stretch, env)
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
    ) -> Self {
        use crate::widgets::nav::navigation::NavigationStackRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&stack);
        let state = Rc::new(RefCell::new(NavigationStackRenderState::from_stack(stack)));
        Self::build_widget(state, stretch, env)
    }

    /// Build a persistent tabs node: the tab labels are move-only `AnyView`s
    /// pre-built into [`RetainedSubview`]s, the selected tab's content is rebuilt
    /// from its `Rc`-backed builder each flush, and the `selection` binding is read
    /// through `read_signal` so a tab change schedules a frame and the active
    /// content/indicator updates. Stretch is `Both` (read from the config).
    pub(super) fn build_tabs(
        tabs: TabsLayout,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        use crate::widgets::nav::tabs::TabsRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&tabs);
        let mut state = TabsRenderState::from_tabs(tabs);
        state.prebuild_labels(renderer, env);
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(state, stretch, env)
    }

    /// Build a persistent gradient node: retain the view — its `Paint` is
    /// already a resolved engine paint in unit space — and re-fill it every
    /// flush at the current bounds. The payload carries no signal, so nothing
    /// is watched; the gradient stretches to fill the proposal
    /// (`StretchAxis::Both`, read from the payload).
    pub(super) fn build_gradient(gradient: waterui_graphics::Gradient, env: &Environment) -> Self {
        let stretch = waterui_core::NativeView::stretch_axis(&gradient);
        let gradient = Rc::new(RefCell::new(gradient));
        Self::build_widget(gradient, stretch, env)
    }

    /// Build a persistent shape node: retain the resolved shape payload and re-fill
    /// its bounds-aware path every flush. The fill signal is observed by the render
    /// path so theme and binding changes patch this node precisely. The shape
    /// stretches to fill the proposal (`StretchAxis::Both`, read from the payload).
    pub(super) fn build_shape(shape: ResolvedShape, env: &Environment) -> Self {
        let stretch = waterui_core::NativeView::stretch_axis(&shape);
        let shape = Rc::new(RefCell::new(shape));
        Self::build_widget(shape, stretch, env)
    }

    /// Build a persistent morph-shape node: retain the resolved morph payload and
    /// re-fill its interpolated path every flush. The morph progress is resolved
    /// through the animation controller each frame — an explicit `progress` signal
    /// is watched via `read_signal`/`resolve_animated_scalar_with_discriminator`, and
    /// a time-based animation drives continuous frames via `sample_morph_progress` —
    /// so the morph stays live. Stretches to fill the proposal (`StretchAxis::Both`).
    pub(super) fn build_morph_shape(shape: ResolvedMorphShape, env: &Environment) -> Self {
        let stretch = waterui_core::NativeView::stretch_axis(&shape);
        let shape = Rc::new(RefCell::new(shape));
        Self::build_widget(shape, stretch, env)
    }

    /// Build a persistent webview node for the platform bridge: retain the
    /// semantic `WebView` and its `MacSystemWebViewHandle` so the AppKit view
    /// host keeps drawing it across flushes. Stretches to fill the proposal.
    #[cfg(hydrolysis_macos_system_webview)]
    pub(super) fn build_webview(
        webview: WebView,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> RenderNode {
        use crate::widgets::platform::webview::WebViewRenderState;
        let stretch = waterui_core::View::stretch_axis(&webview);
        let mut state = WebViewRenderState::from_view(webview, env);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(state, stretch, env)
    }

    /// Without the platform bridge a `WebView` reaching the backend has no
    /// engine to draw it — a missing realization, not a drawable stand-in.
    #[cfg(not(hydrolysis_macos_system_webview))]
    pub(super) fn build_webview(
        _webview: WebView,
        _env: &Environment,
        _renderer: &mut SemanticCore,
    ) -> Self {
        unsupported_webview()
    }

    /// Build a persistent spacer node: a no-op render with zero intrinsic; it
    /// expands during placement, not from its intrinsic size. Stretch is its
    /// main-axis fill (`StretchAxis::MainAxis`, read from the config).
    pub(super) fn build_spacer(spacer: Spacer, env: &Environment) -> Self {
        let stretch = waterui_core::NativeView::stretch_axis(&spacer);
        let spacer = Rc::new(RefCell::new(spacer));
        Self::build_widget(spacer, stretch, env)
    }

    /// Build a persistent empty (`()`) node: a no-op render with zero intrinsic and
    /// no accessibility. Never stretches (`StretchAxis::None`, read from the unit
    /// view).
    pub(super) fn build_empty(env: &Environment) -> Self {
        let stretch = waterui_core::NativeView::stretch_axis(&());
        let empty = Rc::new(RefCell::new(()));
        Self::build_widget(empty, stretch, env)
    }

    /// Build a persistent divider node: a static separator line drawn from theme
    /// metrics, oriented by the enclosing stack axis (read from env every flush).
    /// Stretches on its cross axis (`StretchAxis::CrossAxis`, matching the dispatch
    /// path's [`effective_stretch_axis`] for `Divider`).
    pub(super) fn build_divider(divider: Divider, env: &Environment) -> Self {
        let divider = Rc::new(RefCell::new(divider));
        Self::build_widget(divider, StretchAxis::CrossAxis, env)
    }

    /// Build a persistent string node: an immutable `Str` rendered as plain styled
    /// text each flush (no signal — its content never changes for a given node).
    /// Never stretches (`StretchAxis::None`, like text).
    pub(super) fn build_str(text: Str, env: &Environment) -> Self {
        let text = Rc::new(RefCell::new(text));
        Self::build_widget(text, StretchAxis::None, env)
    }
}
