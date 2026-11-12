//! What the GPU and CPU [`LayerTarget`](super::LayerTarget)s share: both are
//! the one Cherenkov front end on a different backend, so canvas text, filter
//! registrations and material backdrop groups go through the same engine and
//! surface calls on either.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use cherenkov_record::{Layer, MaterialRegistry, ResourceError, Transaction};
use rustc_hash::FxHashMap;
use waterui_core::contract_violation;
use waterui_graphics::backend::filtrate::Filter;
use waterui_graphics::backend::{Handle, TextBackend, TextRegistration, TextRequest};

use super::MaterialTerms;
use crate::renderer::material::MaterialChain;
use crate::renderer::mount::backdrop::{
    BackdropGroupKey, ChromeBackdropGroups, ChromeGroupKey, ChromeMemberPayload, ChromeParams,
    MaterialBackdropGroups, MaterialMembership, MemberBind, MemberJoin, Unbound,
};
use crate::renderer::recording::SceneResources;
use crate::text::TextService;
use crate::text::{CanvasLayout, shape_canvas_text};

/// Installs on `resources` the registry that lays canvas text out through
/// `text` and registers it with `engine`.
pub fn install_engine_text<B: cherenkov::Backend, E: crate::text::TextEngine>(
    resources: &SceneResources,
    engine: &Rc<cherenkov::Engine<B>>,
    text: &Arc<TextService<E>>,
) {
    resources.install_text(text, || {
        Rc::new(EngineText {
            engine: Rc::clone(engine),
            service: Arc::clone(text),
            faces: RefCell::default(),
        })
    });
}

/// Canvas text on a Cherenkov target: laid out by the session's text engine,
/// registered with the Cherenkov engine as glyph runs.
struct EngineText<B: cherenkov::Backend, E: crate::text::TextEngine> {
    engine: Rc<cherenkov::Engine<B>>,
    service: Arc<TextService<E>>,
    /// The faces text layouts have registered, by blob and index, so a text
    /// laid out again does not register its fonts again. Each layout holds
    /// its own share of a face; the cache holds one for as long as the
    /// registry lives.
    faces: RefCell<FxHashMap<(u64, u32), cherenkov::Font>>,
}

impl<B: cherenkov::Backend, E: crate::text::TextEngine> TextBackend for EngineText<B, E> {
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

impl super::FilterRegistration for cherenkov::Filter {
    fn filter_id(&self) -> cherenkov_record::FilterId {
        self.id()
    }
}

impl MaterialTerms<cherenkov::BackdropShader> {
    /// Attaches the registry to `engine` (water-rs/waterui#1788).
    pub(crate) fn resolve<B: cherenkov::BackdropShaders>(
        engine: &cherenkov::Engine<B>,
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
pub fn attach_material_shaders<B: cherenkov::BackdropShaders>(
    engine: &cherenkov::Engine<B>,
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

/// Joins `layer` — a `ChromeMaterial` member — to the chrome group `key`
/// names on `surface`, binding the member's backdrop shader. The body of
/// both Cherenkov targets' [`mount_chrome`](super::LayerTarget::mount_chrome).
///
/// A `surface` dropped during its own commit is a contract violation: the
/// layer joins no chrome group.
#[expect(
    clippy::too_many_arguments,
    reason = "the target's mount step forwarded whole; bundling them obscures the target contract"
)]
pub fn mount_chrome_group<B, E>(
    surface: &Weak<cherenkov::Surface<B>>,
    tx: &mut Transaction<'_, B>,
    groups: &mut ChromeBackdropGroups<cherenkov::BackdropGroup, cherenkov::BackdropShader, E>,
    layer: &Layer,
    key: ChromeGroupKey,
    params: ChromeParams,
    display_scale: f64,
    membership: &MaterialMembership<E>,
    payload: impl FnOnce() -> ChromeMemberPayload<cherenkov::BackdropShader>,
    bind: MemberBind,
) where
    B: cherenkov::Backend + cherenkov_record::Target + cherenkov::Backdrop,
    E: crate::text::TextEngine,
{
    if surface.upgrade().is_none() {
        contract_violation!(
            "hydrolysis materials: the engine surface was dropped during its commit; \
             the layer joins no chrome group"
        );
        groups.clear(layer.id());
        tx[layer].clear_backdrop();
        return;
    }
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
            |params: &ChromeParams, scale| chrome_group(surface, &key, params, scale),
            bind_chrome_sample::<B>,
        ),
        bind,
    );
}

