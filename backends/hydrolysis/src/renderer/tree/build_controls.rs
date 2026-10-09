//! Widget-leaf builders for native controls (button, toggle, slider, stepper,
//! progress, menu, pickers, text fields, badge): each retains the config's
//! live signals in a [`WidgetNode`] re-dispatched every flush.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

impl_widget_behavior!(
    crate::widgets::controls::button::ButtonRenderState,
    crate::widgets::controls::button::render_button_node,
    crate::widgets::controls::button::measure_button_node
    ; prepare: ensure_label_built
    ; a11y: crate::widgets::controls::button::emit_button_accessibility
);
impl_widget_behavior!(
    crate::widgets::controls::toggle::ToggleRenderState,
    crate::widgets::controls::toggle::render_toggle_node,
    crate::widgets::controls::toggle::measure_toggle_node
    ; a11y: crate::widgets::controls::toggle::emit_toggle_accessibility
);
impl_widget_behavior!(
    crate::widgets::controls::slider::SliderRenderState,
    crate::widgets::controls::slider::render_slider_node,
    crate::widgets::controls::slider::measure_slider_node
    ; a11y: crate::widgets::controls::slider::emit_slider_accessibility
);
impl_widget_behavior!(
    crate::widgets::controls::stepper::StepperRenderState,
    crate::widgets::controls::stepper::render_stepper_node,
    crate::widgets::controls::stepper::measure_stepper_node
    ; a11y: crate::widgets::controls::stepper::emit_stepper_accessibility
);
impl_widget_behavior!(
    crate::widgets::controls::progress::ProgressRenderState,
    crate::widgets::controls::progress::render_progress_node,
    crate::widgets::controls::progress::measure_progress_node
    ; a11y: crate::widgets::controls::progress::emit_progress_accessibility
);
impl_widget_behavior!(
    crate::widgets::controls::button::MenuRenderState,
    crate::widgets::controls::button::render_menu_node,
    crate::widgets::controls::button::measure_menu_node
    ; prepare: ensure_label_built
    ; a11y: crate::widgets::controls::button::emit_menu_accessibility
);
impl_widget_behavior!(
    crate::widgets::controls::date_picker::DatePickerRenderState,
    crate::widgets::controls::date_picker::render_date_picker_node,
    crate::widgets::controls::date_picker::measure_date_picker_node
    ; a11y: crate::widgets::controls::date_picker::emit_date_picker_accessibility
);
impl_widget_behavior!(
    crate::widgets::controls::color_picker::ColorPickerRenderState,
    crate::widgets::controls::color_picker::render_color_picker_node,
    crate::widgets::controls::color_picker::measure_color_picker_node
    ; a11y: crate::widgets::controls::color_picker::emit_color_picker_accessibility
);
impl_widget_behavior!(
    crate::widgets::controls::picker::PickerRenderState,
    crate::widgets::controls::picker::render_picker_node,
    crate::widgets::controls::picker::measure_picker_node
    ; a11y: crate::widgets::controls::picker::emit_picker_accessibility
);
impl_widget_behavior!(
    crate::widgets::controls::text_field::TextFieldRenderState,
    crate::widgets::controls::text_field::render_text_field_node,
    crate::widgets::controls::text_field::measure_text_field_node
    ; a11y: crate::widgets::controls::text_field::emit_text_field_accessibility
);
impl_widget_behavior!(
    crate::widgets::controls::text_field::SecureFieldRenderState,
    crate::widgets::controls::text_field::render_secure_field_node,
    crate::widgets::controls::text_field::measure_secure_field_node
    ; a11y: crate::widgets::controls::text_field::emit_secure_field_accessibility
);
impl_widget_behavior!(
    crate::widgets::layout::badge::BadgeRenderState,
    crate::widgets::layout::badge::render_badge_node,
    crate::widgets::layout::badge::measure_badge_node
    ; a11y: crate::widgets::layout::badge::emit_badge_accessibility
);

impl_widget_behavior!(
    crate::widgets::platform::platform_view::PlatformViewRenderState,
    crate::widgets::platform::platform_view::render_platform_view_node,
    crate::widgets::platform::platform_view::measure_platform_view_node
);

impl RenderNode {
    /// Build a `Widget` node around its single shared state allocation.
    pub(super) fn build_widget<S>(
        renderer: &SemanticCore,
        state: Rc<S>,
        stretch: StretchAxis,
        env: &Environment,
    ) -> Self
    where
        S: WidgetBehavior + 'static,
    {
        Self::build_widget_with_core(state, stretch, env, renderer.new_core())
    }

