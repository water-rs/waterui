//! The HWUI layer target: the mount's commit seam for the Android
//! `RenderNode` backend. Primitive, text and picture recording are fully
//! supported; GPU-content, filtered and material capabilities are named
//! refusals at install, matching `Encoder::refuse`'s policy — the issues
//! that carry each capability are named in the panic.

use std::convert::Infallible;
use std::rc::Rc;

use cherenkov_record::{FilterId, Layer, MaterialRegistry, Transaction};
use waterui_graphics::Reactive;
use waterui_graphics::filtrate::filters::OverscrollStretch;

#[cfg(hydrolysis_gpu)]
use crate::gpu_view::{ExternalFrameRuntime, GpuContentRuntime};
use crate::hwui::HwuiTarget;
#[cfg(hydrolysis_gpu)]
use crate::renderer::ProducerWake;
use crate::renderer::effects::FilteredRuntime;
use crate::renderer::mount::backdrop::{
    BackdropGroupKey, ChromeBackdropGroups, ChromeGroupKey, ChromeMemberPayload,
    MaterialBackdropGroups, MaterialMembership, MemberBind,
};
use crate::renderer::mount::target::{FilterHandle, LayerTarget, MaterialTerms};
use crate::renderer::recording::SceneResources;

/// The HWUI target's host: the session's shared resource tables and the
/// theme's material registry. No engine handles — `Shader` is `Infallible`.
#[derive(Debug)]
pub struct HwuiHost {
    resources: Rc<SceneResources>,
    materials: MaterialTerms<Infallible>,
}

impl HwuiHost {
    /// A host whose registered resources live in `backend` and whose
    /// material registry is `registry` (empty `shaders` — the mount's
    /// attach assertion requires an empty registry for this target).
    #[expect(
        clippy::zero_sized_map_values,
        reason = "the target has no shader handles: Infallible makes this shared registry map uninhabited"
    )]
    pub fn new(backend: Rc<crate::hwui::HwuiResources>, registry: Rc<MaterialRegistry>) -> Self {
        Self {
            resources: Rc::new(SceneResources::over(
                waterui_graphics::SceneResources::with_shaders(Rc::clone(&backend), backend),
            )),
            materials: MaterialTerms {
                registry,
                shaders: rustc_hash::FxHashMap::default(),
            },
        }
    }
}

impl<E: crate::text::TextEngine> LayerTarget<E> for HwuiTarget {
    type Host = HwuiHost;
    type Group = Infallible;
    type Shader = Infallible;

    const BACKDROP_SHADERS: bool = false;

    fn resources(host: &Self::Host) -> &Rc<SceneResources> {
        &host.resources
    }

    #[cfg(hydrolysis_gpu)]
    fn mount_gpu_content(
        _host: &Self::Host,
        _tx: &mut Transaction<'_, Self>,
        _layer: &Layer,
        _runtime: &mut GpuContentRuntime,
        _pixels: (u32, u32),
        _wake: ProducerWake,
    ) {
        panic!(
            "hydrolysis HWUI: GpuContentView requires the hosted GPU-content realization \
             (water-rs/waterui#1812)"
        );
    }

    #[cfg(hydrolysis_gpu)]
    fn mount_external_frame(
        _host: &Self::Host,
        _tx: &mut Transaction<'_, Self>,
        _layer: &Layer,
        _runtime: &mut ExternalFrameRuntime,
        _wake: ProducerWake,
    ) -> Option<(u32, u32)> {
        panic!(
            "hydrolysis HWUI: ExternalFrameView requires the hosted GPU-content realization \
             (water-rs/waterui#1812)"
        );
    }

    fn filter(_host: &Self::Host, _runtime: &mut FilteredRuntime) -> FilterId {
        panic!(
            "hydrolysis HWUI: FilteredView requires native RenderEffect lowering (water-rs/waterui#1750)"
        );
    }

    fn stretch_filter(_host: &Self::Host, _filter: OverscrollStretch<Reactive>) -> FilterHandle {
        panic!(
            "hydrolysis HWUI: overscroll stretch requires native RenderEffect lowering (water-rs/waterui#1750)"
        );
    }