/// Resolves the container groups whose membership or member recordings
/// changed in this commit (water-rs/waterui#2834), building any rebuilt
/// group on `surface`. The body of both Cherenkov targets'
/// [`settle_chrome`](super::LayerTarget::settle_chrome).
pub fn settle_chrome_groups<B, E>(
    surface: &Weak<cherenkov::Surface<B>>,
    tx: &mut Transaction<'_, B>,
    groups: &mut ChromeBackdropGroups<cherenkov::BackdropGroup, cherenkov::BackdropShader, E>,
    registry: &Rc<MaterialRegistry>,
    display_scale: f64,
) where
    B: cherenkov::Backend + cherenkov_record::Target + cherenkov::Backdrop,
    E: crate::text::TextEngine,
{
    groups.settle(
        tx,
        registry,
        display_scale,
        (
            |key: &ChromeGroupKey, params: &ChromeParams, scale| {
                chrome_group(surface, key, params, scale)
            },
            bind_chrome_sample::<B>,
        ),
        super::super::layers::NodeLayers::member_layer,
    );
}

/// The engine group a chrome group of `key` is built as on `surface` at
/// display `scale`, from `params`.
///
/// # Errors
/// [`Unbound`], reported, when the engine surface dropped or `params`
/// cannot be built at `scale` ([`chrome_spec`]).
fn chrome_group<B>(
    surface: &Weak<cherenkov::Surface<B>>,
    key: &ChromeGroupKey,
    params: &ChromeParams,
    scale: f64,
) -> Result<cherenkov::BackdropGroup, Unbound>
where
    B: cherenkov::Backend + cherenkov::Backdrop,
{
    let Some(surface) = surface.upgrade() else {
        contract_violation!(
            "hydrolysis materials: the engine surface was dropped during its commit; \
             the chrome group is not built"
        );
        return Err(Unbound);
    };
    Ok(surface.backdrop_group_unfiltered(chrome_spec(params, scale, *key)?))
}

/// Binds `member` to the chrome sample of `payload` on `group`.
fn bind_chrome_sample<B: cherenkov_record::BackdropSampling>(
    tx: &mut Transaction<'_, B>,
    member: &Layer,
    payload: &ChromeMemberPayload<cherenkov::BackdropShader>,
    group: &cherenkov::BackdropGroup,
    scale: f64,
) -> Result<(), Unbound> {
    tx[member].backdrop(chrome_sample(payload, group.id(), scale)?);
    Ok(())
}

/// The spec a chrome group of `key` is created with at display `scale`:
/// the class's capture scale, levels, pyramid and member blend space, its
/// union field converted at `scale`, and — for a scoped group — its scope's
/// anchor layer, beneath which it captures like a material group
/// (water-rs/waterui#2097).
///
/// # Errors
/// [`Unbound`], reported as [`union_of`] reports it, for a union smoothing
/// invalid at `scale`: the group is not built.
pub fn chrome_spec(
    params: &ChromeParams,
    scale: f64,
    key: ChromeGroupKey,
) -> Result<cherenkov::BackdropSpec, Unbound> {
    let capture = &params.capture;
    let mut spec = cherenkov::BackdropSpec::new(capture.scale, capture.levels)
        .pyramid(capture.pyramid)
        .blend_space(capture.blend_space);
    if let Some(anchor) = key.anchor() {
        spec = spec.anchor(anchor);
    }
    if let Some(union) = union_of(capture, scale, params.class)? {
        spec = spec.union(union);
    }
    Ok(spec)
}

/// The union field `params` declares for a group built at `scale`:
/// `None` for `Solo` and `Shared` classes; for `Union`, the class's
/// logical smoothing converted to device pixels — [`BackdropUnion`]'s
/// input — with its error surfaced, never clamped.
///
/// [`BackdropUnion`]: cherenkov::BackdropUnion
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
/// [`BackdropOuter`](cherenkov::BackdropOuter), reported through
/// [`contract_violation!`]: its error is surfaced, never clamped.
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

/// Joins `layer` to the material backdrop group `key` names on `surface`:
/// the group captures beneath its anchor, runs the level's
/// [`MaterialChain`] once, and every member under the same key samples the
/// result. The body of both Cherenkov targets'
/// [`mount_material`](super::LayerTarget::mount_material).
///
/// A `surface` dropped during its own commit is a contract violation: the
/// layer joins no backdrop group.
#[expect(
    clippy::too_many_arguments,
    reason = "the target's mount step forwarded whole; bundling them obscures the target contract"
)]
pub fn mount_material_group<B, E>(
    surface: &Weak<cherenkov::Surface<B>>,
    tx: &mut Transaction<'_, B>,
    groups: &mut MaterialBackdropGroups<cherenkov::BackdropGroup, E>,
    layer: &Layer,
    key: BackdropGroupKey,
    display_scale: f64,
    membership: &MaterialMembership<E>,
    bind: MemberBind,
) where
    B: cherenkov::Backend
        + cherenkov_record::Target
        + cherenkov::BackdropRuns<<MaterialChain as Filter>::Kind, MaterialChain>,
    E: crate::text::TextEngine,
{
    let Some(surface) = surface.upgrade() else {
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
            |tx: &mut Transaction<'_, B>,
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
