//! `MirrorTarget`: a layer target whose queue drains inline and applies
//! every change set to an in-process `SurfaceTree`, so a test reads the
//! layer tree a [`Mount`](crate::renderer::mount::Mount) committed — its
//! transforms, clips, opacities, bound producer sizes and backdrop
//! membership — without an engine frame.

use std::cell::{Ref, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use cherenkov::{
    BackdropId, FilterId, Layer, LayerContent, LayerId, LayerNode, Realize, Shared, SurfaceId,
    SurfaceTree, Transaction,
};
use cherenkov_record::{ChangeSet, MaterialRegistry};
use rustc_hash::FxHashMap;

use crate::engine::GpuEngine;
use crate::gpu_view::{ExternalFrameRuntime, GpuContentRuntime};
use crate::renderer::effects::{AppliedFilterMetrics, FilteredRuntime};
use crate::renderer::mount::backdrop::{MemberBind, MemberJoin};
use crate::renderer::mount::layers::{LayerVisitor, NodeLayers, visit};
use crate::renderer::mount::target::{LayerTarget, MaterialTerms, attach_material_shaders};
use crate::renderer::mount::{Mount, MountStats};
use crate::renderer::recording::{Recording, SceneResources};
use crate::renderer::{HydrolysisRenderer, ProducerWake};

/// The test target.
pub struct MirrorTarget;

impl cherenkov::Target for MirrorTarget {
    type Queue = MirrorQueue;
    /// The pixel size a producer was bound at.
    type Install = (u32, u32);
}

impl std::fmt::Debug for MirrorTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MirrorTarget").finish_non_exhaustive()
    }
}

impl cherenkov::GpuInstalls for MirrorTarget {}

impl cherenkov::BackdropSampling for MirrorTarget {}

/// What the mirror's consumer side holds.
pub struct Mirrored {
    tree: SurfaceTree,
    installs: FxHashMap<LayerId, (u32, u32)>,
}

/// Applies each drained change set to the mirrored tree.
pub struct MirrorQueue {
    mirrored: Rc<RefCell<Mirrored>>,
}

impl std::fmt::Debug for MirrorQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MirrorQueue").finish_non_exhaustive()
    }
}

impl MirrorQueue {
    fn apply_changes<T: cherenkov::Target<Install = (u32, u32)>>(&self, changes: ChangeSet<T>) {
        let mut mirrored = self.mirrored.borrow_mut();
        for op in changes.ops {
            match mirrored.tree.apply_op(op) {
                Realize::Remove(layer) => {
                    mirrored.installs.remove(&layer);
                }
                Realize::Install(layer, install) => {
                    mirrored.installs.insert(layer, install.into_inner());
                }
                Realize::Content(..) | Realize::Applied => {}
            }
        }
    }
}

impl cherenkov::Queue<MirrorTarget> for MirrorQueue {
    fn drains_inline(&self) -> bool {
        true
    }

    fn apply(&self, changes: ChangeSet<MirrorTarget>) {
        self.apply_changes(changes);
    }

    fn wake(&self) {}
}

impl cherenkov::Queue<NoShaderTarget> for MirrorQueue {
    fn drains_inline(&self) -> bool {
        true
    }

    fn apply(&self, changes: ChangeSet<NoShaderTarget>) {
        self.apply_changes(changes);
    }

    fn wake(&self) {}
}

/// The mirror's host: scene resources over the test GPU engine.
pub struct MirrorHost {
    resources: Rc<SceneResources>,
    engine: Rc<GpuEngine>,
    metrics: Arc<AppliedFilterMetrics>,
    materials: MaterialTerms<MirrorTarget>,
    /// Each chrome group's union field at creation — what
    /// [`mount::target::union_of`] resolved for the key's class at the
    /// group's display scale. `None` for a `Solo`/`Shared` class.
    union_log: RefCell<Vec<Option<cherenkov::BackdropUnion>>>,
}

impl std::fmt::Debug for MirrorHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MirrorHost").finish_non_exhaustive()
    }
}