    fn mount_material(
        _host: &Self::Host,
        _tx: &mut Transaction<'_, Self>,
        _groups: &mut MaterialBackdropGroups<Self::Group, E>,
        _layer: &Layer,
        _key: BackdropGroupKey,
        _display_scale: f64,
        _membership: &MaterialMembership<E>,
        _bind: MemberBind,
    ) {
        panic!(
            "hydrolysis HWUI: Material requires the HWUI backdrop realization (water-rs/waterui#1899)"
        );
    }

    fn material_terms(host: &Self::Host) -> &MaterialTerms<Self::Shader> {
        &host.materials
    }

    fn mount_chrome(
        _host: &Self::Host,
        _tx: &mut Transaction<'_, Self>,
        _groups: &mut ChromeBackdropGroups<Self::Group, Self::Shader, E>,
        _layer: &Layer,
        _key: ChromeGroupKey,
        _params: cherenkov_record::MaterialCapture,
        _display_scale: f64,
        _membership: &MaterialMembership<E>,
        _payload: impl FnOnce() -> ChromeMemberPayload<Self::Shader>,
        _bind: MemberBind,
    ) {
        panic!(
            "hydrolysis HWUI: material chrome requires HWUI backdrop shaders (water-rs/waterui#1899)"
        );
    }
}

use crate::hwui::{HwuiResources, HwuiSurface};
use crate::renderer::HydrolysisRenderer;
use crate::renderer::mount::Mount;

#[cfg_attr(
    hydrolysis_gpu_painter,
    expect(
        dead_code,
        reason = "the Android HWUI runner is the only caller; test builds keep the methods alongside the mount's contract surface"
    )
)]
impl<E: crate::text::TextEngine> HydrolysisRenderer<E> {
    /// The mount and host of `surface`: the HWUI peer of the compositor's
    /// window mount, built against the renderer's material registry.
    pub(crate) fn mount_hwui(
        &self,
        surface: &HwuiSurface,
        resources: Rc<HwuiResources>,
    ) -> (Mount<HwuiTarget, E>, HwuiHost) {
        let host = HwuiHost::new(resources, Rc::clone(&self.material_registry));
        let mount = Mount::new(Rc::clone(surface.shared()), &self.material_registry);
        (mount, host)
    }

    /// The platform views staged this frame, as `(layer, view, bounds)`
    /// bindings the surface binds into their owner's `RenderNode`.
    /// `out` is cleared and refilled — expired owners simply drop out.
    pub(crate) fn platform_view_bindings(&self, out: &mut Vec<crate::hwui::PlatformViewBinding>) {
        out.clear();
        for staged in &self.core.materialized_platform_views {
            let Some(owner) = staged.owner.upgrade() else {
                continue;
            };
            let retained = owner.retained();
            let borrowed = retained.layers.borrow();
            let Some(layers) = borrowed.as_ref() else {
                continue;
            };
            out.push(crate::hwui::PlatformViewBinding::new(
                layers.frame().id(),
                staged.placement.id(),
                staged.local,
            ));
        }
    }

    /// The platform view that owns a touch at host pixel `point`: the
    /// topmost view `encoder` draws there, unless an interactive target
    /// painted above it covers the point — the rule every backend applies
    /// to embedded native views. `scale` is the density the touch edge
    /// divides host pixels by.
    ///
    /// # Errors
    ///
    /// A [`crate::hwui::HwuiError`] when a layer's projective pose is
    /// degenerate.
    pub(crate) fn platform_view_hit(
        &self,
        encoder: &crate::hwui::Encoder,
        point: kurbo::Point,
        scale: f64,
    ) -> Result<Option<u64>, crate::hwui::HwuiError> {
        let view = encoder.platform_view_at(point)?;
        let logical = kurbo::Point::new(point.x / scale, point.y / scale);
        Ok(view.filter(|view| !self.core.platform_view_occluded(*view, logical)))
    }
}
