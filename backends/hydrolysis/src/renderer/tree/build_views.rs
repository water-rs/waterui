//! Builders for structured and visual views (list, table, navigation, tabs,
//! icon, gradient, shapes, webview, spacer, divider, plain text).

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
// A build with no selected engine reports a WebView as a contract violation.
#[cfg(all(
    waterui_feature = "webview",
    not(any(hydrolysis_webview_engine, hydrolysis_android_system_webview))
))]
use waterui_core::contract_violation;

#[cfg(feature = "list")]
impl_widget_behavior!(
    crate::widgets::layout::list::ListRenderState<E>,
    crate::widgets::layout::list::render_list_node,
    |state: &crate::widgets::layout::list::ListRenderState<E>, proposal, hydro, env, theme| {
        crate::widgets::layout::list::measure_list_node(state, proposal, hydro, env, theme)
    }
    ; prepare: prepare_first_row
    ; a11y: crate::widgets::layout::list::emit_list_accessibility
    ; surface: surface
);
#[cfg(feature = "table")]
impl_widget_behavior!(
    crate::widgets::layout::table::TableRenderState<E>,
    crate::widgets::layout::table::render_table_node,
    |state: &crate::widgets::layout::table::TableRenderState<E>, proposal, hydro, env, theme| {
        crate::widgets::layout::table::measure_table_node(state, proposal, hydro, env, theme)
    }
    ; prepare: prepare_headers
    ; a11y: crate::widgets::layout::table::emit_table_accessibility
    ; surface: surface
);
#[cfg(feature = "navigation")]
impl_widget_behavior!(
    crate::widgets::nav::navigation::NavigationViewRenderState<E>,
    crate::widgets::nav::navigation::render_navigation_view_node,
    crate::widgets::nav::navigation::measure_navigation_view_node
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::nav::navigation::emit_navigation_view_accessibility
    ; page: navigation_page_transition
);

