//! `MirrorTarget`: a layer target whose queue drains inline and applies
//! every change set to an in-process `SurfaceTree`, so a test reads the
//! layer tree a [`Mount`](crate::renderer::mount::Mount) committed — its
//! transforms, clips, opacities, bound producer sizes and backdrop
//! membership — without an engine frame.

use std::cell::{Ref, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use cherenkov::{
    FilterId, Layer, LayerContent, LayerId, LayerNode, Realize, Shared, SurfaceId, SurfaceTree,
    Transaction,
};
use cherenkov_record::ChangeSet;
use rustc_hash::FxHashMap;

use crate::engine::GpuEngine;
use crate::gpu_view::{ExternalFrameRuntime, GpuContentRuntime};
use crate::renderer::effects::{AppliedFilterMetrics, FilteredRuntime};
use crate::renderer::mount::layers::{LayerVisitor, visit};
use crate::renderer::mount::target::LayerTarget;
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

impl cherenkov::GpuInstalls for MirrorTarget {}

/// What the mirror's consumer side holds.
pub struct Mirrored {
    tree: SurfaceTree,
    installs: FxHashMap<LayerId, (u32, u32)>,
}

/// Applies each drained change set to the mirrored tree.
pub struct MirrorQueue {
    mirrored: Rc<RefCell<Mirrored>>,
}

impl cherenkov::Queue<MirrorTarget> for MirrorQueue {
    fn drains_inline(&self) -> bool {
        true
    }

    fn apply(&self, changes: ChangeSet<MirrorTarget>) {
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

    fn wake(&self) {}
}

/// The mirror's host: scene resources over the test GPU engine.
pub struct MirrorHost {
    resources: Rc<SceneResources>,
    engine: Rc<GpuEngine>,
    metrics: Arc<AppliedFilterMetrics>,
}

impl LayerTarget for MirrorTarget {
    type Host = MirrorHost;
    /// The mirror installs no backdrop group object — its table keeps the
    /// membership bookkeeping, which `backdrops()` reads.
    type Group = ();

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
        groups: &mut crate::renderer::mount::backdrop::BackdropGroups<()>,
        layer: &Layer,
        key: crate::renderer::mount::backdrop::BackdropGroupKey,
        display_scale: f64,
        membership: &crate::renderer::mount::backdrop::MaterialMembership,
    ) {
        groups.join(
            tx,
            layer,
            key,
            display_scale,
            membership,
            (|_runtime, _scale, _anchor| (), |_tx, _member, _group| {}),
        );
    }
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
    fn new() -> Self {
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
            engine,
            metrics: Arc::new(AppliedFilterMetrics::default()),
        };
        Self {
            mount: Mount::new(shared),
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
        let roots = self.mount_roots();
        let window = self.mirror.get_or_insert_with(MirrorWindow::new);
        let redraw = self.host_redraw_handle.clone();
        let core = &mut self.core;
        let window_material = core.window_material;
        let mut wakes = |cell: &Rc<crate::renderer::mount::cell::NodeCell>| {
            core.producer_wake(cell, redraw.clone())
        };
        window.mount.commit(
            &window.host,
            kurbo::Affine::IDENTITY,
            1.0,
            &roots,
            &mut wakes,
            window_material.as_ref(),
        );
        let stats = window.mount.take_stats();
        self.core.clear_commit_marks();
        crate::renderer::mount::layers::census(&roots, stats)
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
