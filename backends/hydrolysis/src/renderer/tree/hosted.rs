//! Hosted leaves use the same retained install layer as other producers.

#[allow(clippy::wildcard_imports)]
use super::*;

struct HostedNode {
    runtime: Rc<crate::hosted::HostedRuntime>,
}

impl HydroNativeView for Native<crate::HostedView> {
    fn intrinsic(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        LayoutSize::new(0.0, 0.0)
    }

    fn dimensions(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}

impl Drop for HostedNode {
    fn drop(&mut self) {
        self.runtime.content.unmount();
    }
}

impl WidgetBehavior for HostedNode {
    fn measure(
        &self,
        _state: &mut HydroState,
        proposal: ProposalSize,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }

    fn render(
        self: Rc<Self>,
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        _safe_area: Option<safe_area::SafeAreaLayout>,
    ) {
        let runtime = &self.runtime;
        renderer.read_signal(&runtime.focused);
        let key = InteractionKey::for_rc(runtime, 0);
        let (_, slot, _) =
            renderer.bind_control_interaction_target(key.clone(), ctx.bounds, env, false);
        let focus = Rc::clone(runtime);
        renderer.register_interactive_pointer_target(ctx.bounds, slot, move |_, _, _| {
            focus.content.request_focus();
            true
        });
        emit_graphics_image_accessibility(renderer, Some(ctx), env, None, None, true);
        renderer.register_retained(
            NativeViewOcclusion {
                bounds: ctx.bounds,
                order: 0,
                sink: runtime.occlusion.sink(),
                hosted: (key, Rc::clone(runtime)),
            },
            ctx.bounds,
            |regs| &mut regs.native_view_occlusions,
        );
        renderer.program().program_mut().producer = Some(mount::ProducerContent::Hosted {
            runtime: Rc::clone(runtime),
            bounds: ctx.bounds,
        });
    }

    #[cfg(feature = "accessibility")]
    fn emit_accessibility(self: Rc<Self>, renderer: &mut SemanticCore, env: &Environment) {
        emit_graphics_image_accessibility(renderer, None, env, None, None, true);
    }
}

impl RenderNode {
    pub(super) fn build_hosted(
        view: crate::hosted::HostedView,
        env: &Environment,
        renderer: &SemanticCore,
    ) -> Self {
        let state = Rc::new(HostedNode {
            runtime: crate::hosted::HostedRuntime::new(view),
        });
        Self::build_widget(renderer, state, StretchAxis::Both, env)
    }
}
