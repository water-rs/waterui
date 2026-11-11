//! What the GPU and CPU [`LayerTarget`](super::LayerTarget)s share: both are
//! the one Cherenkov front end on a different backend, so canvas text, filter
//! registrations and material backdrop groups go through the same engine and
//! surface calls on either.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use cherenkov_record::{Layer, MaterialRegistry, ResourceError, Transaction};
use rustc_hash::FxHashMap;
use waterui_graphics::backend::filtrate::Filter;
use waterui_graphics::backend::{Handle, TextBackend, TextRegistration, TextRequest};

use super::MaterialTerms;
use crate::renderer::material::MaterialChain;
use crate::renderer::mount::backdrop::{
    BackdropGroupKey, ChromeBackdropGroups, ChromeGroupKey, ChromeMemberPayload,
    MaterialBackdropGroups, MaterialMembership, MemberBind, MemberJoin,
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
/// group binds. A source the engine rejects panics at attach, naming the
/// key — the registry is the theme's own contract, so a rejection is a
/// programmer error.
pub fn attach_material_shaders<B: cherenkov::BackdropShaders>(
    engine: &cherenkov::Engine<B>,
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

/// Joins `layer` — a `ChromeMaterial` member — to the chrome group `key`
/// names on `surface`, binding the member's backdrop shader. The body of
/// both Cherenkov targets' [`mount_chrome`](super::LayerTarget::mount_chrome).
///
/// # Panics
/// Panics when `surface` was dropped during its own commit.
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
    params: cherenkov_record::MaterialCapture,
    display_scale: f64,
    membership: &MaterialMembership<E>,
    payload: impl FnOnce() -> ChromeMemberPayload<cherenkov::BackdropShader>,
    bind: MemberBind,
) where
    B: cherenkov::Backend + cherenkov_record::Target + cherenkov::Backdrop,
    E: crate::text::TextEngine,
{
    let surface = surface
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
            move |params, scale| surface.backdrop_group_unfiltered(chrome_spec(params, scale, key)),
            |tx: &mut Transaction<'_, B>,
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
/// [`BackdropUnion`]: cherenkov::BackdropUnion
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
/// [`BackdropOuter`](cherenkov::BackdropOuter): its error is surfaced,
/// never clamped.
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

/// Joins `layer` to the material backdrop group `key` names on `surface`:
/// the group captures beneath its anchor, runs the level's
/// [`MaterialChain`] once, and every member under the same key samples the
/// result. The body of both Cherenkov targets'
/// [`mount_material`](super::LayerTarget::mount_material).
///
/// # Panics
/// Panics when `surface` was dropped during its own commit.
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
    let surface = surface
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
            |tx: &mut Transaction<'_, B>,
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