    /// Build a `Widget` node around a caller-created core. `prebuild` sites
    /// make the core first and run the state's build-time reads under it as
    /// the record reader, so the subviews the prebuild materializes attach
    /// to this widget's cell rather than the window root (water-rs/waterui
    /// §A.2: the parent cell is pushed during `build`).
    pub(super) fn build_widget_with_core<S>(
        state: Rc<S>,
        stretch: StretchAxis,
        env: &Environment,
        core: NodeCore,
    ) -> Self
    where
        S: WidgetBehavior + 'static,
    {
        Self::Widget(WidgetNode {
            accessibility_identity: Rc::new(()),
            core,
            safe_area: None,
            behavior: state,
            stretch,
            fill_leaf: false,
            env: env.clone(),
        })
    }

    /// Build a platform-view placement leaf: the retained state is the factory
    /// key, a stable placement id and the session's `PlatformViewSink` — the
    /// sink lookup happens here so a runner that embeds no native views fails
    /// at build instead of at first flush.
    pub(super) fn build_platform_view(
        renderer: &SemanticCore,
        config: &crate::platform_view::PlatformView,
        env: &Environment,
    ) -> Self {
        use crate::widgets::platform::platform_view::PlatformViewRenderState;
        let state = Rc::new(RefCell::new(PlatformViewRenderState::from_config(
            config, env,
        )));
        Self::build_widget(renderer, state, StretchAxis::Both, env)
    }

    /// Build a persistent button node: retain the config behind an `Rc<RefCell<…>>`
    /// (its `Label` carries the live content signal; its action is invoked through the
    /// shared cell), and re-render it every flush so a reactive label stays live.
    pub(super) fn build_button(
        renderer: &SemanticCore,
        config: ButtonConfig,
        env: &Environment,
    ) -> Self {
        use crate::widgets::controls::button::ButtonRenderState;
        let mut state = ButtonRenderState::from_config(config);
        // Create the general label sub-view unstyled; the layout-time prepare
        // pass paints it with the theme and builds it before first measure.
        state.init_label();
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(renderer, state, StretchAxis::None, env)
    }

