//! The target seam a [`Mount`](super::Mount) commits through (§A.1): what
//! a node's layers need from the engine beyond recorded content.

use std::rc::{Rc, Weak};
use std::sync::Arc;

use cherenkov::{FilterId, Layer, Transaction};
use cherenkov_record::MaterialRegistry;
use rustc_hash::FxHashMap;

use crate::gpu_view::{ExternalFrameRuntime, GpuContentRuntime};
use crate::renderer::ProducerWake;
use crate::renderer::effects::{AppliedFilterMetrics, FilteredRuntime};
use crate::renderer::mount::backdrop::{
    BackdropGroupKey, ChromeBackdropGroups, ChromeGroupKey, MaterialBackdropGroups,
    MaterialMembership, MemberJoin,
};
use crate::renderer::mount::program::ChromeMaterial;
use crate::renderer::recording::SceneResources;

/// A layer target: recorded runs land through `Transaction`, and the
/// producer, filter and backdrop installs through the target's host.
pub trait LayerTarget: cherenkov::Target {
    /// What the target installs against (engine, resources, devices).
    type Host;
    /// The backdrop-group object the mount's [`MaterialBackdropGroups`]
    /// and [`ChromeBackdropGroups`] tables hold — on the GPU target a
    /// [`cherenkov::BackdropGroup`], whose drop unregisters it.
    type Group: 'static;

    /// The engine's backdrop-shader handle the material registry's
    /// sources resolve to at attach — `cherenkov::BackdropShader` on the
    /// GPU target.
    type Shader: 'static;

    /// Whether the target's engine realizes backdrop shaders at all.
    /// `false` — a CPU engine — makes a [`Mount`](super::Mount::new)
    /// attach panic when the theme's material registry is not empty
    /// (water-rs/waterui#1788).
    const BACKDROP_SHADERS: bool;

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

    /// Joins `layer` to the backdrop group `key` names, sharing the
    /// group's filtered capture with every member under the same key,
    /// rebuilding it when `display_scale` changes. The membership the
    /// layer's node holds ends with its layers; its owner link is how a
    /// rebuild re-points every member at the replacement group.
    fn mount_material(
        host: &Self::Host,
        tx: &mut Transaction<'_, Self>,
        groups: &mut MaterialBackdropGroups<Self::Group>,
        layer: &Layer,
        key: BackdropGroupKey,
        display_scale: f64,
        membership: &MaterialMembership,
    );

    /// Clears the backdrop membership [`mount_material`](Self::mount_material)
    /// installed on `layer`. Targets that never install one leave the
    /// default no-op.
    fn clear_material(tx: &mut Transaction<'_, Self>, layer: &Layer) {
        let _ = (tx, layer);
    }

    /// The per-engine terms the theme's material registry resolved to at
    /// attach — the registry itself plus each registered shader's engine
    /// handle.
    fn material_terms(host: &Self::Host) -> &MaterialTerms<Self>;

    /// Joins `layer` — a `ChromeMaterial` member — to the chrome group
    /// `key` names, sharing the group's unfiltered capture with every
    /// member under the same `(scope, class, canvas)` key, rebuilding it
    /// when `display_scale` changes. `params` is the class's registered
    /// capture terms; `chrome` carries the live shape and effect the
    /// member binds (water-rs/waterui#1788).
    #[expect(
        clippy::too_many_arguments,
        reason = "one install hands the group's whole context at once"
    )]
    fn mount_chrome(
        host: &Self::Host,
        tx: &mut Transaction<'_, Self>,
        groups: &mut ChromeBackdropGroups<Self::Group>,
        layer: &Layer,
        key: ChromeGroupKey,
        params: cherenkov_record::MaterialCapture,
        display_scale: f64,
        membership: &MaterialMembership,
        chrome: &ChromeMaterial,
    );

    /// Clears the backdrop membership [`mount_chrome`](Self::mount_chrome)
    /// installed on `layer`. Targets that never install one leave the
    /// default no-op.
    fn clear_chrome(tx: &mut Transaction<'_, Self>, layer: &Layer) {
        let _ = (tx, layer);
    }
}

/// The material terms one host resolved at engine attach: the theme's
/// registry and each registered shader's handle on this engine. A host
/// with no backdrop shaders fills this only when the registry is empty —
/// `Mount`'s attach assertion covers the rest (water-rs/waterui#1788).
pub struct MaterialTerms<T: LayerTarget> {
    /// The theme's registry: capture classes resolve at member install.
    pub registry: Rc<MaterialRegistry>,
    /// Each registered shader's engine handle, filled at attach by
    /// registering its source with the engine.
    pub shaders: FxHashMap<cherenkov_record::MaterialShader, T::Shader>,
}
impl<T: LayerTarget> std::fmt::Debug for MaterialTerms<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaterialTerms")
            .field("registry", &self.registry)
            .field("shaders", &self.shaders.keys())
            .finish()
    }
}

