//! The GPU implementation of [`LayerTarget`]: concrete types, material
//! shader attach and the chrome/group sampling queries against a
//! `CherenkovHost`.

use std::rc::{Rc, Weak};
use std::sync::Arc;

use cherenkov_record::{FilterId, Layer, MaterialRegistry, Transaction};
use rustc_hash::FxHashMap;
use waterui_graphics::Reactive;
use waterui_graphics::filtrate::filters::OverscrollStretch;

use crate::gpu_view::{ExternalFrameRuntime, GpuContentRuntime};
use crate::renderer::ProducerWake;
use crate::renderer::effects::{AppliedFilterMetrics, FilteredRuntime};
use crate::renderer::mount::backdrop::{
    BackdropGroupKey, ChromeBackdropGroups, ChromeGroupKey, ChromeMemberPayload,
    MaterialBackdropGroups, MaterialMembership, MemberBind, MemberJoin,
};
use crate::renderer::recording::SceneResources;

use super::{FilterHandle, FilterRegistration, LayerTarget, MaterialTerms};

impl MaterialTerms<cherenkov::BackdropShader> {
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
            let shader = engine
                .backdrop_shader(source.clone())
                .unwrap_or_else(|error| {
                    panic!(
                        "hydrolysis materials: backdrop shader {key:?} failed to register on the \
                         attached engine: {error}"
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
    pub materials: MaterialTerms<cherenkov::BackdropShader>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
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
    const BACKDROP_SHADERS: bool = true;

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
        runtime.filter(|source| {
            FilterHandle::new(
                host.engine
                    .effect(crate::renderer::effects::EngineEffect::new(
                        source,
                        Arc::clone(&host.metrics),
                    )),
            )
        })
    }

    fn stretch_filter(host: &CherenkovHost, filter: OverscrollStretch<Reactive>) -> FilterHandle {
        FilterHandle::new(host.engine.filter(filter))
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
                payload: || (),
                resolve: super::super::layers::NodeLayers::frame_layer,
            },
            (
                |runtime: &crate::renderer::material::MaterialRuntime, scale| {
                    let spec = cherenkov::BackdropSpec::new(
                        crate::renderer::material::capture_scale(),
                        cherenkov::CaptureLevels::ONE,
                    );
                    // A `.material_group()` scope's groups capture beneath
                    // its anchor layer (water-rs/waterui#2097).
                    let spec = key.anchor.map_or(spec, |anchor| spec.anchor(anchor));
                    surface.backdrop_group(runtime.chain(scale), spec)
                },
                |tx: &mut Transaction<'_, Self>,
                 member: &Layer,
                 _payload: &(),
                 group: &cherenkov::BackdropGroup,
                 _scale: f64| {
                    tx[member].backdrop(group.sample());
                },
            ),
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
        let surface = host
            .surface
            .upgrade()
            .expect("hydrolysis materials: the engine surface was dropped during its commit");
        groups.join(
            tx,
            key,
            params,
            display_scale,
            MemberJoin {
                layer,
                membership,
                payload,
                resolve: super::super::layers::NodeLayers::member_layer,
            },
            (
                move |params, scale| {
                    surface.backdrop_group_unfiltered(chrome_spec(params, scale, key))
                },
                |tx: &mut Transaction<'_, Self>,
                 member: &Layer,
                 payload: &ChromeMemberPayload<cherenkov::BackdropShader>,
                 group: &cherenkov::BackdropGroup,
                 scale: f64| {
                    tx[member].backdrop(chrome_sample(payload, group.id(), scale));
                },
            ),
            bind,
        );
    }

    fn clear_chrome(tx: &mut Transaction<'_, Self>, layer: &Layer) {
        tx[layer].clear_backdrop();
    }
}

/// The spec a chrome group of `key` is created with at display `scale`:
/// the class's capture scale, levels and member blend space, its union
/// field converted at `scale`, and — for a scoped group — its scope's
/// anchor layer, beneath which it captures like a material group
/// (water-rs/waterui#2097).
///
/// # Panics
/// Panics as [`union_of`] does for a union smoothing invalid at `scale`.
pub fn chrome_spec(
    params: &cherenkov_record::MaterialCapture,
    scale: f64,
    key: ChromeGroupKey,
) -> cherenkov::BackdropSpec {
    let mut spec =
        cherenkov::BackdropSpec::new(params.scale, params.levels).blend_space(params.blend_space);
    if let Some(anchor) = key.anchor() {
        spec = spec.anchor(anchor);
    }
    if let Some(union) = union_of(params, scale, key.class()) {
        spec = spec.union(union);
    }
    spec
}

/// The union field `params` declares for a group built at `scale`:
/// `None` for `Solo` and `Shared` classes; for `Union`, the class's
/// logical smoothing converted to device pixels — [`BackdropUnion`]'s
/// input — with its error surfaced, never clamped.
///
/// # Panics
/// Panics, naming the class, smoothing and scale, when the converted
/// smoothing is not a valid [`BackdropUnion`].
pub fn union_of(
    params: &cherenkov_record::MaterialCapture,
    scale: f64,
    class: cherenkov_record::CaptureClass,
) -> Option<cherenkov::BackdropUnion> {
    let cherenkov_record::MaterialGrouping::Union { smoothing } = params.grouping else {
        return None;
    };
    let device = f64::from(smoothing.get()) * scale;
    Some(
        cherenkov::BackdropUnion::new(crate::num_cast::f64_as_f32(device)).unwrap_or_else(|error| {
            panic!(
                "hydrolysis materials: capture class {class:?} union smoothing {} is invalid at display scale {scale}: {error}",
                smoothing.get(),
            )
        }),
    )
}

/// The member's logical outer `extent` as device pixels at `scale` — the
/// display scale the group was built for.
///
/// # Panics
/// Panics when `extent` converted to device pixels is not a valid
/// [`BackdropOuter`]: its error is surfaced, never clamped.
pub fn outer_of(extent: cherenkov_record::OuterExtent, scale: f64) -> cherenkov::BackdropOuter {
    let device = f64::from(extent.get()) * scale;
    cherenkov::BackdropOuter::new(crate::num_cast::f64_as_f32(device)).unwrap_or_else(|error| {
        panic!(
            "hydrolysis materials: outer extent {} is invalid at display scale {scale}: {error}",
            extent.get(),
        )
    })
}

/// The live backdrop sample a `ChromeMaterial` member binds on the group
/// `id`, built for `scale`: the member's effect rebound from its value
/// now — a stored payload never restarts stale — and mapped through
/// [`group_sample_with`].
pub fn chrome_sample(
    payload: &ChromeMemberPayload<cherenkov::BackdropShader>,
    id: cherenkov::BackdropId,
    scale: f64,
) -> cherenkov_record::SharedLive<cherenkov::BackdropSample> {
    let shader = payload.shader.clone();
    payload
        .effect
        .rebound()
        .map(move |effect| group_sample_with(id, &shader, &effect, scale))
}

/// The display `scale` the group was built for as the member's
/// [`RecordingScale`](cherenkov::RecordingScale): the device pixels one
/// logical pixel of the recording spans.
///
/// # Panics
/// Panics when `scale` is not a valid recording scale: its error is
/// surfaced, never clamped.
pub fn recording_scale_of(scale: f64) -> cherenkov::RecordingScale {
    cherenkov::RecordingScale::new(crate::num_cast::f64_as_f32(scale)).unwrap_or_else(|error| {
        panic!("hydrolysis materials: display scale {scale} is not a recording scale: {error}")
    })
}

/// The live backdrop sample a `ChromeMaterial` member binds: the group's
/// id and the member's effect mapped through its shader, the effect's
/// logical outer extent converted to device pixels at `scale` — the
/// display scale the group was built for — and `scale` itself as the
/// member's recording scale, so its shader converts the recipe's
/// logical lengths.
pub fn group_sample_with(
    id: cherenkov::BackdropId,
    shader: &cherenkov::BackdropShader,
    effect: &cherenkov_record::MaterialEffect,
    scale: f64,
) -> cherenkov::BackdropSample {
    let outer = outer_of(effect.outer_extent(), scale);
    cherenkov::BackdropSample::with_effect(id, shader.effect(effect.uniforms().to_vec()))
        .outer(outer)
        .scale(recording_scale_of(scale))
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

impl FilterRegistration for cherenkov::Filter {
    fn filter_id(&self) -> cherenkov_record::FilterId {
        self.id()
    }
}