    /// Build a persistent toggle node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the cloneable config drives the control + accessibility, and its
    /// `toggle` binding is read through `resolve_toggle_progress` which watches it.
    ///
    /// The node's stretch follows the resolved label: a visible label claims
    /// the full row (`label … switch`), while a hidden label draws nothing
    /// and takes no space, so the toggle is content-sized — just the switch.
    /// A stretched hidden-label toggle would report a frame the label's dead
    /// zone inflates past the control, and taps inside it would hit nothing
    /// (water-rs/waterui#2241).
    pub(super) fn build_toggle(
        config: ToggleConfig,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        use crate::widgets::controls::toggle::ToggleRenderState;
        let stretch = if matches!(
            config.label.effective_display_mode(env),
            waterui_controls::label::LabelDisplayMode::Hidden
        ) {
            StretchAxis::None
        } else {
            waterui_core::NativeView::stretch_axis(&config)
        };
        let mut state = ToggleRenderState::from_config(config);
        // The prebuild materializes subviews and reads bindings: run it
        // under this widget's cell as the record reader so its reads
        // and the subviews' cells attach to the widget, not the root.
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild(renderer, env);
        });
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }

    /// Build a persistent slider node: its value-end labels are move-only
    /// `AnyView`s, so they are pre-built into [`RetainedSubview`]s (the measure
    /// path has only `&mut HydroState`, no renderer to build on); the `value`
    /// binding is read through `read_signal` so a change schedules a frame.
    pub(super) fn build_slider(
        config: SliderConfig,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        use crate::widgets::controls::slider::SliderRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = SliderRenderState::from_config(config);
        // The prebuild materializes subviews and reads bindings: run it
        // under this widget's cell as the record reader so its reads
        // and the subviews' cells attach to the widget, not the root.
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild_labels(renderer, env);
        });
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }

    /// Build a persistent stepper node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the cloneable config drives the buttons + accessibility, and its
    /// value/step signals are read through `read_signal` so a change schedules a frame.
    pub(super) fn build_stepper(
        config: StepperConfig,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        use crate::widgets::controls::stepper::StepperRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = StepperRenderState::from_config(config);
        // The prebuild materializes subviews and reads bindings: run it
        // under this widget's cell as the record reader so its reads
        // and the subviews' cells attach to the widget, not the root.
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild(renderer, env);
        });
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }

    /// Build a persistent progress node: its label/value labels are move-only
    /// `AnyView`s pre-built into [`RetainedSubview`]s; the `value` is read through
    /// `read_signal` so a change schedules a frame. Stretch is style-dependent
    /// (Linear → Horizontal, Circular → None), read from the config.
    pub(super) fn build_progress(
        config: ProgressConfig,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        use crate::widgets::controls::progress::ProgressRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = ProgressRenderState::from_config(config);
        // The prebuild materializes subviews and reads bindings: run it
        // under this widget's cell as the record reader so its reads
        // and the subviews' cells attach to the widget, not the root.
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild_labels(renderer, env);
        });
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }

    /// Build a persistent menu node: its trigger label is a move-only `AnyView`
    /// pre-built into a [`RetainedSubview`]; its `accessibility_label` and `items`
    /// signals are read through `read_signal` so a change schedules a frame.
    pub(super) fn build_menu(
        renderer: &SemanticCore,
        menu: ResolvedMenu,
        env: &Environment,
    ) -> Self {
        use crate::widgets::controls::button::MenuRenderState;
        let stretch = <ResolvedMenu as waterui_core::NativeView>::stretch_axis(&menu);
        // The label sub-view (created by `from_resolved`) is painted with the
        // theme and built by the layout-time prepare pass before first measure.
        let state = Rc::new(RefCell::new(MenuRenderState::from_resolved(menu, env)));
        Self::build_widget(renderer, state, stretch, env)
    }

    /// Build a persistent date-picker node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the cloneable config drives the field + accessibility, and its
    /// value is read through `read_signal` so a change schedules a frame. Stretch is
    /// content-sized (read from the config).
    pub(super) fn build_date_picker(
        config: DatePickerConfig,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        use crate::widgets::controls::date_picker::DatePickerRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = DatePickerRenderState::from_config(config);
        // The prebuild materializes subviews and reads bindings: run it
        // under this widget's cell as the record reader so its reads
        // and the subviews' cells attach to the widget, not the root.
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild(renderer, env);
        });
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }

    /// Build a persistent color-picker node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the cloneable config drives the swatch + accessibility, and its
    /// value is read through `read_signal` so a change schedules a frame. Stretch is
    /// content-sized (read from the config).
    pub(super) fn build_color_picker(
        config: ColorPickerConfig,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        use crate::widgets::controls::color_picker::ColorPickerRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = ColorPickerRenderState::from_config(config);
        // The prebuild materializes subviews and reads bindings: run it
        // under this widget's cell as the record reader so its reads
        // and the subviews' cells attach to the widget, not the root.
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild(renderer, env);
        });
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }

    /// Build a persistent picker node: its field label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no
    /// renderer to build on); the cloneable config drives the field +
    /// accessibility, and its `items`/`selection` signals are read through
    /// `read_signal` each frame so a membership or selection change schedules a
    /// frame. Stretch is content-sized (read from the config).
    pub(super) fn build_picker(
        config: PickerConfig,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        use crate::widgets::controls::picker::PickerRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = PickerRenderState::from_config(config);
        // The prebuild materializes subviews and reads bindings: run it
        // under this widget's cell as the record reader so its reads
        // and the subviews' cells attach to the widget, not the root.
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild(renderer, env);
        });
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }

    /// Build a persistent text-field node: its floating label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on) and re-flushed under the animated label transform each frame; the
    /// cloneable config's `prompt/value/selection_menu` are read each frame, with the
    /// value `Binding<StyledStr>` read through `read_signal` so typing or a binding
    /// change schedules a frame. The node re-runs the same text-input target
    /// registration each flush, so cursor/focus/IME state is preserved. Stretch is
    /// horizontal (read from the config).
    pub(super) fn build_text_field(
        config: ResolvedTextFieldConfig,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        use crate::widgets::controls::text_field::TextFieldRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = TextFieldRenderState::from_config(config);
        // The prebuild materializes subviews and reads bindings: run it
        // under this widget's cell as the record reader so its reads
        // and the subviews' cells attach to the widget, not the root.
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild(renderer, env);
        });
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }

    /// Build a persistent secure-field node: its floating label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on) and re-flushed under the animated label transform each frame; the
    /// cloneable config's `Binding<Secure>` value is read through `read_signal` each
    /// frame so typing or a binding change schedules a frame and the masked display
    /// updates. The node re-runs the same text-input target registration each flush,
    /// preserving cursor/focus/IME state. Stretch is horizontal (read from the config).
    pub(super) fn build_secure_field(
        config: SecureFieldConfig,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        use crate::widgets::controls::text_field::SecureFieldRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = SecureFieldRenderState::from_config(config);
        // The prebuild materializes subviews and reads bindings: run it
        // under this widget's cell as the record reader so its reads
        // and the subviews' cells attach to the widget, not the root.
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild(renderer, env);
        });
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }

    /// Build a persistent badge node: its wrapped content is a move-only `AnyView`
    /// pre-built into a [`RetainedSubview`]; the `value` is read through
    /// `read_signal` so a change schedules a frame. Badge sizes to its content and
    /// never stretches (`StretchAxis::None`, read from the config).
    pub(super) fn build_badge(
        config: BadgeConfig,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        use crate::widgets::layout::badge::BadgeRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = BadgeRenderState::from_config(config);
        // The prebuild materializes subviews and reads bindings: run it
        // under this widget's cell as the record reader so its reads
        // and the subviews' cells attach to the widget, not the root.
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild_content(renderer, env);
        });
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }
}