impl LayerTarget for MirrorTarget {
    type Host = MirrorHost;
    /// The mirror's chrome-group object is a synthetic `BackdropId` the
    /// sample binds — the install a member commits carries it, so a test
    /// reads membership and identity off the mirrored tree.
    type Group = BackdropId;
    /// The mirror's engine realizes real backdrop shaders.
    type Shader = cherenkov::BackdropShader;
    const BACKDROP_SHADERS: bool = true;

    fn resources(host: &MirrorHost) -> &Rc<SceneResources> {
        &host.resources
    }

    fn mount_gpu_content(
        _host: &MirrorHost,
        tx: &mut Transaction<'_, Self>,
        layer: &Layer,
        runtime: &mut GpuContentRuntime,
        pixels: (u32, u32),
        _wake: ProducerWake,
    ) {
        let binding = (layer.id(), pixels);
        if runtime.binding != Some(binding) {
            tx[layer].content(LayerContent::install(pixels));
            runtime.binding = Some(binding);
        }
    }

    fn mount_external_frame(
        _host: &MirrorHost,
        tx: &mut Transaction<'_, Self>,
        layer: &Layer,
        runtime: &mut ExternalFrameRuntime,
        _wake: ProducerWake,
    ) -> Option<(u32, u32)> {
        let pixels = runtime.frame_pixels?;
        let binding = (layer.id(), pixels);
        if runtime.binding != Some(binding) {
            tx[layer].content(LayerContent::install(pixels));
            runtime.binding = Some(binding);
        }
        Some(pixels)
    }

    fn filter(host: &MirrorHost, runtime: &mut FilteredRuntime) -> FilterId {
        runtime.filter(&host.engine, &host.metrics).id()
    }

    fn mount_material(
        _host: &MirrorHost,
        tx: &mut Transaction<'_, Self>,
        groups: &mut crate::renderer::mount::backdrop::MaterialBackdropGroups<BackdropId>,
        layer: &Layer,
        key: crate::renderer::mount::backdrop::BackdropGroupKey,
        display_scale: f64,
        membership: &crate::renderer::mount::backdrop::MaterialMembership,
        bind: MemberBind,
    ) {
        // The mirror's material bind installs nothing of its own.
        groups.join(
            tx,
            key,
            key.runtime(),
            display_scale,
            MemberJoin {
                layer,
                membership,
                payload: || (),
                resolve: NodeLayers::frame_layer,
            },
            (
                |_runtime, _scale| key_id(&key),
                |_tx, _member, _p: &(), _group, _scale| {},
            ),
            bind,
        );
    }

    fn material_terms(host: &MirrorHost) -> &MaterialTerms<Self> {
        &host.materials
    }

    fn mount_chrome(
        host: &MirrorHost,
        tx: &mut Transaction<'_, Self>,
        groups: &mut crate::renderer::mount::backdrop::ChromeBackdropGroups<
            BackdropId,
            cherenkov::BackdropShader,
        >,
        layer: &Layer,
        key: crate::renderer::mount::backdrop::ChromeGroupKey,
        params: cherenkov_record::MaterialCapture,
        display_scale: f64,
        membership: &crate::renderer::mount::backdrop::MaterialMembership,
        payload: impl FnOnce() -> crate::renderer::mount::backdrop::ChromeMemberPayload<
            cherenkov::BackdropShader,
        >,
        bind: MemberBind,
    ) {
        // The mirror's group id is the id its members' samples carry: the
        // install binds the member's own payload through the GPU target's
        // `chrome_sample`.
        groups.join(
            tx,
            key,
            params,
            display_scale,
            MemberJoin {
                layer,
                membership,
                payload,
                resolve: NodeLayers::member_layer,
            },
            (
                move |params: &cherenkov_record::MaterialCapture, scale| {
                    // The same union conversion the GPU target runs; a
                    // test reads the device-pixel result off the log.
                    let mut log = host.union_log.borrow_mut();
                    log.push(crate::renderer::mount::target::union_of(
                        params,
                        scale,
                        key.class(),
                    ));
                    // Every group built gets an id of its own, as the
                    // engine's do: a member left on a released group
                    // carries an id no live group has.
                    key_id(&(key, log.len()))
                },
                |tx: &mut Transaction<'_, Self>,
                 member: &Layer,
                 payload: &crate::renderer::mount::backdrop::ChromeMemberPayload<
                    cherenkov::BackdropShader,
                >,
                 id: &BackdropId,
                 scale: f64| {
                    tx[member].backdrop(crate::renderer::mount::target::chrome_sample(
                        payload, *id, scale,
                    ));
                },
            ),
            bind,
        );
    }