/// The declaration on a navigation view's content root — the page root a
/// navigation stack owns.
#[cfg(feature = "navigation")]
fn navigation_page_transition<E: crate::text::TextEngine>(
    state: &crate::widgets::nav::navigation::NavigationViewRenderState<E>,
) -> Option<waterui_structural_transition::TransitionSpec> {
    super::transition::claim_transition(state.content_node())
}
#[cfg(feature = "navigation")]
impl_widget_behavior!(
    crate::widgets::nav::navigation::NavigationSplitRenderState<E>,
    crate::widgets::nav::navigation::render_navigation_split_node,
    crate::widgets::nav::navigation::measure_navigation_split_node
    ; prepare: prepare_columns
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::nav::navigation::emit_navigation_split_accessibility
);
#[cfg(feature = "navigation")]
impl_widget_behavior!(
    crate::widgets::nav::navigation::NavigationStackRenderState<E>,
    crate::widgets::nav::navigation::render_navigation_stack_node,
    crate::widgets::nav::navigation::measure_navigation_stack_node
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::nav::navigation::emit_navigation_stack_accessibility
);
#[cfg(feature = "navigation")]
impl_widget_behavior!(
    crate::widgets::nav::tabs::TabsRenderState<E>,
    crate::widgets::nav::tabs::render_tabs_node,
    crate::widgets::nav::tabs::measure_tabs_node
    ; prepare: prepare_tabs
    ; dirty: take_layout_dirty
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

impl<E: crate::text::TextEngine> RenderNode<E> {
    /// Build a persistent list node: retain the config (its `contents` collection,
    /// `editing` signal, and `on_delete`/`on_move` handlers stay owned by the cell).
    /// Each flush re-resolves the visible row window from the enclosing scroll's
    /// pushed viewport and re-dispatches only those rows, so reactive row content and
    /// a changing collection length stay live; the `LazyListController` row-extent
    /// cache (keyed by body-order cursor slot) persists across frames. Stretches to
    /// fill the proposal (`StretchAxis::Both`, read from the config).
    #[cfg(feature = "list")]
    pub(super) fn build_list(
        config: ListConfig<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
    ) -> Self {
        use crate::widgets::layout::list::ListRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let state = ListRenderState::from_config(config, env, renderer);
        // The first row is realized under this widget's cell as the record
        // reader, so its node attaches to the list rather than the window.
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.refresh_first_row(env, renderer);
        });
        Self::build_widget_with_core(Rc::new(RefCell::new(state)), stretch, env, core)
    }

    /// Build a persistent table node: retain the config (its `columns` signal and
    /// per-column reactive `rows` stay owned by the cell) and realize every
    /// column's header. Each flush re-reads the columns, re-resolves the visible
    /// row/column windows from the enclosing scroll's pushed viewport, and
    /// realizes only the data cells that become visible, so reactive cell content
    /// and a changing column/row set stay live. Stretch is read from the config.
    #[cfg(feature = "table")]
    pub(super) fn build_table(
        renderer: &mut SemanticCore<E>,
        config: TableConfig,
        env: &Environment,
    ) -> Self {
        use crate::widgets::layout::table::TableRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let state = TableRenderState::from_config(config);
        // The headers are realized under this widget's cell as the record
        // reader, so their nodes attach to the table rather than the window.
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            let columns = renderer.read_signal(&state.config.columns);
            state.refresh_headers(&columns, env, renderer);
        });
        Self::build_widget_with_core(Rc::new(RefCell::new(state)), stretch, env, core)
    }

    /// Build a persistent navigation-view node: its bar `title`/`leading`/`trailing`
    /// and screen `content` are move-only `AnyView`s pre-built into
    /// [`RetainedSubview`]s; the bar's `color`/`hidden` signals are read through
    /// `read_signal` each frame so an appearance change schedules a frame and the
    /// bar re-renders. Stretch is `Both` (read from the config).
    #[cfg(feature = "navigation")]
    pub(super) fn build_navigation_view(
        navigation: NavigationView<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
    ) -> Self {
        use crate::widgets::nav::navigation::NavigationViewRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&navigation);
        let mut state = NavigationViewRenderState::from_view(navigation, env);
        // See build_controls: the prebuild runs under this widget's
        // cell as the record reader, so its subviews attach to it.
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild(renderer, env);
        });
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }

    /// Build a persistent navigation-split node: the layout's sidebar/detail/
    /// placeholder are `Rc`-backed builders rebuilt fresh and re-dispatched each
    /// frame, and the `selection` binding is read through `read_signal` so a
    /// selection change schedules a frame and the detail re-resolves. Stretch is
    /// `Both` (read from the config).
    #[cfg(feature = "navigation")]
    pub(super) fn build_navigation_split(
        split: NavigationSplitLayout,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
    ) -> Self {
        use crate::widgets::nav::navigation::NavigationSplitRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&split);
        let mut state = NavigationSplitRenderState::from_layout(split);
        // Pre-build the sidebar + placeholder sub-views (the measure path has no renderer).
        // See build_controls: the prebuild runs under this widget's
        // cell as the record reader, so its subviews attach to it.
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild(renderer, env);
        });
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }

    /// Build a persistent navigation-stack node: the move-only stack root is held in
    /// a [`RetainedSubview`] (re-rendered into a fresh scene each frame for the
    /// transition cross-fade), and pushed destinations are rebuilt from their
    /// `Rc`-backed builders each flush; the navigation controller and transition
    /// slot persist across frames so push/pop transitions keep animating. Stretch is
    /// `Both` (read from the config).
    #[cfg(feature = "navigation")]
    pub(super) fn build_navigation_stack(
        renderer: &SemanticCore<E>,
        stack: NavigationStackLayout<HeldChild<E>>,
        env: &Environment,
    ) -> Self {
        use crate::widgets::nav::navigation::NavigationStackRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&stack);
        let state = Rc::new(RefCell::new(NavigationStackRenderState::from_stack(stack)));
        Self::build_widget(renderer, state, stretch, env)
    }

    /// Build a persistent tabs node: the tab labels are move-only `AnyView`s
    /// pre-built into [`RetainedSubview`]s, the selected tab's content is rebuilt
    /// from its `Rc`-backed builder each flush, and the `selection` binding is read
    /// through `read_signal` so a tab change schedules a frame and the active
    /// content/indicator updates. Stretch is `Both` (read from the config).
    #[cfg(feature = "navigation")]
    pub(super) fn build_tabs(
        tabs: TabsLayout<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
    ) -> Self {
        use crate::widgets::nav::tabs::TabsRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&tabs);
        let mut state = TabsRenderState::from_tabs(tabs, renderer);
        // See build_controls: the prebuild runs under this widget's
        // cell as the record reader, so its subviews attach to it.
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild_labels(renderer, env);
        });
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }

    /// Build a persistent gradient node: retain the view — its `Paint` is
    /// already a resolved engine paint in unit space — and re-fill it every
    /// flush at the current bounds. The payload carries no signal, so nothing
    /// is watched; the gradient stretches to fill the proposal
    /// (`StretchAxis::Both`, read from the payload). A gradient paints a
    /// fill — `fill_leaf` marks it so a background slot can give it §7.1's
    /// band extension.
    pub(super) fn build_gradient(
        renderer: &SemanticCore<E>,
        gradient: waterui_graphics::Gradient,
        env: &Environment,
    ) -> Self {
        let stretch = waterui_core::NativeView::stretch_axis(&gradient);
        let gradient = Rc::new(RefCell::new(gradient));
        let mut node = Self::build_widget(renderer, gradient, stretch, env);
        if let Self::Widget(widget) = &mut node {
            widget.fill_leaf = true;
        }
        node
    }

    /// Build a persistent shape node: retain the resolved shape payload and re-fill
    /// its bounds-aware path every flush. The fill signal is observed by the render
    /// path so theme and binding changes patch this node precisely. The shape
    /// stretches to fill the proposal (`StretchAxis::Both`, read from the payload).
    pub(super) fn build_shape(
        renderer: &SemanticCore<E>,
        shape: ResolvedShape,
        env: &Environment,
    ) -> Self {
        let stretch = waterui_core::NativeView::stretch_axis(&shape);
        let shape = Rc::new(RefCell::new(shape));
        Self::build_widget(renderer, shape, stretch, env)
    }

    /// Build the `WebView` leaf. Where a hosted system bridge exists it
    /// mounts through the hosted-content leaf (`WKWebView` on macOS, an
    /// `<iframe>` on the web); on Android the `HydrolysisWebView` wrapper the
    /// controller opened mounts as a platform-view *instance* placement,
    /// keyed by the id Kotlin registered it under — the page's handle rides
    /// in the node as the instance owner so the native peer lives exactly as
    /// long as the leaf that embeds it. Where no engine is bridged at all a
    /// `WebView` that still reaches the backend has nothing to draw it — a
    /// missing realization, not a drawable stand-in.
    #[cfg(all(
        waterui_feature = "webview",
        not(any(hydrolysis_cef_webview, hydrolysis_wpe_webview))
    ))]
    pub(super) fn build_webview(
        open: waterui::webview::WebViewOpen,
        env: &Environment,
        renderer: &SemanticCore<E>,
    ) -> Self {
        #[cfg(hydrolysis_system_webview)]
        {
            let controller =
                crate::widgets::platform::webview::SelectedWebViewController::default();
            let Some(opened) = open.open_with(&controller, env) else {
                return Self::build_empty(renderer, env);
            };
            let hosted = crate::widgets::platform::webview::hosted_from_handle(opened.handle());
            Self::build_hosted_retained(hosted, env, renderer, opened.into_webview())
        }
        #[cfg(hydrolysis_android_system_webview)]
        {
            use crate::widgets::platform::platform_view::PlatformViewRenderState;
            use crate::widgets::platform::webview::AndroidSystemWebViewHandle;

            let stretch = waterui_core::NativeView::stretch_axis(&open);
            let controller = renderer.android_webview_controller();
            let Some(opened) = open.open_with(&controller, env) else {
                return Self::build_empty(renderer, env);
            };
            let instance = AndroidSystemWebViewHandle::instance(opened.handle());
            let owner: Rc<dyn core::any::Any> = Rc::new(opened.into_webview());
            let Some(state) = PlatformViewRenderState::from_instance(instance, owner, env) else {
                return Self::build_empty(renderer, env);
            };
            let state = Rc::new(RefCell::new(state));
            Self::build_widget(renderer, state, stretch, env)
        }
        #[cfg(not(any(hydrolysis_system_webview, hydrolysis_android_system_webview)))]
        {
            drop(open);
            contract_violation!("{UNSUPPORTED_WEBVIEW}; the web view is not drawn");
            Self::build_empty(renderer, env)
        }
    }

    /// Build a persistent spacer node: a no-op render with zero intrinsic; it
    /// expands during placement, not from its intrinsic size. Stretch is its
    /// main-axis fill (`StretchAxis::MainAxis`, read from the config).
    pub(super) fn build_spacer(
        renderer: &SemanticCore<E>,
        spacer: Spacer,
        env: &Environment,
    ) -> Self {
        let stretch = waterui_core::NativeView::stretch_axis(&spacer);
        let spacer = Rc::new(RefCell::new(spacer));
        Self::build_widget(renderer, spacer, stretch, env)
    }

    /// Build a persistent empty (`()`) node: a no-op render with zero intrinsic and
    /// no accessibility. Never stretches (`StretchAxis::None`, read from the unit
    /// view).
    pub(super) fn build_empty(renderer: &SemanticCore<E>, env: &Environment) -> Self {
        let stretch = waterui_core::NativeView::stretch_axis(&());
        let empty = Rc::new(RefCell::new(()));
        Self::build_widget(renderer, empty, stretch, env)
    }

    /// Build a persistent divider node: a static separator line drawn from theme
    /// metrics, oriented by the enclosing stack axis (read from env every flush).
    /// Stretches on its cross axis (`StretchAxis::CrossAxis`).
    pub(super) fn build_divider(
        renderer: &SemanticCore<E>,
        divider: Divider,
        env: &Environment,
    ) -> Self {
        let divider = Rc::new(RefCell::new(divider));
        Self::build_widget(renderer, divider, StretchAxis::CrossAxis, env)
    }

    /// Build a persistent string node: an immutable `Str` rendered as plain styled
    /// text each flush (no signal — its content never changes for a given node).
    /// Never stretches (`StretchAxis::None`, like text).
    pub(super) fn build_str(renderer: &SemanticCore<E>, text: Str, env: &Environment) -> Self {
        let text = Rc::new(RefCell::new(text));
        Self::build_widget(renderer, text, StretchAxis::None, env)
    }

    /// Build a semantic-only leaf for a backend-provided component (`AppleView`,
    /// `AndroidView`, …) that answered [`waterui_backend_core::ProvidedSemantics`]
    /// on [`crate::SemanticRuntime`]. Never constructed on rendered runtimes.
    #[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
    pub(super) fn build_provided_semantics(
        renderer: &SemanticCore<E>,
        semantics: waterui_backend_core::ProvidedSemantics,
        stretch: StretchAxis,
        env: &Environment,
    ) -> Self {
        let state = Rc::new(ProvidedSemanticsNode { semantics });
        Self::build_widget(renderer, state, stretch, env)
    }
}

