//! The CPU implementation of [`LayerTarget`]: the same Cherenkov front end
//! the GPU target drives, on `cherenkov-cpu`'s rasterizer.
//!
//! What a CPU cannot realize without a device is refused at mount, naming the
//! view and the missing GPU: GPU content, external GPU frames and hosted
//! platform content. Shader effects and backdrop shaders are not realized on
//! the CPU backend yet (water-rs/waterui#4065): a filtered view is refused at
//! mount, and the mount refuses a theme that registers backdrop shaders at
//! attach. Each refusal is a contract violation — a debug build panics, a
//! release build logs it and the refused content is not drawn.

use std::convert::Infallible;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use cherenkov_record::{FilterId, Layer, Transaction};
use waterui_core::contract_violation;
use waterui_graphics::backend::Reactive;
use waterui_graphics::backend::filtrate::filters::OverscrollStretch;

#[cfg(hydrolysis_gpu)]
use crate::gpu_view::{ExternalFrameRuntime, GpuContentRuntime};
#[cfg(hydrolysis_gpu)]
use crate::renderer::ProducerWake;
use crate::renderer::effects::FilteredRuntime;
use crate::renderer::mount::backdrop::{
    BackdropGroupKey, ChromeBackdropGroups, ChromeGroupKey, ChromeMemberPayload, ChromeParams,
    MaterialBackdropGroups, MaterialMembership, MemberBind,
};
use crate::renderer::recording::SceneResources;
use crate::text::TextService;

use super::engine_target::{install_engine_text, mount_material_group};
use super::{FilterHandle, LayerTarget, MaterialTerms};

/// Why a view that needs a GPU device cannot mount on this target. A build
/// without the `gpu` feature refuses those views before they reach a target
/// (`refuse_gpu_device_view`), so here the build has both painters and
/// adapter selection chose this one.
#[cfg(hydrolysis_gpu)]
const NO_GPU: &str = "this runtime renders on the Cherenkov CPU backend: the host has no usable \
                      hardware GPU";

/// Why a theme's backdrop shaders cannot attach to this target.
const NO_BACKDROP_SHADERS: &str = "the Cherenkov CPU backend does not realize backdrop shaders \
                                   yet (water-rs/waterui#4065)";

/// The Cherenkov CPU target's host: one per window of an offscreen runtime
/// that selected the CPU backend.
pub struct RasterHost {
    pub engine: Rc<crate::engine::RasterEngine>,
    pub resources: Rc<SceneResources>,
    /// The engine surface backdrop groups allocate on. Held weakly so the
    /// host never extends the surface past its window.
    pub surface: Weak<cherenkov::Surface<cherenkov_cpu::Raster>>,
    /// The theme's material terms: the theme's registry with no shader
    /// handles, since this target has no backdrop shaders — the mount's
    /// attach reports a registry that names any, and a chrome member then
    /// finds no handle and draws no chrome.
    pub materials: MaterialTerms<Infallible>,
}

impl std::fmt::Debug for RasterHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RasterHost").finish_non_exhaustive()
    }
}

impl<E: crate::text::TextEngine> LayerTarget<E> for cherenkov_cpu::Raster {
    type Host = RasterHost;
    type Group = cherenkov::BackdropGroup;
    /// No backdrop shader exists on this target, so no chrome member can be
    /// bound: the payload a chrome join would build is uninhabited.
    type Shader = Infallible;
    const BACKDROP_SHADER_REFUSAL: Option<&'static str> = Some(NO_BACKDROP_SHADERS);

    fn resources(host: &RasterHost) -> &Rc<SceneResources> {
        &host.resources
    }

    fn install_text(host: &RasterHost, text: &Arc<TextService<E>>) {
        install_engine_text(&host.resources, &host.engine, text);
    }

    /// Hosted platform content is presented by the GPU engine's host
    /// presentation; the CPU framebuffer has no platform object to place.
    #[cfg(all(hydrolysis_hosted, hydrolysis_gpu))]
    fn mount_hosted(
        _tx: &mut Transaction<'_, Self>,
        _layer: &Layer,
        _runtime: &crate::hosted::HostedRuntime,
        _size: kurbo::Size,
    ) -> bool {
        crate::renderer::unsupported_gpu_device_view("hosted platform content", NO_GPU);
        false
    }

    #[cfg(hydrolysis_gpu)]
    fn mount_gpu_content(
        _host: &RasterHost,
        _tx: &mut Transaction<'_, Self>,
        _layer: &Layer,
        _runtime: &mut GpuContentRuntime,
        _pixels: (u32, u32),
        _wake: ProducerWake,
    ) {
        crate::renderer::unsupported_gpu_device_view("GpuContentView", NO_GPU);
    }

    #[cfg(hydrolysis_gpu)]
    fn mount_external_frame(
        _host: &RasterHost,
        _tx: &mut Transaction<'_, Self>,
        _layer: &Layer,
        _runtime: &mut ExternalFrameRuntime,
        _presented: waterui_graphics::gpu::PresentedSize,
        _wake: ProducerWake,
    ) -> Option<(u32, u32)> {
        crate::renderer::unsupported_gpu_device_view("ExternalFrameView", NO_GPU);
        None
    }

    fn filter(_host: &RasterHost, _runtime: &mut FilteredRuntime) -> Option<FilterId> {
        contract_violation!(
            "hydrolysis: a filtered view runs a shader effect, which the Cherenkov CPU backend \
             does not realize yet (water-rs/waterui#4065); the layer draws without its filter"
        );
        None
    }

    fn stretch_filter(
        host: &RasterHost,
        filter: OverscrollStretch<Reactive>,
    ) -> Option<FilterHandle> {
        Some(FilterHandle::new(host.engine.filter(filter)))
    }

    fn mount_material(
        host: &RasterHost,
        tx: &mut Transaction<'_, Self>,
        groups: &mut MaterialBackdropGroups<cherenkov::BackdropGroup, E>,
        layer: &Layer,
        key: BackdropGroupKey,
        display_scale: f64,
        membership: &MaterialMembership<E>,
        bind: MemberBind,
    ) {
        mount_material_group(
            &host.surface,
            tx,
            groups,
            layer,
            key,
            display_scale,
            membership,
            bind,
        );
    }

    fn clear_material(tx: &mut Transaction<'_, Self>, layer: &Layer) {
        tx[layer].clear_backdrop();
    }

    fn material_terms(host: &RasterHost) -> &MaterialTerms<Infallible> {
        &host.materials
    }

    #[expect(
        unreachable_code,
        reason = "the payload carries an `Infallible` shader, so building it already diverges; the empty match states that no shader value exists on this target"
    )]
    fn mount_chrome(
        _host: &RasterHost,
        _tx: &mut Transaction<'_, Self>,
        _groups: &mut ChromeBackdropGroups<cherenkov::BackdropGroup, Infallible, E>,
        _layer: &Layer,
        _key: ChromeGroupKey,
        _params: ChromeParams,
        _display_scale: f64,
        _membership: &MaterialMembership<E>,
        payload: impl FnOnce() -> ChromeMemberPayload<Infallible>,
        _bind: MemberBind,
    ) {
        // A chrome member is bound only once `chrome_terms` found its
        // shader's handle, and this target's shader map is always empty;
        // the shader its payload names has no value on this target.
        match payload().shader {}
    }
}
