//! The CPU implementation of [`LayerTarget`]: the same Cherenkov front end
//! the GPU target drives, on `cherenkov-cpu`'s rasterizer.
//!
//! What a CPU cannot realize without a device is refused at mount, naming the
//! view and the missing GPU: GPU content, external GPU frames and hosted
//! platform content. A theme's backdrop shaders run on the CPU backend's
//! lane executor, through the same material and chrome groups as on the GPU
//! target, and so does a filtered view's filter: its composed stages run on
//! the raster's bands. A filtered view the raster cannot run — an arbitrary
//! GPU effect, or a declared output size — is refused at mount with the
//! reason. Each refusal is a contract violation — a debug build panics, a
//! release build logs it and the refused content is not drawn.

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

use super::engine_target::{
    install_engine_text, mount_chrome_group, mount_material_group, settle_chrome_groups,
};
use super::{FilterHandle, LayerTarget, MaterialTerms};

/// Why a view that needs a GPU device cannot mount on this target. A build
/// without the `gpu` feature refuses those views before they reach a target
/// (`refuse_gpu_device_view`), so here the build has both painters and
/// adapter selection chose this one.
#[cfg(hydrolysis_gpu)]
const NO_GPU: &str = "this runtime renders on the Cherenkov CPU backend: the host has no usable \
                      hardware GPU";

/// The Cherenkov CPU target's host: one per window of an offscreen runtime
/// that selected the CPU backend.
pub struct RasterHost {
    pub engine: Rc<crate::engine::RasterEngine>,
    pub resources: Rc<SceneResources>,
    /// The engine surface backdrop groups allocate on. Held weakly so the
    /// host never extends the surface past its window.
    pub surface: Weak<cherenkov::Surface<cherenkov_cpu::Raster>>,
    /// The theme's material terms resolved on this engine at attach
    /// (water-rs/waterui#1788).
    pub materials: MaterialTerms<cherenkov::BackdropShader>,
}

impl std::fmt::Debug for RasterHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RasterHost").finish_non_exhaustive()
    }
}

impl<E: crate::text::TextEngine> LayerTarget<E> for cherenkov_cpu::Raster {
    type Host = RasterHost;
    type Group = cherenkov::BackdropGroup;
    type Shader = cherenkov::BackdropShader;
    const BACKDROP_SHADER_REFUSAL: Option<&'static str> = None;

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

    fn filter(host: &RasterHost, runtime: &mut FilteredRuntime) -> Option<FilterId> {
        runtime.filter(|source| match source.into_cpu() {
            Ok(effect) => Some(FilterHandle::new(host.engine.effect(effect))),
            Err(error) => {
                contract_violation!(
                    "hydrolysis: a filtered view cannot run on the Cherenkov CPU backend: {error}; \
                     the layer draws without its filter"
                );
                None
            }
        })
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

    fn material_terms(host: &RasterHost) -> &MaterialTerms<cherenkov::BackdropShader> {
        &host.materials
    }

    fn mount_chrome(
        host: &RasterHost,
        tx: &mut Transaction<'_, Self>,
        groups: &mut ChromeBackdropGroups<cherenkov::BackdropGroup, cherenkov::BackdropShader, E>,
        layer: &Layer,
        key: ChromeGroupKey,
        params: ChromeParams,
        display_scale: f64,
        membership: &MaterialMembership<E>,
        payload: impl FnOnce() -> ChromeMemberPayload<cherenkov::BackdropShader>,
        bind: MemberBind,
    ) {
        mount_chrome_group(
            &host.surface,
            tx,
            groups,
            layer,
            key,
            params,
            display_scale,
            membership,
            payload,
            bind,
        );
    }

    fn clear_chrome(tx: &mut Transaction<'_, Self>, layer: &Layer) {
        tx[layer].clear_backdrop();
    }

    fn settle_chrome(
        host: &RasterHost,
        tx: &mut Transaction<'_, Self>,
        groups: &mut ChromeBackdropGroups<cherenkov::BackdropGroup, cherenkov::BackdropShader, E>,
        display_scale: f64,
    ) {
        settle_chrome_groups(
            &host.surface,
            tx,
            groups,
            &host.materials.registry,
            display_scale,
        );
    }
}
