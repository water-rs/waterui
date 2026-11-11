//! The GPU implementation of [`LayerTarget`]: concrete types, material
//! shader attach and the chrome/group sampling queries against a
//! `CherenkovHost`.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use cherenkov_record::{FilterId, Layer, MaterialRegistry, ResourceError, Transaction};
use rustc_hash::FxHashMap;
use waterui_core::contract_violation;
use waterui_graphics::backend::filtrate::filters::OverscrollStretch;
use waterui_graphics::backend::{Handle, Reactive, TextBackend, TextRegistration, TextRequest};
use waterui_graphics::gpu::PresentedSize;

use crate::gpu_view::{ExternalFrameRuntime, GpuContentRuntime};
use crate::renderer::ProducerWake;
use crate::renderer::effects::{AppliedFilterMetrics, FilteredRuntime};
use crate::renderer::mount::backdrop::{
    BackdropGroupKey, ChromeBackdropGroups, ChromeGroupKey, ChromeMemberPayload,
    MaterialBackdropGroups, MaterialMembership, MemberBind, MemberJoin, Unbound,
};
use crate::renderer::recording::SceneResources;
use crate::text::TextService;
use crate::text::{CanvasLayout, shape_canvas_text};

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
/// group binds. A source the engine rejects is reported at attach, naming
/// the key, and stays out of the table: the chrome that names it is not
/// drawn.
pub fn attach_material_shaders(
    engine: &crate::engine::GpuEngine,
    registry: &MaterialRegistry,
) -> FxHashMap<cherenkov_record::MaterialShader, cherenkov::BackdropShader> {
    registry
        .shaders()
        .filter_map(|(key, source)| {
            let shader = engine
                .backdrop_shader(source.clone())
                .inspect_err(|error| {
                    contract_violation!(
                        "hydrolysis materials: backdrop shader {key:?} failed to register on the \
                         attached engine: {error}; the chrome that names it is not drawn"
                    );
                })
                .ok()?;
            Some((key, shader))
        })
        .collect()
}

/// Installs on `resources` the registry that lays canvas text out through
/// `text` and registers it with `engine`.
pub fn install_gpu_text<E: crate::text::TextEngine>(
    resources: &SceneResources,
    engine: &Rc<crate::engine::GpuEngine>,
    text: &Arc<TextService<E>>,
) {
    resources.install_text(text, || {
        Rc::new(GpuText {
            engine: Rc::clone(engine),
            service: Arc::clone(text),
            faces: RefCell::default(),
        })
    });
}

/// Canvas text on the GPU target: laid out by the session's text engine,
/// registered with the Cherenkov engine as glyph runs.
struct GpuText<E: crate::text::TextEngine> {
    engine: Rc<crate::engine::GpuEngine>,
    service: Arc<TextService<E>>,
    /// The faces text layouts have registered, by blob and index, so a text
    /// laid out again does not register its fonts again. Each layout holds
    /// its own share of a face; the cache holds one for as long as the
    /// registry lives.
    faces: RefCell<FxHashMap<(u64, u32), cherenkov::Font>>,
}

impl<E: crate::text::TextEngine> TextBackend for GpuText<E> {
    fn register_text(
        &self,
        request: TextRequest,
    ) -> Result<Option<TextRegistration>, ResourceError> {
        let Some(CanvasLayout { layout, size }) = shape_canvas_text(&self.service, &request) else {
            return Ok(None);
        };
        let mut faces = self.faces.borrow_mut();
        let text = self.service.cherenkov_text(&layout, &mut |face| {
            let key = (face.data.id(), face.index);
            if let Some(font) = faces.get(&key) {
                return Ok(font.clone());
            }
            let font = self.engine.font(cherenkov::FontSource {
                data: Arc::from(face.data.data()),
                index: face.index,
            })?;
            faces.insert(key, font.clone());
            Ok(font)
        })?;
        drop(faces);
        let handle = self.engine.text_layout(text)?;
        Ok(Some(TextRegistration::new(Handle::new(handle), size)))
    }
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

