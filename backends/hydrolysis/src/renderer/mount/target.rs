//! The target seam a [`Mount`](super::Mount) commits through (§A.1): what
//! a node's layers need from the engine beyond recorded content.

use std::rc::{Rc, Weak};
use std::sync::Arc;

use cherenkov::{FilterId, Layer, Transaction};

use crate::gpu_view::{ExternalFrameRuntime, GpuContentRuntime};
use crate::renderer::ProducerWake;
use crate::renderer::effects::{AppliedFilterMetrics, FilteredRuntime};
use crate::renderer::material::MaterialRuntime;
use crate::renderer::recording::SceneResources;

/// A layer target: recorded runs land through `Transaction`, and the
/// producer, filter and backdrop installs through the target's host.
pub trait LayerTarget: cherenkov::Target {
    /// What the target installs against (engine, resources, devices).
    type Host;
    /// The backdrop state a material frame holds; dropping it releases
    /// the backdrop group.
    type Material: 'static;

    fn resources(host: &Self::Host) -> &Rc<SceneResources>;

    /// Binds a GPU content producer at `pixels` on `layer`.
    fn mount_gpu_content(
        host: &Self::Host,
        tx: &mut Transaction<'_, Self>,
        layer: &Layer,
        runtime: &mut GpuContentRuntime,
        pixels: (u32, u32),
        wake: ProducerWake,
    );

    /// Starts (once) and feeds an external frame stream on `layer`;
    /// returns the bound frame's pixel size once a frame arrived.
    fn mount_external_frame(
        host: &Self::Host,
        tx: &mut Transaction<'_, Self>,
        layer: &Layer,
        runtime: &mut ExternalFrameRuntime,
        wake: ProducerWake,
    ) -> Option<(u32, u32)>;

    fn filter(host: &Self::Host, runtime: &mut FilteredRuntime) -> FilterId;

    /// Makes `layer` a member of the material's backdrop group while
    /// `visible`, (re)building the group when `display_scale` changes;
    /// releases it when not visible.
    fn mount_material(
        host: &Self::Host,
        tx: &mut Transaction<'_, Self>,
        layer: &Layer,
        runtime: &MaterialRuntime,
        display_scale: f64,
        visible: bool,
        state: &mut Option<Self::Material>,
    );
}

/// The Cherenkov GPU target's host: one per engine window surface.
pub struct CherenkovHost {
    pub engine: Rc<crate::engine::GpuEngine>,
    pub resources: Rc<SceneResources>,
    pub metrics: Arc<AppliedFilterMetrics>,
    /// The engine surface backdrop groups allocate on. Held weakly so the
    /// host never extends the surface past its window.
    pub surface: Weak<cherenkov::Surface<cherenkov_gpu::Gpu>>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

/// A GPU material frame's backdrop group membership.
pub struct GpuMaterial {
    _group: cherenkov::BackdropGroup,
    display_scale: u64,
}

impl GpuMaterial {
    #[cfg(test)]
    pub(crate) const fn display_scale(&self) -> f64 {
        f64::from_bits(self.display_scale)
    }
}

impl LayerTarget for cherenkov_gpu::Gpu {
    type Host = CherenkovHost;
    type Material = GpuMaterial;

    fn resources(host: &CherenkovHost) -> &Rc<SceneResources> {
        &host.resources
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
        wake: ProducerWake,
    ) -> Option<(u32, u32)> {
        if runtime.receiver.is_none() {
            let redraw = waterui_graphics::gpu::RedrawHandle::new(move || wake.request_redraw());
            let (producer, sink) = host.engine.frame_producer();
            runtime.producer = Some(producer);
            runtime.sink = Some(sink);
            runtime.receiver = Some(
                runtime
                    .view
                    .stream()
                    .start(&host.device, &host.queue, redraw),
            );
        }
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

    fn filter(host: &CherenkovHost, runtime: &mut FilteredRuntime) -> FilterId {
        runtime.filter(&host.engine, &host.metrics).id()
    }

    fn mount_material(
        host: &CherenkovHost,
        tx: &mut Transaction<'_, Self>,
        layer: &Layer,
        runtime: &MaterialRuntime,
        display_scale: f64,
        visible: bool,
        state: &mut Option<GpuMaterial>,
    ) {
        if !visible {
            if state.take().is_some() {
                tx[layer].clear_backdrop();
            }
            return;
        }
        let bits = display_scale.to_bits();
        if state
            .as_ref()
            .is_some_and(|material| material.display_scale == bits)
        {
            return;
        }
        let surface = host
            .surface
            .upgrade()
            .expect("hydrolysis material: the engine surface was dropped during its commit");
        let group = surface.backdrop_group(
            runtime.chain(display_scale),
            crate::renderer::material::capture_scale(),
        );
        tx[layer].backdrop(group.sample());
        *state = Some(GpuMaterial {
            _group: group,
            display_scale: bits,
        });
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
