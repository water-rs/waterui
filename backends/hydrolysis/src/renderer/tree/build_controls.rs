//! Widget-leaf builders for native controls (button, toggle, slider, stepper,
//! progress, menu, pickers, text fields, badge): each retains the config's
//! live signals in a [`WidgetNode`] re-dispatched every flush.

#[cfg(feature = "link")]
use crate::widgets::controls::link::LinkRenderState;
// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
#[cfg(feature = "link")]
use waterui_controls::link::LinkConfig;

#[cfg(waterui_feature = "avatar")]
impl_widget_behavior!(
    crate::widgets::controls::avatar::AvatarRenderState<E>,
    crate::widgets::controls::avatar::render_avatar_node,
    crate::widgets::controls::avatar::measure_avatar_node
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::controls::avatar::emit_avatar_accessibility
    ; label: contributed_label
);
impl_widget_behavior!(
    crate::widgets::controls::button::ButtonRenderState<E>,
    crate::widgets::controls::button::render_button_node,
    crate::widgets::controls::button::measure_button_node
    ; prepare: ensure_label_built
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::controls::button::emit_button_accessibility
    ; label: contributed_label
);
#[cfg(feature = "link")]
impl_widget_behavior!(
    LinkRenderState<E>,
    crate::widgets::controls::link::render_link_node,
    crate::widgets::controls::link::measure_link_node
    ; prepare: ensure_label_built
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::controls::link::emit_link_accessibility
    ; label: contributed_label
);
impl_widget_behavior!(
    crate::widgets::controls::disclosure::DisclosureRenderState<E>,
    crate::widgets::controls::disclosure::render_disclosure_node,
    crate::widgets::controls::disclosure::measure_disclosure_node
    ; prepare: prepare
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::controls::disclosure::emit_disclosure_accessibility
    ; label: contributed_label
);
#[cfg(feature = "toggle")]
impl_widget_behavior!(
    crate::widgets::controls::toggle::ToggleRenderState<E>,
    crate::widgets::controls::toggle::render_toggle_node,
    crate::widgets::controls::toggle::measure_toggle_node
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::controls::toggle::emit_toggle_accessibility
    ; label: contributed_label
);
#[cfg(feature = "slider")]
impl_widget_behavior!(
    crate::widgets::controls::slider::SliderRenderState<E>,
    crate::widgets::controls::slider::render_slider_node,
    crate::widgets::controls::slider::measure_slider_node
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::controls::slider::emit_slider_accessibility
    ; label: contributed_label
);
#[cfg(feature = "stepper")]
impl_widget_behavior!(
    crate::widgets::controls::stepper::StepperRenderState<E>,
    crate::widgets::controls::stepper::render_stepper_node,
    crate::widgets::controls::stepper::measure_stepper_node
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::controls::stepper::emit_stepper_accessibility
    ; label: contributed_label
);
impl_widget_behavior!(
    crate::widgets::controls::progress::ProgressRenderState<E>,
    crate::widgets::controls::progress::render_progress_node,
    crate::widgets::controls::progress::measure_progress_node
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::controls::progress::emit_progress_accessibility
    ; label: contributed_label
);
#[cfg(feature = "menu")]
impl_widget_behavior!(
    crate::widgets::controls::button::MenuRenderState<E>,
    crate::widgets::controls::button::render_menu_node,
    crate::widgets::controls::button::measure_menu_node
    ; prepare: ensure_label_built
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::controls::button::emit_menu_accessibility
    ; label: contributed_label
);
#[cfg(feature = "date-picker")]
impl_widget_behavior!(
    crate::widgets::controls::date_picker::DatePickerRenderState<E>,
    crate::widgets::controls::date_picker::render_date_picker_node,
    crate::widgets::controls::date_picker::measure_date_picker_node
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::controls::date_picker::emit_date_picker_accessibility
    ; label: contributed_label
);
#[cfg(feature = "color-picker")]
impl_widget_behavior!(
    crate::widgets::controls::color_picker::ColorPickerRenderState<E>,
    crate::widgets::controls::color_picker::render_color_picker_node,
    crate::widgets::controls::color_picker::measure_color_picker_node
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::controls::color_picker::emit_color_picker_accessibility
    ; label: contributed_label
);
#[cfg(feature = "picker")]
impl_widget_behavior!(
    crate::widgets::controls::picker::PickerRenderState<E>,
    crate::widgets::controls::picker::render_picker_node,
    crate::widgets::controls::picker::measure_picker_node
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::controls::picker::emit_picker_accessibility
    ; label: contributed_label
);
#[cfg(feature = "text-field")]
impl_widget_behavior!(
    crate::widgets::controls::text_field::TextFieldRenderState<E>,
    crate::widgets::controls::text_field::render_text_field_node,
    crate::widgets::controls::text_field::measure_text_field_node
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::controls::text_field::emit_text_field_accessibility
    ; label: contributed_label
);
#[cfg(feature = "secure-field")]
impl_widget_behavior!(
    crate::widgets::controls::text_field::SecureFieldRenderState<E>,
    crate::widgets::controls::text_field::render_secure_field_node,
    crate::widgets::controls::text_field::measure_secure_field_node
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::controls::text_field::emit_secure_field_accessibility
    ; label: contributed_label
);
impl_widget_behavior!(
    crate::widgets::layout::badge::BadgeRenderState<E>,
    crate::widgets::layout::badge::render_badge_node,
    crate::widgets::layout::badge::measure_badge_node
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::layout::badge::emit_badge_accessibility
);
impl_widget_behavior!(
    crate::widgets::layout::card::CardRenderState<E>,
    crate::widgets::layout::card::render_card_node,
    crate::widgets::layout::card::measure_card_node
    ; prepare: prepare
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::layout::card::emit_card_accessibility
    ; label: contributed_label
);
#[cfg(waterui_feature = "highlight")]
impl_widget_behavior!(
    crate::widgets::layout::code::CodeRenderState<E>,
    crate::widgets::layout::code::render_code_node,
    crate::widgets::layout::code::measure_code_node
    ; prepare: prepare
    ; dirty: take_layout_dirty
    ; a11y: crate::widgets::layout::code::emit_code_accessibility
    ; label: contributed_label
);