#[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
struct ProvidedSemanticsNode {
    semantics: waterui_backend_core::ProvidedSemantics,
}

#[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
impl<E: crate::text::TextEngine> WidgetBehavior<E> for ProvidedSemanticsNode {
    fn measure(
        &self,
        _state: &mut HydroState<E>,
        _proposal: ProposalSize,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> ViewDimensions {
        ViewDimensions::new(Size::zero())
    }

    fn render(
        self: Rc<Self>,
        _renderer: &mut HydrolysisRenderer<E>,
        _ctx: RenderContext,
        _env: &Environment,
        _safe_area: Option<safe_area::SafeAreaLayout>,
    ) {
        unreachable!("ProvidedSemanticsNode is built only on SemanticRuntime (`semantic_walk`)");
    }

    fn emit_accessibility(self: Rc<Self>, renderer: &mut SemanticCore<E>, env: &Environment) {
        if env
            .get::<AccessibilityHidden>()
            .is_some_and(AccessibilityHidden::is_hidden)
        {
            return;
        }
        let default_label = self
            .semantics
            .label
            .as_ref()
            .map(|signal| renderer.read_signal(signal).as_str().to_owned());
        let label = renderer.resolve_accessibility_label(env, default_label);
        let value = renderer.resolve_accessibility_value(env, None);
        if env.get::<AccessibilityRole>().is_none()
            && self.semantics.role.is_none()
            && label.is_none()
            && value.is_none()
        {
            return;
        }
        let default_role =
            self.semantics
                .role
                .as_ref()
                .map_or(AccessibilityNodeRole::Group, |role| {
                    crate::renderer::accessibility::accessibility_role_to_accesskit_role(
                        role.clone(),
                    )
                });
        let mut node = AccessibilityNode::new(SemanticCore::<E>::resolve_accessibility_role(
            env,
            default_role,
        ));
        if let Some(label) = label {
            node.set_label(label);
        }
        if let Some(value) = value {
            node.set_value(value);
        }
        let _ = renderer.register_accessibility_leaf(None, node, env, None);
    }

    fn accessibility_label(
        &self,
        renderer: &mut SemanticCore<E>,
        _env: &Environment,
    ) -> Option<String> {
        self.semantics
            .label
            .as_ref()
            .map(|signal| renderer.read_signal(signal).as_str().to_owned())
    }
}
