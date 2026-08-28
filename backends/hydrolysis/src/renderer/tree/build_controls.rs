//! Widget-leaf builders for native controls (button, toggle, slider, stepper,
//! progress, menu, pickers, text fields, badge): each retains the config's
//! live signals in a [`WidgetNode`] re-dispatched every flush.

use super::*;

impl_widget_behavior!(
    crate::widgets::controls::button::ButtonRenderState,
    crate::widgets::controls::button::render_button_node,
    crate::widgets::controls::button::measure_button_node
);
impl_widget_behavior!(
    crate::widgets::controls::toggle::ToggleRenderState,
    crate::widgets::controls::toggle::render_toggle_node,
    crate::widgets::controls::toggle::measure_toggle_node
);
impl_widget_behavior!(
    crate::widgets::controls::slider::SliderRenderState,
    crate::widgets::controls::slider::render_slider_node,
    crate::widgets::controls::slider::measure_slider_node
);
impl_widget_behavior!(
    crate::widgets::controls::stepper::StepperRenderState,
    crate::widgets::controls::stepper::render_stepper_node,
    crate::widgets::controls::stepper::measure_stepper_node
);
impl_widget_behavior!(
    crate::widgets::controls::progress::ProgressRenderState,
    crate::widgets::controls::progress::render_progress_node,
    crate::widgets::controls::progress::measure_progress_node
);
impl_widget_behavior!(
    crate::widgets::controls::button::MenuRenderState,
    crate::widgets::controls::button::render_menu_node,
    crate::widgets::controls::button::measure_menu_node
);
impl_widget_behavior!(
    crate::widgets::controls::date_picker::DatePickerRenderState,
    crate::widgets::controls::date_picker::render_date_picker_node,
    crate::widgets::controls::date_picker::measure_date_picker_node
);
impl_widget_behavior!(
    crate::widgets::controls::color_picker::ColorPickerRenderState,
    crate::widgets::controls::color_picker::render_color_picker_node,
    crate::widgets::controls::color_picker::measure_color_picker_node
);
impl_widget_behavior!(
    crate::widgets::controls::picker::PickerRenderState,
    crate::widgets::controls::picker::render_picker_node,
    crate::widgets::controls::picker::measure_picker_node
);
impl_widget_behavior!(
    crate::widgets::controls::text_field::TextFieldRenderState,
    crate::widgets::controls::text_field::render_text_field_node,
    crate::widgets::controls::text_field::measure_text_field_node
);
impl_widget_behavior!(
    crate::widgets::controls::text_field::SecureFieldRenderState,
    crate::widgets::controls::text_field::render_secure_field_node,
    crate::widgets::controls::text_field::measure_secure_field_node
);
impl_widget_behavior!(
    crate::widgets::layout::badge::BadgeRenderState,
    crate::widgets::layout::badge::render_badge_node,
    crate::widgets::layout::badge::measure_badge_node
);

impl RenderNode {
    /// Build a `Widget` node around its single shared state allocation.
    pub(super) fn build_widget<S>(
        state: Rc<S>,
        stretch: StretchAxis,
        env: &Environment,
    ) -> RenderNode
    where
        S: WidgetBehavior + 'static,
    {
        RenderNode::Widget(WidgetNode {
            #[cfg(feature = "accessibility")]
            accessibility_identity: Rc::new(()),
            behavior: state,
            stretch,
            env: env.clone(),
        })
    }

    /// Build a persistent button node: retain the config behind an `Rc<RefCell<…>>`
    /// (its `Label` carries the live content signal; its action is invoked through the
    /// shared cell), and re-render it every flush so a reactive label stays live.
    pub(super) fn build_button(
        config: ButtonConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::button::ButtonRenderState;
        let mut state = ButtonRenderState::from_config(config);
        // Pre-build the general label sub-view (the measure path has no renderer).
        state.prebuild_label(renderer, env);
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(state, StretchAxis::None, env)
    }

    /// Build a persistent toggle node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the cloneable config drives the control + accessibility, and its
    /// `toggle` binding is read through `resolve_toggle_progress` which watches it.
    pub(super) fn build_toggle(
        config: ToggleConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::toggle::ToggleRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = ToggleRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(state, stretch, env)
    }