    fn clear_chrome(tx: &mut Transaction<'_, Self>, layer: &Layer) {
        tx[layer].clear_backdrop();
    }
}

/// A synthetic group id for a mirror group: stable across the group's
/// rebuilds and distinct from any other key's. The top bit keeps it clear
/// of ids a real surface allocates.
fn key_id(key: &impl std::hash::Hash) -> BackdropId {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(key, &mut hasher);
    BackdropId::new(hasher.finish() | (1 << 63))
}

thread_local! {
    static ENGINE: Rc<GpuEngine> = Rc::new(
        GpuEngine::new(cherenkov_gpu::GpuConfig::default())
            .expect("hydrolysis tests: failed to create the test GPU engine"),
    );
}

/// A renderer's mirror mount and the tree it commits into.
pub struct MirrorWindow {
    mount: Mount<MirrorTarget>,
    host: MirrorHost,
    mirrored: Rc<RefCell<Mirrored>>,
}

impl std::fmt::Debug for MirrorWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MirrorWindow").finish_non_exhaustive()
    }
}

impl MirrorWindow {
    fn new(registry: &Rc<MaterialRegistry>) -> Self {
        let mirrored = Rc::new(RefCell::new(Mirrored {
            tree: SurfaceTree::new(),
            installs: FxHashMap::default(),
        }));
        let shared = Rc::new(RefCell::new(Shared::new(
            SurfaceId::new(1),
            MirrorQueue {
                mirrored: Rc::clone(&mirrored),
            },
        )));
        let engine = ENGINE.with(Rc::clone);
        let host = MirrorHost {
            resources: Rc::new(SceneResources::new(&engine)),
            engine: Rc::clone(&engine),
            metrics: Arc::new(AppliedFilterMetrics::default()),
            materials: MaterialTerms {
                registry: Rc::clone(registry),
                shaders: attach_material_shaders(&engine, registry),
            },
            union_log: RefCell::new(Vec::new()),
        };
        Self {
            mount: Mount::new(shared, registry),
            host,
            mirrored,
        }
    }

    /// The layer path from the window layer down to `id`, inclusive.
    fn path(&self, id: LayerId) -> Vec<LayerId> {
        fn find(tree: &SurfaceTree, at: LayerId, id: LayerId, path: &mut Vec<LayerId>) -> bool {
            path.push(at);
            if at == id {
                return true;
            }
            for &child in &tree.layer(at).children {
                if find(tree, child, id, path) {
                    return true;
                }
            }
            path.pop();
            false
        }
        let mut path = Vec::new();
        let found = find(
            &self.mirrored.borrow().tree,
            self.mount.window().id(),
            id,
            &mut path,
        );
        assert!(found, "hydrolysis mirror: {id:?} is not under the window");
        path
    }

    /// `id`'s content space in window points: every `content_transform`
    /// from the window layer down.
    pub fn world(&self, id: LayerId) -> kurbo::Affine {
        let mirrored = self.mirrored.borrow();
        self.path(id)
            .into_iter()
            .fold(kurbo::Affine::IDENTITY, |world, layer| {
                world * mirrored.tree.layer(layer).content_transform()
            })
    }