impl MaterialTerms<cherenkov_gpu::Gpu> {
    /// Attaches the registry to `engine` (water-rs/waterui#1788).
    pub(crate) fn resolve(
        engine: &crate::engine::GpuEngine,
        registry: Rc<MaterialRegistry>,
    ) -> Self {
        let shaders = attach_material_shaders(engine, &registry);
        Self { registry, shaders }
    }
}

/// Attaches `registry` to `engine`: each shader source registers with the
/// engine and its handle joins the per-engine table a chrome member's
/// group binds. A source the engine rejects panics at attach, naming the
/// key — the registry is the theme's own contract, so a rejection is a
/// programmer error.
pub fn attach_material_shaders(
    engine: &crate::engine::GpuEngine,
    registry: &MaterialRegistry,
) -> FxHashMap<cherenkov_record::MaterialShader, cherenkov::BackdropShader> {
    registry
        .shaders()
        .map(|(key, source)| {
            let shader = engine.backdrop_shader(source.clone()).unwrap_or_else(|error| {
                panic!(
                    "hydrolysis materials: backdrop shader {key:?} failed to register on the                      attached engine: {error}"
                )
            });
            (key, shader)
        })
        .collect()
}

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
    pub materials: MaterialTerms<cherenkov_gpu::Gpu>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl std::fmt::Debug for CherenkovHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CherenkovHost").finish_non_exhaustive()
    }
}

impl LayerTarget for cherenkov_gpu::Gpu {
    type Host = CherenkovHost;
    type Group = cherenkov::BackdropGroup;
    type Shader = cherenkov::BackdropShader;
    const BACKDROP_SHADERS: bool = true;

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
        groups: &mut MaterialBackdropGroups<cherenkov::BackdropGroup>,
        layer: &Layer,
        key: BackdropGroupKey,
        display_scale: f64,
        membership: &MaterialMembership,
    ) {
        let surface = host
            .surface
            .upgrade()
            .expect("hydrolysis material: the engine surface was dropped during its commit");
        groups.join(
            tx,
            key,
            key.runtime(),
            display_scale,
            MemberJoin {
                layer,
                membership,
                resolve: super::layers::NodeLayers::frame_layer,
            },
            (
                |runtime: &crate::renderer::material::MaterialRuntime, scale| {
                    surface.backdrop_group(
                        runtime.chain(scale),
                        crate::renderer::material::capture_scale(),
                    )
                },
                |tx: &mut Transaction<'_, Self>, layer, group: &cherenkov::BackdropGroup| {
                    tx[layer].backdrop(group.sample());
                },
            ),
        );
    }

    fn clear_material(tx: &mut Transaction<'_, Self>, layer: &Layer) {
        tx[layer].clear_backdrop();
    }

    fn material_terms(host: &CherenkovHost) -> &MaterialTerms<Self> {
        &host.materials
    }

    fn mount_chrome(
        host: &CherenkovHost,
        tx: &mut Transaction<'_, Self>,
        groups: &mut ChromeBackdropGroups<cherenkov::BackdropGroup>,
        layer: &Layer,
        key: ChromeGroupKey,
        params: cherenkov_record::MaterialCapture,
        display_scale: f64,
        membership: &MaterialMembership,
        chrome: &ChromeMaterial,
    ) {
        let surface = host
            .surface
            .upgrade()
            .expect("hydrolysis materials: the engine surface was dropped during its commit");
        let shader = host
            .materials
            .shaders
            .get(&chrome.material.shader())
            .unwrap_or_else(|| {
                panic!(
                    "hydrolysis materials: backdrop shader {:?} is registered but has no handle                      on this engine — a theme's registry is registered at engine attach",
                    chrome.material.shader(),
                )
            })
            .clone();
        let (_, effect) = chrome.material.lives();
        groups.join(
            tx,
            key,
            params,
            display_scale,
            MemberJoin {
                layer,
                membership,
                resolve: super::layers::NodeLayers::member_layer,
            },
            (
                |params, _scale| {
                    surface.backdrop_group_unfiltered(cherenkov::BackdropSpec::new(
                        params.scale,
                        params.levels,
                    ))
                },
                |tx, layer, group| {
                    let id = group.id();
                    let shader = shader.clone();
                    tx[layer].backdrop(
                        effect
                            .clone()
                            .map(move |effect| group_sample_with(id, &shader, &effect)),
                    );
                },
            ),
        );
    }

    fn clear_chrome(tx: &mut Transaction<'_, Self>, layer: &Layer) {
        tx[layer].clear_backdrop();
    }
}

/// The live backdrop sample a `ChromeMaterial` member binds: the group's
/// id and the member's effect mapped through its shader.
fn group_sample_with(
    id: cherenkov::BackdropId,
    shader: &cherenkov::BackdropShader,
    effect: &cherenkov_record::MaterialEffect,
) -> cherenkov::BackdropSample {
    cherenkov::BackdropSample::with_effect(id, shader.effect(effect.uniforms().to_vec()))
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