    fn install_text(host: &CherenkovHost, text: &Arc<TextService<E>>) {
        install_gpu_text(&host.resources, &host.engine, text);
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
        let Some(surface) = host.surface.upgrade() else {
            contract_violation!(
                "hydrolysis material: the engine surface was dropped during its commit; \
                 the layer joins no backdrop group"
            );
            groups.clear(layer.id());
            tx[layer].clear_backdrop();
            return;
        };
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
                    Ok(surface.backdrop_group(runtime.chain(scale), spec))
                },
                |tx: &mut Transaction<'_, Self>,
                 member: &Layer,
                 _payload: &(),
                 group: &cherenkov::BackdropGroup,
                 _scale: f64| {
                    tx[member].backdrop(group.sample());
                    Ok(())
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
        let Some(surface) = host.surface.upgrade() else {
            contract_violation!(
                "hydrolysis materials: the engine surface was dropped during its commit; \
                 the layer joins no chrome group"
            );
            groups.clear(layer.id());
            tx[layer].clear_backdrop();
            return;
        };
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
                    Ok(surface.backdrop_group_unfiltered(chrome_spec(params, scale, key)?))
                },
                |tx: &mut Transaction<'_, Self>,
                 member: &Layer,
                 payload: &ChromeMemberPayload<cherenkov::BackdropShader>,
                 group: &cherenkov::BackdropGroup,
                 scale: f64| {
                    tx[member].backdrop(chrome_sample(payload, group.id(), scale)?);
                    Ok(())
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
/// the class's capture scale, levels, pyramid and member blend space, its union
/// field converted at `scale`, and — for a scoped group — its scope's
/// anchor layer, beneath which it captures like a material group
/// (water-rs/waterui#2097).
///
/// # Errors
/// [`Unbound`], reported as [`union_of`] reports it, for a union smoothing
/// invalid at `scale`: the group is not built.
pub fn chrome_spec(
    params: &cherenkov_record::MaterialCapture,
    scale: f64,
    key: ChromeGroupKey,
) -> Result<cherenkov::BackdropSpec, Unbound> {
    let mut spec = cherenkov::BackdropSpec::new(params.scale, params.levels)
        .pyramid(params.pyramid)
        .blend_space(params.blend_space);
    if let Some(anchor) = key.anchor() {
        spec = spec.anchor(anchor);
    }
    if let Some(union) = union_of(params, scale, key.class())? {
        spec = spec.union(union);
    }
    Ok(spec)
}

/// The union field `params` declares for a group built at `scale`:
/// `None` for `Solo` and `Shared` classes; for `Union`, the class's
/// logical smoothing converted to device pixels — [`BackdropUnion`]'s
/// input — with its error surfaced, never clamped.
///
/// # Errors
/// [`Unbound`] when the converted smoothing is not a valid
/// [`BackdropUnion`], reported through [`contract_violation!`] naming the
/// class, smoothing and scale.
pub fn union_of(
    params: &cherenkov_record::MaterialCapture,
    scale: f64,
    class: cherenkov_record::CaptureClass,
) -> Result<Option<cherenkov::BackdropUnion>, Unbound> {
    let cherenkov_record::MaterialGrouping::Union { smoothing } = params.grouping else {
        return Ok(None);
    };
    let device = f64::from(smoothing.get()) * scale;
    cherenkov::BackdropUnion::new(crate::num_cast::f64_as_f32(device))
        .map(Some)
        .map_err(|error| {
            contract_violation!(
                "hydrolysis materials: capture class {class:?} union smoothing {} is invalid at \
                 display scale {scale}: {error}; the class's group is not built and its members \
                 are left unbound",
                smoothing.get(),
            );
            Unbound
        })
}

/// The member's logical outer `extent` as device pixels at `scale` — the
/// display scale the group was built for.
///
/// # Errors
/// [`Unbound`] when `extent` converted to device pixels is not a valid
/// [`BackdropOuter`], reported through [`contract_violation!`]: its error
/// is surfaced, never clamped.
pub fn outer_of(
    extent: cherenkov_record::OuterExtent,
    scale: f64,
) -> Result<cherenkov::BackdropOuter, Unbound> {
    let device = f64::from(extent.get()) * scale;
    cherenkov::BackdropOuter::new(crate::num_cast::f64_as_f32(device)).map_err(|error| {
        contract_violation!(
            "hydrolysis materials: outer extent {} is invalid at display scale {scale}: {error}; \
             the member is not bound to it",
            extent.get(),
        );
        Unbound
    })
}

/// The live backdrop sample a `ChromeMaterial` member binds on the group
/// `id`, built for `scale`: the member's effect rebound from its value
/// now — a stored payload never restarts stale — and mapped through
/// [`group_sample_with`].
///
/// # Errors
/// [`Unbound`], reported, when `scale` is not a recording scale or the
/// effect's outer extent now is invalid at it: the member is not bound.
/// A later change of a bound effect to an extent invalid at `scale` is
/// reported the same way, but a bound sample cannot unbind its layer from
/// inside the signal, so the member keeps the last sample it bound: the
/// change is rejected, never clamped.
pub fn chrome_sample(
    payload: &ChromeMemberPayload<cherenkov::BackdropShader>,
    id: cherenkov::BackdropId,
    scale: f64,
) -> Result<cherenkov_record::SharedLive<cherenkov::BackdropSample>, Unbound> {
    let recording = recording_scale_of(scale)?;
    let effect = payload.effect.rebound();
    let shader = payload.shader.clone();
    let bound = RefCell::new(group_sample_with(
        id,
        &shader,
        effect.value(),
        scale,
        recording,
    )?);
    Ok(effect.map(
        move |effect| match group_sample_with(id, &shader, &effect, scale, recording) {
            Ok(sample) => {
                bound.replace(sample.clone());
                sample
            }
            Err(Unbound) => bound.borrow().clone(),
        },
    ))
}

/// The display `scale` the group was built for as the member's
/// [`RecordingScale`](cherenkov::RecordingScale): the device pixels one
/// logical pixel of the recording spans.
///
/// # Errors
/// [`Unbound`] when `scale` is not a valid recording scale, reported
/// through [`contract_violation!`]: its error is surfaced, never clamped.
pub fn recording_scale_of(scale: f64) -> Result<cherenkov::RecordingScale, Unbound> {
    cherenkov::RecordingScale::new(crate::num_cast::f64_as_f32(scale)).map_err(|error| {
        contract_violation!(
            "hydrolysis materials: display scale {scale} is not a recording scale: {error}; \
             the member is left unbound"
        );
        Unbound
    })
}

/// The backdrop sample a `ChromeMaterial` member binds: the group's id
/// and the member's effect mapped through its shader, the effect's
/// logical outer extent converted to device pixels at `scale` — the
/// display scale the group was built for — and `recording`, `scale` as
/// a [`RecordingScale`](cherenkov::RecordingScale), so its shader
/// converts the recipe's logical lengths.
///
/// # Errors
/// [`Unbound`], reported as [`outer_of`] reports it, when the effect's
/// outer extent is invalid at `scale`.
pub fn group_sample_with(
    id: cherenkov::BackdropId,
    shader: &cherenkov::BackdropShader,
    effect: &cherenkov_record::MaterialEffect,
    scale: f64,
    recording: cherenkov::RecordingScale,
) -> Result<cherenkov::BackdropSample, Unbound> {
    let outer = outer_of(effect.outer_extent(), scale)?;
    Ok(
        cherenkov::BackdropSample::with_effect(id, shader.effect(effect.uniforms().to_vec()))
            .outer(outer)
            .scale(recording),
    )
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