    /// Build a persistent slider node: its value-end labels are move-only
    /// `AnyView`s, so they are pre-built into [`RetainedSubview`]s (the measure
    /// path has only `&mut HydroState`, no renderer to build on); the `value`
    /// binding is read through `read_signal` so a change schedules a frame.
    pub(super) fn build_slider(
        config: SliderConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::slider::SliderRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = SliderRenderState::from_config(config);
        state.prebuild_labels(renderer, env);
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(state, stretch, env)
    }

    /// Build a persistent stepper node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the cloneable config drives the buttons + accessibility, and its
    /// value/step signals are read through `read_signal` so a change schedules a frame.
    pub(super) fn build_stepper(
        config: StepperConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::stepper::StepperRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = StepperRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(state, stretch, env)
    }

    /// Build a persistent progress node: its label/value labels are move-only
    /// `AnyView`s pre-built into [`RetainedSubview`]s; the `value` is read through
    /// `read_signal` so a change schedules a frame. Stretch is style-dependent
    /// (Linear → Horizontal, Circular → None), read from the config.
    pub(super) fn build_progress(
        config: ProgressConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::progress::ProgressRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = ProgressRenderState::from_config(config);
        state.prebuild_labels(renderer, env);
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(state, stretch, env)
    }

    /// Build a persistent menu node: its trigger label is a move-only `AnyView`
    /// pre-built into a [`RetainedSubview`]; its `accessibility_label` and `items`
    /// signals are read through `read_signal` so a change schedules a frame.
    pub(super) fn build_menu(
        menu: ResolvedMenu,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::button::MenuRenderState;
        let stretch = <ResolvedMenu as waterui_core::NativeView>::stretch_axis(&menu);
        let mut state = MenuRenderState::from_resolved(menu);
        state.prebuild_label(renderer, env);
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(state, stretch, env)
    }

    /// Build a persistent date-picker node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the cloneable config drives the field + accessibility, and its
    /// value is read through `read_signal` so a change schedules a frame. Stretch is
    /// content-sized (read from the config).
    pub(super) fn build_date_picker(
        config: DatePickerConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::date_picker::DatePickerRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = DatePickerRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(state, stretch, env)
    }

    /// Build a persistent color-picker node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the cloneable config drives the swatch + accessibility, and its
    /// value is read through `read_signal` so a change schedules a frame. Stretch is
    /// content-sized (read from the config).
    pub(super) fn build_color_picker(
        config: ColorPickerConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::color_picker::ColorPickerRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = ColorPickerRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(state, stretch, env)
    }

    /// Build a persistent picker node: retain the config (its `items`/`selection`
    /// signals are read through `read_signal` each frame so a membership or selection
    /// change schedules a frame). Stretch is content-sized (read from the config).
    pub(super) fn build_picker(config: PickerConfig, env: &Environment) -> RenderNode {
        use crate::widgets::controls::picker::PickerRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let state = Rc::new(RefCell::new(PickerRenderState::new(config)));
        Self::build_widget(state, stretch, env)
    }

    /// Build a persistent text-field node: its floating label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on) and re-flushed under the animated label transform each frame; the
    /// cloneable config's prompt/value/selection_menu are read each frame, with the
    /// value `Binding<StyledStr>` read through `read_signal` so typing or a binding
    /// change schedules a frame. The node re-runs the same text-input target
    /// registration each flush, so cursor/focus/IME state is preserved. Stretch is
    /// horizontal (read from the config).
    pub(super) fn build_text_field(
        config: ResolvedTextFieldConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::text_field::TextFieldRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = TextFieldRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(state, stretch, env)
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
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::text_field::SecureFieldRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = SecureFieldRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(state, stretch, env)
    }

    /// Build a persistent badge node: its wrapped content is a move-only `AnyView`
    /// pre-built into a [`RetainedSubview`]; the `value` is read through
    /// `read_signal` so a change schedules a frame. Badge sizes to its content and
    /// never stretches (`StretchAxis::None`, read from the config).
    pub(super) fn build_badge(
        config: BadgeConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::layout::badge::BadgeRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = BadgeRenderState::from_config(config);
        state.prebuild_content(renderer, env);
        let state = Rc::new(RefCell::new(state));
        Self::build_widget(state, stretch, env)
    }
}