    /// The layers `id` composites under, window layer first, `id` last.
    pub fn ancestry(&self, id: LayerId) -> Vec<Ref<'_, LayerNode>> {
        self.path(id)
            .into_iter()
            .map(|layer| {
                Ref::map(self.mirrored.borrow(), |mirrored| {
                    mirrored.tree.layer(layer)
                })
            })
            .collect()
    }

    /// Every bound producer, by layer, with the pixel size it is bound at.
    pub fn installs(&self) -> Vec<(LayerId, (u32, u32))> {
        let mut installs: Vec<_> = self
            .mirrored
            .borrow()
            .installs
            .iter()
            .map(|(&layer, &pixels)| (layer, pixels))
            .collect();
        installs.sort_by_key(|(layer, _)| layer.raw());
        installs
    }

    /// Every backdrop member, by layer, with its group's display scale.
    pub fn backdrops(&self) -> Vec<(LayerId, f64)> {
        self.mount.groups().member_scales()
    }

    /// The theme's material terms on the mirror's engine.
    pub fn materials(&self) -> &MaterialTerms<MirrorTarget> {
        &self.host.materials
    }

    /// The union field each chrome group was created with, in creation
    /// order — `None` for `Solo`/`Shared` classes.
    pub fn union_log(&self) -> Vec<Option<cherenkov::BackdropUnion>> {
        self.host.union_log.borrow().clone()
    }

    /// The mirror's chrome group table.
    pub fn chrome_groups(
        &self,
    ) -> &crate::renderer::mount::backdrop::ChromeBackdropGroups<
        BackdropId,
        cherenkov::BackdropShader,
    > {
        self.mount.chrome_groups()
    }

    /// The member's parent layer's committed children, in paint order.
    pub fn siblings(&self, id: LayerId) -> Vec<LayerId> {
        let path = self.path(id);
        let parent = path[path.len() - 2];
        self.mirrored.borrow().tree.layer(parent).children.clone()
    }
}

/// Runs `f` on the test engine — engine-level checks that need no window.
pub fn mirror_engine<T>(f: impl FnOnce(&Rc<GpuEngine>) -> T) -> T {
    ENGINE.with(|engine| f(engine))
}

/// A layer target whose engine lacks backdrop shaders — the CPU-engine
/// attach rule (water-rs/waterui#1788) mounts against it.
pub struct NoShaderTarget;

impl cherenkov::Target for NoShaderTarget {
    type Queue = MirrorQueue;
    type Install = (u32, u32);
}

impl std::fmt::Debug for NoShaderTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoShaderTarget").finish_non_exhaustive()
    }
}

/// The no-shader target's host: the mirror's engine terms with no shader
/// handles — `Shader` is `()`.
pub struct NoShaderHost {
    resources: Rc<SceneResources>,
    engine: Rc<GpuEngine>,
    metrics: Arc<AppliedFilterMetrics>,
    materials: MaterialTerms<NoShaderTarget>,
}

impl std::fmt::Debug for NoShaderHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoShaderHost").finish_non_exhaustive()
    }
}

/// A `Mount<NoShaderTarget>` under construction against `registry` — the
/// CPU-engine attach rule's panic lives in `Mount::new`.
pub fn no_shader_mount(registry: &MaterialRegistry) -> Mount<NoShaderTarget> {
    let shared = Rc::new(RefCell::new(Shared::new(
        SurfaceId::new(2),
        MirrorQueue {
            mirrored: Rc::new(RefCell::new(Mirrored {
                tree: SurfaceTree::new(),
                installs: FxHashMap::default(),
            })),
        },
    )));
    Mount::new(shared, registry)
}

impl LayerTarget for NoShaderTarget {
    type Host = NoShaderHost;
    type Group = ();
    type Shader = ();
    const BACKDROP_SHADERS: bool = false;

    fn resources(host: &NoShaderHost) -> &Rc<SceneResources> {
        &host.resources
    }

    fn mount_gpu_content(
        _host: &NoShaderHost,
        _tx: &mut Transaction<'_, Self>,
        _layer: &Layer,
        _runtime: &mut GpuContentRuntime,
        _pixels: (u32, u32),
        _wake: ProducerWake,
    ) {
    }

    fn mount_external_frame(
        _host: &NoShaderHost,
        _tx: &mut Transaction<'_, Self>,
        _layer: &Layer,
        _runtime: &mut ExternalFrameRuntime,
        _wake: ProducerWake,
    ) -> Option<(u32, u32)> {
        None
    }