#[cfg(hydrolysis_android_system_webview)]
impl_widget_behavior!(
    crate::widgets::platform::platform_view::PlatformViewRenderState,
    crate::widgets::platform::platform_view::render_platform_view_node,
    crate::widgets::platform::platform_view::measure_platform_view_node
);

impl<E: crate::text::TextEngine> RenderNode<E> {
    /// Build a `Widget` node around its single shared state allocation.
    pub(super) fn build_widget<S>(
        renderer: &SemanticCore<E>,
        state: Rc<S>,
        stretch: StretchAxis,
        env: &Environment,
    ) -> Self
    where
        S: WidgetBehavior<E> + 'static,
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
        core: NodeCore<E>,
    ) -> Self
    where
        S: WidgetBehavior<E> + 'static,
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

    /// Build a persistent button node: retain the config behind an `Rc<RefCell<…>>`
    /// (its `Label` carries the live content signal; its action is invoked through the
    /// shared cell), and re-render it every flush so a reactive label stays live.
    pub(super) fn build_button(
        renderer: &SemanticCore<E>,
        config: ButtonConfig<HeldChild<E>>,
        env: &Environment,
    ) -> Self {
        use crate::widgets::controls::button::ButtonRenderState;
        let mut state = ButtonRenderState::from_config(config);
        // Create the general label sub-view unstyled; the layout-time prepare
        // pass paints it with the theme and builds it before first measure.
        state.init_label();
        let state = Rc::new(RefCell::new(state));
        // Interactive button labels must never hijack pointer clicks for text
        // selection even when placed inside an enclosing `.text_selectable(true)`.
        let button_env = env.extending(waterui_text::TextSelectable::new(false));
        Self::build_widget(renderer, state, StretchAxis::None, &button_env)
    }

    /// Build a persistent link node: retains the label and reactive destination
    /// URL behind an `Rc<RefCell<…>>` and renders with link chrome, pointer cursor,
    /// and link accessibility semantics.
    #[cfg(feature = "link")]
    pub(super) fn build_link(
        renderer: &SemanticCore<E>,
        config: LinkConfig<HeldChild<E>>,
        env: &Environment,
    ) -> Self {
        let mut state = LinkRenderState::from_config(config);
        state.init_label();
        let state = Rc::new(RefCell::new(state));
        // Interactive link labels must never hijack pointer clicks for text
        // selection even when placed inside an enclosing `.text_selectable(true)`.
        let link_env = env.extending(waterui_text::TextSelectable::new(false));
        Self::build_widget(renderer, state, StretchAxis::None, &link_env)
    }

    /// Build a persistent toggle node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the cloneable config drives the control + accessibility, and its
    /// `toggle` binding is watched by the toggle's live parts.
    ///
    /// The node's stretch is the resolved payload's own axis: a switch with a
    /// visible label claims the full row (`label … switch`), while a checkbox
    /// or a hidden label is content-sized. A stretched hidden-label toggle
    /// would report a frame the label's dead zone inflates past the control,
    /// and taps inside it would hit nothing (water-rs/waterui#2241).
    #[cfg(feature = "toggle")]
    pub(super) fn build_toggle(
        config: ToggleConfig<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
    ) -> Self {
        use crate::widgets::controls::toggle::ToggleRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
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

    /// Build a persistent slider node: its value-end labels are held child
    /// slots pre-built into [`RetainedSubview`]s (the measure path has only
    /// `&mut HydroState`, no renderer to build on); the `value` binding is
    /// read through `read_signal` so a change schedules a frame.
    #[cfg(feature = "slider")]
    pub(super) fn build_slider(
        config: SliderConfig<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
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
    ///
    /// The node's stretch is the resolved payload's own axis: a stepper with a
    /// visible label claims the full row (`label … buttons`), while a hidden
    /// label is content-sized — a stretched hidden-label stepper would report
    /// a frame inflated past the buttons that accepts no input
    /// (water-rs/waterui#2364).
    #[cfg(feature = "stepper")]
    pub(super) fn build_stepper(
        config: StepperConfig<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
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

    /// Build a persistent progress node: its label/value labels are held child
    /// slots pre-built into [`RetainedSubview`]s; the `value` is read through
    /// `read_signal` so a change schedules a frame. Stretch is style-dependent
    /// (Linear → Horizontal, Circular → None), read from the config.
    pub(super) fn build_progress(
        config: ProgressConfig<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
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

    /// Build a persistent menu node: its trigger label is a held child slot
    /// pre-built into a [`RetainedSubview`]; its `accessibility_label` and `items`
    /// signals are read through `read_signal` so a change schedules a frame.
    #[cfg(feature = "menu")]
    pub(super) fn build_menu(
        renderer: &SemanticCore<E>,
        menu: ResolvedMenu<HeldChild<E>>,
        env: &Environment,
    ) -> Self {
        use crate::widgets::controls::button::MenuRenderState;
        let stretch = <ResolvedMenu<HeldChild<E>> as waterui_core::NativeView>::stretch_axis(&menu);
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
    #[cfg(feature = "date-picker")]
    pub(super) fn build_date_picker(
        config: DatePickerConfig<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
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
    #[cfg(feature = "color-picker")]
    pub(super) fn build_color_picker(
        config: ColorPickerConfig<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
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
    /// frame. The node's stretch is the payload's own axis: `Horizontal` for a
    /// segmented picker, content-sized otherwise.
    #[cfg(feature = "picker")]
    pub(super) fn build_picker(
        config: PickerConfig<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
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
    #[cfg(feature = "text-field")]
    pub(super) fn build_text_field(
        config: TextFieldConfig<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
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
    #[cfg(feature = "secure-field")]
    pub(super) fn build_secure_field(
        config: SecureFieldConfig<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
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

    /// Build a persistent badge node: its wrapped content is a held child slot
    /// pre-built into a [`RetainedSubview`]; the `value` is read through
    /// `read_signal` so a change schedules a frame. Badge sizes to its content and
    /// never stretches (`StretchAxis::None`, read from the config).
    pub(super) fn build_badge(
        config: BadgeConfig<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
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

    /// Build a persistent code block node styled from `WidgetTheme::code`
    /// during prepare and tracking the installed `Body` font slot.
    #[cfg(waterui_feature = "highlight")]
    pub(super) fn build_code(
        config: waterui_text::code::CodeConfig,
        env: &Environment,
        renderer: &SemanticCore<E>,
    ) -> Self {
        use crate::widgets::layout::code::CodeRenderState;
        let state = CodeRenderState::from_config(config);
        let stretch = state.stretch_axis();
        Self::build_widget(renderer, Rc::new(RefCell::new(state)), stretch, env)
    }

    /// Build a persistent card node: its body content is pre-built eagerly so
    /// the card inherits its content's stretch axis, while optional title and
    /// subtitle text views are styled from `WidgetTheme::card` during prepare.
    pub(super) fn build_card(
        config: waterui_controls::card::CardConfig<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
    ) -> Self {
        use crate::widgets::layout::card::CardRenderState;
        let mut state = CardRenderState::from_config(config);
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild(renderer, env);
        });
        let stretch = state.stretch_axis();
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }

    /// Build a persistent avatar node: custom fallback and image sub-views are
    /// pre-built eagerly so layout measurements and accessibility suppression work
    /// on first flush.
    #[cfg(waterui_feature = "avatar")]
    pub(super) fn build_avatar(
        config: waterui_controls::avatar::AvatarConfig<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
    ) -> Self {
        use crate::widgets::controls::avatar::AvatarRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = AvatarRenderState::from_config(config);
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild(renderer, env);
        });
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }

    /// Build a persistent disclosure node: its header label is pre-built
    /// eagerly, while its expandable body is created lazily only when expanded.
    pub(super) fn build_disclosure(
        config: waterui_controls::disclosure::DisclosureConfig<HeldChild<E>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
    ) -> Self {
        use crate::widgets::controls::disclosure::DisclosureRenderState;
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = DisclosureRenderState::from_config(config);
        let core = renderer.new_core();
        renderer.with_probe_reader(&core, ReaderPhase::Record, |renderer| {
            state.prebuild(renderer, env);
        });
        let state = Rc::new(RefCell::new(state));
        Self::build_widget_with_core(state, stretch, env, core)
    }
}
