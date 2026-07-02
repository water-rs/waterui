//! Widget-leaf builders for native controls (button, toggle, slider, stepper,
//! progress, menu, pickers, text fields, badge): each retains the config's
//! live signals in a [`WidgetNode`] re-dispatched every flush.

use super::*;

impl RenderNode {
    /// Build a `Widget` node from its per-flush render + measure closures and its
    /// stretch axis. The closures retain the widget's signal-holding config.
    pub(super) fn build_widget(
        render: WidgetRenderFn,
        measure: WidgetMeasureFn,
        stretch: StretchAxis,
        env: &Environment,
    ) -> RenderNode {
        RenderNode::Widget(Box::new(WidgetNode {
            render,
            measure,
            stretch,
            env: env.clone(),
        }))
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
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    crate::widgets::controls::button::render_button_node(
                        &mut widget_ctx,
                        &state,
                        env,
                    );
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    crate::widgets::controls::button::measure_button_node(
                        &state.borrow(),
                        proposal,
                        hydro,
                        env,
                    )
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, StretchAxis::None, env)
    }

    /// Build a persistent toggle node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the clonable config drives the control + accessibility, and its
    /// `toggle` binding is read through `resolve_toggle_progress` which watches it.
    pub(super) fn build_toggle(
        config: ToggleConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::toggle::{
            ToggleRenderState, measure_toggle_node, render_toggle_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = ToggleRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_toggle_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_toggle_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
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
        use crate::widgets::controls::slider::{
            SliderRenderState, measure_slider_node, render_slider_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = SliderRenderState::from_config(config);
        state.prebuild_labels(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_slider_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_slider_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent stepper node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the clonable config drives the buttons + accessibility, and its
    /// value/step signals are read through `read_signal` so a change schedules a frame.
    pub(super) fn build_stepper(
        config: StepperConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::stepper::{
            StepperRenderState, measure_stepper_node, render_stepper_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = StepperRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_stepper_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_stepper_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
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
        use crate::widgets::controls::progress::{
            ProgressRenderState, measure_progress_node, render_progress_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = ProgressRenderState::from_config(config);
        state.prebuild_labels(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_progress_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_progress_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent menu node: its trigger label is a move-only `AnyView`
    /// pre-built into a [`RetainedSubview`]; its `accessibility_label` and `items`
    /// signals are read through `read_signal` so a change schedules a frame.
    pub(super) fn build_menu(
        menu: ResolvedMenu,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::button::{
            MenuRenderState, measure_menu_node, render_menu_node,
        };
        let stretch = <ResolvedMenu as waterui_core::NativeView>::stretch_axis(&menu);
        let mut state = MenuRenderState::from_resolved(menu);
        state.prebuild_label(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_menu_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_menu_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent date-picker node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the clonable config drives the field + accessibility, and its
    /// value is read through `read_signal` so a change schedules a frame. Stretch is
    /// content-sized (read from the config).
    pub(super) fn build_date_picker(
        config: DatePickerConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::date_picker::{
            DatePickerRenderState, measure_date_picker_node, render_date_picker_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = DatePickerRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_date_picker_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_date_picker_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent color-picker node: its main label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on); the clonable config drives the swatch + accessibility, and its
    /// value is read through `read_signal` so a change schedules a frame. Stretch is
    /// content-sized (read from the config).
    pub(super) fn build_color_picker(
        config: ColorPickerConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::color_picker::{
            ColorPickerRenderState, measure_color_picker_node, render_color_picker_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = ColorPickerRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_color_picker_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_color_picker_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent picker node: retain the config (its `items`/`selection`
    /// signals are read through `read_signal` each frame so a membership or selection
    /// change schedules a frame). Stretch is content-sized (read from the config).
    pub(super) fn build_picker(config: PickerConfig, env: &Environment) -> RenderNode {
        use crate::widgets::controls::picker::{measure_picker_node, render_picker_node};
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let config = Rc::new(RefCell::new(config));
        // Node-owned menu open/closed state: it persists across frames as a field of
        // this node's render closure, not in a renderer-global, flush-order-indexed
        // slot pool. The renderer keeps only an Rc-pruned registry of these handles so
        // an outside click can dismiss every open menu; a dropped picker's handle falls
        // out of the registry by strong count.
        let menu_open = Rc::new(Cell::new(false));
        let render = {
            let config = Rc::clone(&config);
            let menu_open = Rc::clone(&menu_open);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_picker_node(&mut widget_ctx, &config, &menu_open, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let config = Rc::clone(&config);
            Box::new(
                move |state: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_picker_node(&config.borrow(), proposal, state, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent text-field node: its floating label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on) and re-flushed under the animated label transform each frame; the
    /// clonable config's prompt/value/selection_menu are read each frame, with the
    /// value `Binding<StyledStr>` read through `read_signal` so typing or a binding
    /// change schedules a frame. The node re-runs the same text-input target
    /// registration each flush, so cursor/focus/IME state is preserved. Stretch is
    /// horizontal (read from the config).
    pub(super) fn build_text_field(
        config: ResolvedTextFieldConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::text_field::{
            TextFieldRenderState, measure_text_field_node, render_text_field_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = TextFieldRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_text_field_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_text_field_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }

    /// Build a persistent secure-field node: its floating label is pre-built into a
    /// [`RetainedSubview`] (the measure path has only `&mut HydroState`, no renderer
    /// to build on) and re-flushed under the animated label transform each frame; the
    /// clonable config's `Binding<Secure>` value is read through `read_signal` each
    /// frame so typing or a binding change schedules a frame and the masked display
    /// updates. The node re-runs the same text-input target registration each flush,
    /// preserving cursor/focus/IME state. Stretch is horizontal (read from the config).
    pub(super) fn build_secure_field(
        config: SecureFieldConfig,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        use crate::widgets::controls::text_field::{
            SecureFieldRenderState, measure_secure_field_node, render_secure_field_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = SecureFieldRenderState::from_config(config);
        state.prebuild(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_secure_field_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_secure_field_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
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
        use crate::widgets::layout::badge::{
            BadgeRenderState, measure_badge_node, render_badge_node,
        };
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let mut state = BadgeRenderState::from_config(config);
        state.prebuild_content(renderer, env);
        let state = Rc::new(RefCell::new(state));
        let render = {
            let state = Rc::clone(&state);
            Box::new(
                move |renderer: &mut HydrolysisRenderer, ctx: RenderContext, env: &Environment| {
                    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
                    render_badge_node(&mut widget_ctx, &state, env);
                },
            ) as Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &Environment)>
        };
        let measure = {
            let state = Rc::clone(&state);
            Box::new(
                move |hydro: &mut HydroState, proposal: ProposalSize, env: &Environment| {
                    measure_badge_node(&state.borrow(), proposal, hydro, env)
                },
            )
                as Box<dyn Fn(&mut HydroState, ProposalSize, &Environment) -> ViewDimensions>
        };
        Self::build_widget(render, measure, stretch, env)
    }
}