    fn filter(host: &NoShaderHost, runtime: &mut FilteredRuntime) -> FilterId {
        runtime.filter(&host.engine, &host.metrics).id()
    }

    fn mount_material(
        _host: &NoShaderHost,
        _tx: &mut Transaction<'_, Self>,
        _groups: &mut crate::renderer::mount::backdrop::MaterialBackdropGroups<()>,
        _layer: &Layer,
        _key: crate::renderer::mount::backdrop::BackdropGroupKey,
        _display_scale: f64,
        _membership: &crate::renderer::mount::backdrop::MaterialMembership,
        _bind: MemberBind,
    ) {
    }

    fn material_terms(host: &NoShaderHost) -> &MaterialTerms<Self> {
        &host.materials
    }

    fn mount_chrome(
        _host: &NoShaderHost,
        _tx: &mut Transaction<'_, Self>,
        _groups: &mut crate::renderer::mount::backdrop::ChromeBackdropGroups<(), ()>,
        _layer: &Layer,
        _key: crate::renderer::mount::backdrop::ChromeGroupKey,
        _params: cherenkov_record::MaterialCapture,
        _display_scale: f64,
        _membership: &crate::renderer::mount::backdrop::MaterialMembership,
        _payload: impl FnOnce() -> crate::renderer::mount::backdrop::ChromeMemberPayload<()>,
        _bind: MemberBind,
    ) {
    }
}

/// Flattens every committed run into one window-space recording.
struct Painted(Recording);

impl LayerVisitor for Painted {
    fn run(&mut self, world: kurbo::Affine, recording: &Recording) {
        self.0.append(recording, world);
    }

    fn open_clip(&mut self, world: kurbo::Affine, clip: &cherenkov::ShapeData) {
        self.0.push_clip_data(world, clip.clone());
    }

    fn close_clip(&mut self) {
        self.0.pop_scope();
    }
}

impl HydrolysisRenderer {
    /// Commits the window's host frames into the renderer's mirror mount
    /// and returns that commit's stats with the mounted layers' census.
    pub fn commit_mirror(&mut self) -> MountStats {
        self.commit_mirror_at(1.0)
    }

    /// [`commit_mirror`](Self::commit_mirror) at `scale` device pixels per
    /// point: the display scale the mount builds its backdrop terms for.
    pub fn commit_mirror_at(&mut self, scale: f64) -> MountStats {
        let roots = self.mount_roots();
        let registry = Rc::clone(&self.material_registry);
        let window = self
            .mirror
            .get_or_insert_with(|| MirrorWindow::new(&registry));
        let redraw = self.host_redraw_handle.clone();
        let core = &mut self.core;
        let window_material = core.window_material;
        let mut wakes = |cell: &Rc<crate::renderer::mount::cell::NodeCell>| {
            core.producer_wake(cell, redraw.clone())
        };
        window.mount.commit(
            &window.host,
            kurbo::Affine::IDENTITY,
            scale,
            &roots,
            &mut wakes,
            window_material.as_ref(),
        );
        let stats = window.mount.take_stats();
        self.core.clear_commit_marks();
        crate::renderer::mount::layers::census(&roots, stats)
    }

    /// Drops the mirror mount, so the next
    /// [`commit_mirror`](Self::commit_mirror) commits into a fresh one —
    /// a device remount: every cell re-lowers the program it last
    /// lowered, on a mount its layers never saw.
    pub fn remount_mirror(&mut self) {
        self.mirror = None;
    }

    /// The mirror mount [`commit_mirror`](Self::commit_mirror) committed.
    ///
    /// # Panics
    ///
    /// Panics when read before the first `commit_mirror`.
    pub fn mirror(&self) -> &MirrorWindow {
        self.mirror
            .as_ref()
            .expect("hydrolysis tests: read the mirror after commit_mirror")
    }

    /// Everything the committed layers paint, in window points.
    pub fn painted_scene(&self) -> Recording {
        let mut painted = Painted(Recording::default());
        visit(&self.mount_roots(), &mut painted);
        painted.0
    }
}
