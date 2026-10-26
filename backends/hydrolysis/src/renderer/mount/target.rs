//! The target seam a [`Mount`](super::Mount) commits through (§A.1): what
//! a node's layers need from the engine beyond recorded content.

use std::rc::Rc;

use cherenkov_record::{FilterId, Layer, MaterialRegistry, Transaction};
use rustc_hash::FxHashMap;
use waterui_graphics::Reactive;
use waterui_graphics::filtrate::filters::OverscrollStretch;

#[cfg(hydrolysis_gpu)]
use crate::gpu_view::{ExternalFrameRuntime, GpuContentRuntime};
#[cfg(hydrolysis_gpu)]
use crate::renderer::ProducerWake;
use crate::renderer::effects::FilteredRuntime;
use crate::renderer::mount::backdrop::{
    BackdropGroupKey, ChromeBackdropGroups, ChromeGroupKey, ChromeMemberPayload,
    MaterialBackdropGroups, MaterialMembership, MemberBind,
};
use crate::renderer::recording::SceneResources;

/// A layer target: recorded runs land through `Transaction`, and the
/// producer, filter and backdrop installs through the target's host.
pub trait LayerTarget<E: crate::text::TextEngine>: cherenkov_record::Target {
    /// What the target installs against (engine, resources, devices).
    type Host;
    /// The backdrop-group object the mount's [`MaterialBackdropGroups`]
    /// and [`ChromeBackdropGroups`] tables hold — on the GPU target a
    /// [`cherenkov::BackdropGroup`], whose drop unregisters it.
    type Group: 'static;

    /// The engine's backdrop-shader handle the material registry's
    /// sources resolve to at attach — `cherenkov::BackdropShader` on the
    /// GPU target.
    type Shader: Clone + 'static;

    /// Whether the target's engine realizes backdrop shaders at all.
    /// `false` — a CPU engine — makes a [`Mount`](super::Mount::new)
    /// attach panic when the theme's material registry is not empty
    /// (water-rs/waterui#1788).
    const BACKDROP_SHADERS: bool;

    fn resources(host: &Self::Host) -> &Rc<SceneResources>;

    /// Binds hosted content at `size` on `layer`; returns whether this call
    /// (re)bound it. Only a target with system-compositor planes hosts
    /// platform objects, so reaching this anywhere else is a build bug.
    #[cfg(all(hydrolysis_hosted, hydrolysis_gpu))]
    fn mount_hosted(
        _tx: &mut Transaction<'_, Self>,
        _layer: &Layer,
        _runtime: &crate::hosted::HostedRuntime,
        _size: kurbo::Size,
    ) -> bool {
        panic!("hydrolysis: this target cannot mount hosted content");
    }

    /// Binds a GPU content producer at `pixels` on `layer`.
    #[cfg(hydrolysis_gpu)]
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
    #[cfg(hydrolysis_gpu)]
    fn mount_external_frame(
        host: &Self::Host,
        tx: &mut Transaction<'_, Self>,
        layer: &Layer,
        runtime: &mut ExternalFrameRuntime,
        wake: ProducerWake,
    ) -> Option<(u32, u32)>;

    fn filter(host: &Self::Host, runtime: &mut FilteredRuntime) -> FilterId;

    /// Registers the overscroll stretch a scroll node's inner layer draws
    /// with; the returned handle's drop unregisters it.
    fn stretch_filter(host: &Self::Host, filter: OverscrollStretch<Reactive>) -> FilterHandle;

    /// Joins `layer` to the backdrop group `key` names, sharing the
    /// group's filtered capture with every member under the same key,
    /// rebuilding it when `display_scale` changes. The membership the
    /// layer's node holds ends with its layers; its owner link is how a
    /// rebuild re-points every member at the replacement group. `bind`
    /// says when the layer's sample binds.
    #[expect(
        clippy::too_many_arguments,
        reason = "one mount step per parameter; bundling them obscures the target contract"
    )]
    fn mount_material(
        host: &Self::Host,
        tx: &mut Transaction<'_, Self>,
        groups: &mut MaterialBackdropGroups<Self::Group, E>,
        layer: &Layer,
        key: BackdropGroupKey,
        display_scale: f64,
        membership: &MaterialMembership<E>,
        bind: MemberBind,
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
    fn material_terms(host: &Self::Host) -> &MaterialTerms<Self::Shader>;

    /// Joins `layer` — a `ChromeMaterial` member — to the chrome group
    /// `key` names, sharing the group's unfiltered capture with every
    /// member under the same `(scope, class, canvas)` key, rebuilding it
    /// when `display_scale` changes. `params` is the class's registered
    /// capture terms; `payload` builds the member's own shader handle and
    /// live effect — the terms the member binds and a group rebuild
    /// rebinds — and runs only when `bind` binds the member
    /// (water-rs/waterui#1788).
    #[expect(
        clippy::too_many_arguments,
        reason = "one install hands the group's whole context at once"
    )]
    fn mount_chrome(
        host: &Self::Host,
        tx: &mut Transaction<'_, Self>,
        groups: &mut ChromeBackdropGroups<Self::Group, Self::Shader, E>,
        layer: &Layer,
        key: ChromeGroupKey,
        params: cherenkov_record::MaterialCapture,
        display_scale: f64,
        membership: &MaterialMembership<E>,
        payload: impl FnOnce() -> ChromeMemberPayload<Self::Shader>,
        bind: MemberBind,
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
pub struct MaterialTerms<S> {
    /// The theme's registry: capture classes resolve at member install.
    pub registry: Rc<MaterialRegistry>,
    /// Each registered shader's engine handle, filled at attach by
    /// registering its source with the engine.
    pub shaders: FxHashMap<cherenkov_record::MaterialShader, S>,
}
impl<S> std::fmt::Debug for MaterialTerms<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaterialTerms")
            .field("registry", &self.registry)
            .field("shaders", &self.shaders.keys())
            .finish()
    }
}

/// A registered filter the mount retains: the registration's identity and
/// its ownership — dropping the handle drops the registration.
#[cfg_attr(hydrolysis_hwui, expect(dead_code))]
pub trait FilterRegistration: 'static {
    /// The id the layer names this registration by.
    fn filter_id(&self) -> cherenkov_record::FilterId;
}

/// A mount's hold on one filter registration: the id the layer reports and
/// the registration's ownership. The id comes from the typed registration
/// at construction — an id can never be paired with an unrelated owner, so
/// the layer and the lifetime can never disagree.
pub struct FilterHandle {
    id: cherenkov_record::FilterId,
    _registration: Box<dyn std::any::Any>,
}

impl FilterHandle {
    /// The handle on `registration`; the mount keeps it until its layer
    /// dies, unregistering the filter.
    #[cfg_attr(hydrolysis_hwui, expect(dead_code))]
    pub fn new<F: FilterRegistration>(registration: F) -> Self {
        Self {
            id: registration.filter_id(),
            _registration: Box::new(registration),
        }
    }

    /// The registration's id — what a layer's filter field names.
    pub const fn id(&self) -> cherenkov_record::FilterId {
        self.id
    }
}

impl std::fmt::Debug for FilterHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilterHandle")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
#[cfg(hydrolysis_gpu_painter)]
#[path = "gpu_target.rs"]
mod gpu;

#[cfg(hydrolysis_gpu_painter)]
pub use gpu::CherenkovHost;
#[cfg(all(test, hydrolysis_gpu_painter))]
pub use gpu::{attach_material_shaders, chrome_sample, chrome_spec};
