//! Backend-owned leaf realization through [`NativeView::realize_provided`].
//!
//! Leaves `hydrolysis` itself provides outside [`NativeBackend`]
//! (`PlatformView`, `HostedView`) claim [`HydroProvidedSlot`] inside their
//! [`NativeView::realize_provided`] implementation so
//! [`HydroBackend::provided`](super::build::backend::HydroBackend) and
//! `provided_dyn` carry no hardcoded downcasts for them and only constructed
//! leaves remain reachable.

#[allow(clippy::wildcard_imports)]
use super::*;

/// Non-generic render-pass operations a backend-provided leaf can perform
/// without naming `HydrolysisRenderer<E>`'s text engine `E`.
pub trait HydroProvidedRenderHost {
    fn register_platform_view_placement(
        &mut self,
        bounds: kurbo::Rect,
        table: Rc<RefCell<crate::platform_view::PlatformViewTable>>,
        placement: crate::platform_view::PlatformViewPlacement,
    );

    #[cfg(all(hydrolysis_hosted, hydrolysis_gpu))]
    fn register_hosted_surface(
        &mut self,
        runtime: Rc<crate::hosted::HostedRuntime>,
        ctx: RenderContext,
        env: &Environment,
    );
}

impl<E: crate::text::TextEngine> HydroProvidedRenderHost for HydrolysisRenderer<E> {
    fn register_platform_view_placement(
        &mut self,
        bounds: kurbo::Rect,
        table: Rc<RefCell<crate::platform_view::PlatformViewTable>>,
        placement: crate::platform_view::PlatformViewPlacement,
    ) {
        SemanticCore::register_platform_view_placement(self, bounds, table, placement);
    }

    #[cfg(all(hydrolysis_hosted, hydrolysis_gpu))]
    fn register_hosted_surface(
        &mut self,
        runtime: Rc<crate::hosted::HostedRuntime>,
        ctx: RenderContext,
        env: &Environment,
    ) {
        self.read_signal(&runtime.focused);
        let key = InteractionKey::for_rc(&runtime, 0);
        let (_, slot, _) =
            self.bind_control_interaction_target(key.clone(), ctx.bounds(), env, false);
        let focus = Rc::clone(&runtime);
        self.register_interactive_pointer_target(ctx.bounds(), slot, move |_, _, _| {
            focus.content.request_focus();
            true
        });
        emit_graphics_image_accessibility(self, Some(ctx), env, None, None, true);
        self.register_retained(
            NativeViewOcclusion {
                bounds: ctx.bounds(),
                order: 0,
                sink: runtime.occlusion.clone(),
                hosted: (key, Rc::clone(&runtime)),
            },
            ctx.bounds(),
            |regs| &mut regs.native_view_occlusions,
        );
        self.program().program_mut().producer = Some(mount::ProducerContent::Hosted {
            runtime,
            bounds: ctx.bounds(),
        });
    }
}

/// Non-generic accessibility-pass operations for backend-provided leaves.
#[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
pub trait HydroProvidedA11yHost {
    #[cfg_attr(not(all(hydrolysis_hosted, hydrolysis_gpu)), expect(dead_code))]
    fn emit_graphics_image_accessibility(&mut self, env: &Environment, focusable: bool);
}

#[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
impl<E: crate::text::TextEngine> HydroProvidedA11yHost for SemanticCore<E> {
    fn emit_graphics_image_accessibility(&mut self, env: &Environment, focusable: bool) {
        emit_graphics_image_accessibility(self, None, env, None, None, focusable);
    }
}

/// Non-generic widget behavior for one backend-owned leaf realized via
/// [`HydroProvidedSlot`], adapted to [`WidgetBehavior<E>`] in
/// [`HydroBackend`](super::build::backend::HydroBackend).
pub trait HydroProvidedBehavior {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions;

    fn render(
        self: Rc<Self>,
        host: &mut dyn HydroProvidedRenderHost,
        ctx: RenderContext,
        env: &Environment,
    );

    #[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
    fn emit_accessibility(self: Rc<Self>, host: &mut dyn HydroProvidedA11yHost, env: &Environment) {
        let _ = (host, env);
    }
}

/// Private receiver [`HydroBackend`](super::build::backend::HydroBackend)
/// passes into [`NativeView::realize_provided`] so backend-owned leaves
/// realize without naming them in `provided` / `provided_dyn` or naming `E`.
#[derive(Default)]
pub struct HydroProvidedSlot {
    /// When a leaf claims this slot and leaves `realized` as `None` (for
    /// example, a [`PlatformView`](crate::PlatformView) whose runner installed
    /// no [`PlatformViewSink`](crate::PlatformViewSink)), the backend builds
    /// an empty node.
    pub realized: Option<(Rc<dyn HydroProvidedBehavior>, StretchAxis)>,
}

struct HydroProvidedWidgetAdapter(Rc<dyn HydroProvidedBehavior>);

impl<E: crate::text::TextEngine> WidgetBehavior<E> for HydroProvidedWidgetAdapter {
    fn measure(
        &self,
        _state: &mut HydroState<E>,
        proposal: ProposalSize,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> ViewDimensions {
        self.0.measure(proposal)
    }

    fn render(
        self: Rc<Self>,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
        env: &Environment,
        _safe_area: Option<safe_area::SafeAreaLayout>,
    ) {
        Rc::clone(&self.0).render(renderer, ctx, env);
    }

    #[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
    fn emit_accessibility(self: Rc<Self>, renderer: &mut SemanticCore<E>, env: &Environment) {
        Rc::clone(&self.0).emit_accessibility(renderer, env);
    }
}

pub(super) fn realize_hydro_provided_slot<E: crate::text::TextEngine>(
    renderer: &SemanticCore<E>,
    slot: HydroProvidedSlot,
    env: &Environment,
) -> RenderNode<E> {
    let Some((behavior, stretch)) = slot.realized else {
        return RenderNode::build_empty(renderer, env);
    };
    RenderNode::build_widget(
        renderer,
        Rc::new(HydroProvidedWidgetAdapter(behavior)),
        stretch,
        env,
    )
}

#[cfg(all(hydrolysis_hosted, hydrolysis_gpu, hydrolysis_system_webview))]
impl<E: crate::text::TextEngine> RenderNode<E> {
    pub(super) fn build_hosted_retained(
        view: crate::hosted::HostedView,
        env: &Environment,
        renderer: &SemanticCore<E>,
        retained: impl core::any::Any,
    ) -> Self {
        let behavior = view.into_behavior_retained(retained);
        Self::build_widget(
            renderer,
            Rc::new(HydroProvidedWidgetAdapter(behavior)),
            StretchAxis::Both,
            env,
        )
    }
}
