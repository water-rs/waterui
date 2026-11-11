//! What the GPU and CPU [`LayerTarget`](super::LayerTarget)s share: both are
//! the one Cherenkov front end on a different backend, so canvas text, filter
//! registrations and material backdrop groups go through the same engine and
//! surface calls on either.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use cherenkov_record::{Layer, ResourceError, Transaction};
use rustc_hash::FxHashMap;
use waterui_core::contract_violation;
use waterui_graphics::backend::filtrate::Filter;
use waterui_graphics::backend::{Handle, TextBackend, TextRegistration, TextRequest};

use crate::renderer::material::MaterialChain;
use crate::renderer::mount::backdrop::{
    BackdropGroupKey, MaterialBackdropGroups, MaterialMembership, MemberBind, MemberJoin,
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
