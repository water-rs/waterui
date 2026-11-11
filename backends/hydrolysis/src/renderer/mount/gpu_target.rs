//! The GPU implementation of [`LayerTarget`]: concrete types and the GPU
//! device views against a `CherenkovHost`. Material and chrome groups go
//! through the engine code both Cherenkov targets share.

use std::rc::{Rc, Weak};
use std::sync::Arc;

use cherenkov_record::{FilterId, Layer, Transaction};
use waterui_graphics::backend::Reactive;
use waterui_graphics::backend::filtrate::filters::OverscrollStretch;
use waterui_graphics::gpu::PresentedSize;

use crate::gpu_view::{ExternalFrameRuntime, GpuContentRuntime};
use crate::renderer::ProducerWake;
use crate::renderer::effects::{AppliedFilterMetrics, FilteredRuntime};
use crate::renderer::mount::backdrop::{
    BackdropGroupKey, ChromeBackdropGroups, ChromeGroupKey, ChromeMemberPayload,
    MaterialBackdropGroups, MaterialMembership, MemberBind,
};
use crate::renderer::recording::SceneResources;
use crate::text::TextService;

use super::engine_target::{install_engine_text, mount_chrome_group, mount_material_group};
use super::{FilterHandle, LayerTarget, MaterialTerms};

/// The Cherenkov GPU target's host: one per engine window surface.
pub struct CherenkovHost {
    pub engine: Rc<crate::engine::GpuEngine>,
    pub resources: Rc<SceneResources>,
    pub metrics: Arc<AppliedFilterMetrics>,
    /// The engine surface backdrop groups allocate on. Held weakly so the
    /// host never extends the surface past its window.
    pub surface: Weak<cherenkov::Surface<cherenkov_gpu::Gpu>>,
    /// The theme's material terms resolved on this engine at attach
    /// (water-rs/waterui#1788).
    pub materials: MaterialTerms<cherenkov::BackdropShader>,
    /// The window's device-creation chain, which external-frame sources
    /// import their planes onto.
    pub shared_device: cherenkov_gpu::interop::SharedDevice,
}

impl std::fmt::Debug for CherenkovHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CherenkovHost").finish_non_exhaustive()
    }
}

impl<E: crate::text::TextEngine> LayerTarget<E> for cherenkov_gpu::Gpu {
    type Host = CherenkovHost;
    type Group = cherenkov::BackdropGroup;
    type Shader = cherenkov::BackdropShader;
    const BACKDROP_SHADER_REFUSAL: Option<&'static str> = None;

    #[cfg(hydrolysis_hosted)]
    fn mount_hosted(
        tx: &mut Transaction<'_, Self>,
        layer: &Layer,
        runtime: &crate::hosted::HostedRuntime,
        size: kurbo::Size,
    ) -> bool {
        let binding = (layer.id(), size);
        if *runtime.binding.borrow() == Some(binding) {
            return false;
        }
        tx[layer].content(runtime.hosted.at(size));
        runtime.binding.replace(Some(binding));
        true
    }

    fn resources(host: &CherenkovHost) -> &Rc<SceneResources> {
        &host.resources
    }

    fn install_text(host: &CherenkovHost, text: &Arc<TextService<E>>) {
        install_engine_text(&host.resources, &host.engine, text);
    }

    fn mount_gpu_content(
        host: &CherenkovHost,
        tx: &mut Transaction<'_, Self>,
        layer: &Layer,
        runtime: &mut GpuContentRuntime,
        pixels: (u32, u32),
        _wake: ProducerWake,
    ) {
        let runtime = &mut *runtime;
        let producer = runtime
            .producer
            .get_or_insert_with(|| host.engine.gpu_producer(runtime.view.take_engine_content()));
        let binding = (layer.id(), pixels);
        if runtime.binding != Some(binding) {
            tx[layer].content(producer.at(pixels));
            runtime.binding = Some(binding);
        }
        runtime.view.frame();
    }

    fn mount_external_frame(
        host: &CherenkovHost,
        tx: &mut Transaction<'_, Self>,
        layer: &Layer,
        runtime: &mut ExternalFrameRuntime,
        presented: PresentedSize,
        wake: ProducerWake,
    ) -> Option<(u32, u32)> {
        let receiver = runtime.receiver.get_or_insert_with(|| {
            let redraw = waterui_graphics::gpu::RedrawHandle::new(move || wake.request_redraw());
            let (producer, sink) = host.engine.frame_producer();
            runtime.producer = Some(producer);
            runtime.sink = Some(sink);
            runtime.view.stream().start(&host.shared_device, redraw)
        });
        receiver.report_presented(presented);
        // The UI-side pump runs before the drain, so a frame it hands the
        // source this pass is installed this pass.
        runtime.view.frame();
        if let Some(frame) = runtime
            .receiver
            .as_ref()
            .and_then(waterui_graphics::gpu::FrameReceiver::take)
        {
            runtime.frame_pixels = Some(external_frame_plane_size(&frame));
            if let Some(sink) = &runtime.sink {
                sink.submit(frame);
            }
        }
        let pixels = runtime.frame_pixels?;
        let binding = (layer.id(), pixels);
        if runtime.binding != Some(binding) {
            if let Some(producer) = &runtime.producer {
                tx[layer].content(producer.at(pixels));
            }
            runtime.binding = Some(binding);
        }
        Some(pixels)
    }

    fn filter(host: &CherenkovHost, runtime: &mut FilteredRuntime) -> Option<FilterId> {
        runtime.filter(|source| {
            Some(FilterHandle::new(host.engine.effect(
                crate::renderer::effects::EngineEffect::new(source, Arc::clone(&host.metrics)),
            )))
        })
    }

    fn stretch_filter(
        host: &CherenkovHost,
        filter: OverscrollStretch<Reactive>,
    ) -> Option<FilterHandle> {
        Some(FilterHandle::new(host.engine.filter(filter)))
    }

    fn mount_material(
        host: &CherenkovHost,
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

    fn material_terms(host: &CherenkovHost) -> &MaterialTerms<Self::Shader> {
        &host.materials
    }

    fn mount_chrome(
        host: &CherenkovHost,
        tx: &mut Transaction<'_, Self>,
        groups: &mut ChromeBackdropGroups<cherenkov::BackdropGroup, cherenkov::BackdropShader, E>,
        layer: &Layer,
        key: ChromeGroupKey,
        params: cherenkov_record::MaterialCapture,
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
}

fn external_frame_plane_size(frame: &cherenkov_gpu::interop::ExternalFrame) -> (u32, u32) {
    use cherenkov_gpu::interop::FramePlanes;
    match &frame.planes {
        FramePlanes::Yuv { y, .. } => (y.width(), y.height()),
        FramePlanes::Rgb { plane, .. } => (plane.width(), plane.height()),
        #[cfg(all(unix, not(target_vendor = "apple")))]
        FramePlanes::Native(frame) => frame.size(),
    }
}
