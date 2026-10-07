//! Retained engine layers per node, and the one [`Mount`] per renderer
//! that lowers node programs into them (§A, §C).
//!
//! A node's frame is one engine layer whose transform is its placement
//! delta under its anchor; its program's runs, child frames and scopes
//! become the frame's ordered children. Layers are matched by key across
//! commits — runs by the item they precede, scopes by [`ScopeKey`] — so an
//! unchanged structure emits no tree ops. A dropped node's layers queue
//! their own removal; nothing drops a layer inside a transaction body:
//! retired layers wait in the mount's graveyard until the body returns.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};

use cherenkov::{FilterId, Layer, LayerId, ShapeData, Shared, Transaction};

use waterui_graphics::HeldResources;

use super::cell::NodeCell;
use super::placement::Placement;
use super::program::{
    InnerProgram, Item, MaterialRequest, ProducerContent, Program, RunKey, SceneContentSource,
    ScopeKey, ScopeProps,
};
use super::target::LayerTarget;
use crate::renderer::ProducerWake;

/// The liveness token of one node's layers: parents list children by
/// `(LayerId, Weak<LayerToken>)`, so a dropped child is told apart from a
/// live one without a back-pointer.
pub struct LayerToken {
    owner: Weak<NodeCell>,
}

/// A node's retained state, owned by its `NodeCore`.
#[derive(Default)]
pub struct NodeRetained {
    pub(crate) layers: RefCell<Option<NodeLayers>>,
    /// The finished record the next commit lowers.
    pub(crate) pending: RefCell<Option<Program>>,
    /// The program the layers were last lowered from, lowered again when
    /// the node remounts on a new mount (device or context replacement).
    pub(crate) lowered: RefCell<Option<Program>>,
    /// The placement whose layer this node's frame is a child of (`None`
    /// for frames attached directly to the window layer).
    pub(crate) anchor: RefCell<Option<Rc<Placement>>>,
}

impl NodeRetained {
    /// Stages a freshly recorded program for the next commit. The program
    /// last lowered is dropped with it, so the cells it listed live only as
    /// long as the tree keeps them.
    pub(crate) fn stage(&self, program: Program) {
        *self.pending.borrow_mut() = Some(program);
        self.lowered.borrow_mut().take();
    }
}

type Committed = Vec<(LayerId, Weak<LayerToken>)>;

/// The props last written to one layer.
#[derive(Clone, PartialEq)]
struct LayerProps {
    transform: kurbo::Affine,
    opacity: f32,
    clip: Option<ShapeData>,
    filter: Option<FilterId>,
    scroll: kurbo::Vec2,
}

impl LayerProps {
    const DEFAULT: Self = Self {
        transform: kurbo::Affine::IDENTITY,
        opacity: 1.0,
        clip: None,
        filter: None,
        scroll: kurbo::Vec2::ZERO,
    };
}

fn write_props<T: LayerTarget>(
    tx: &mut Transaction<'_, T>,
    layer: &Layer,
    written: &mut LayerProps,
    props: LayerProps,
) {
    if *written == props {
        return;
    }
    let edit = &mut tx[layer];
    if written.transform != props.transform {
        edit.transform(props.transform);
    }
    if written.opacity.to_bits() != props.opacity.to_bits() {
        edit.opacity(props.opacity);
    }
    if written.clip != props.clip {
        match &props.clip {
            Some(clip) => edit.clip(clip.clone()),
            None => edit.clear_clip(),
        };
    }
    if written.filter != props.filter {
        match props.filter {
            Some(filter) => edit.filter(filter),
            None => edit.clear_filter(),
        };
    }
    if written.scroll != props.scroll {
        edit.scroll_offset(props.scroll);
    }
    *written = props;
}

/// One run of recorded content. The run at the head of a layer's list
/// with no leading child is that layer's own content (`layer: None`).
pub struct RunLayer {
    key: RunKey,
    layer: Option<Layer>,
    held: Option<HeldResources>,
    /// The run's recording, kept for the tests' window-space readback.
    #[cfg(test)]
    recording: Option<crate::renderer::recording::Recording>,
}

/// A widget-internal scope's layer.
pub struct ScopeLayer {
    key: ScopeKey,
    layer: Layer,
    placement: Rc<Placement>,
    props: LayerProps,
    runs: Vec<RunLayer>,
    committed: Committed,
}

/// A node's retained layers.
pub struct NodeLayers {
    token: Rc<LayerToken>,
    /// The mount generation the layers belong to: a remount drops them.
    mount: u64,
    frame: Layer,
    frame_props: LayerProps,
    /// The scroll node's clipped, scrolled content layer.
    inner: Option<(Layer, LayerProps)>,
    /// A producer node's install layer.
    install: Option<(Layer, LayerProps)>,
    /// Whether `install` holds a GPU producer (the census counts them).
    install_gpu: bool,
    /// The material runtime the frame was last committed with.
    #[cfg(test)]
    material_runtime: Option<Rc<crate::renderer::material::MaterialRuntime>>,
    runs: Vec<RunLayer>,
    inner_runs: Vec<RunLayer>,
    scopes: Vec<ScopeLayer>,
    committed: Committed,
    inner_committed: Committed,
    content_held: Option<HeldResources>,
    /// The target's backdrop state (`LayerTarget::Material`).
    material: Option<Box<dyn Any>>,
    /// The layer the frame is attached under.
    attached: Cell<Option<LayerId>>,
}

impl NodeLayers {
    /// The target's material state.
    #[cfg(test)]
    pub const fn material_runtime(
        &self,
    ) -> Option<&Rc<crate::renderer::material::MaterialRuntime>> {
        self.material_runtime.as_ref()
    }

    #[cfg(test)]
    pub fn material<M: 'static>(&self) -> Option<&M> {
        self.material.as_ref().map(|material| {
            material
                .downcast_ref::<M>()
                .expect("hydrolysis layers: material state of another target")
        })
    }
}

/// Engine work one commit did.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MountStats {
    pub created: u64,
    pub removed: u64,
    pub installs: u64,
    /// Live layers holding drawn content: runs, scene content and installs.
    pub scene_layers: u32,
    /// Live layers holding recorded runs.
    pub scene_segments: u32,
    /// Live GPU-content installs.
    pub gpu_content: u32,
    /// Live frames carrying a filter.
    pub filtered: u32,
    /// Live clipped layers.
    pub clip_layers: u32,
    /// The deepest nesting of clipped layers.
    pub max_clip_depth: u32,
}

static NEXT_MOUNT: AtomicU64 = AtomicU64::new(1);

/// The renderer's one mount on a layer target: the window layer under the
/// surface root, and the commit that lowers node programs (§A).
pub struct Mount<T: LayerTarget> {
    id: u64,
    shared: Rc<RefCell<Shared<T>>>,
    root: Layer,
    window: Layer,
    window_props: LayerProps,
    window_committed: Committed,
    window_token: Rc<LayerToken>,
    attached: bool,
    stats: MountStats,
}

/// The state one commit threads through the node walk.
pub struct CommitCx<'m, 'tx, 's, T: LayerTarget> {
    pub tx: &'tx mut Transaction<'s, T>,
    pub host: &'m T::Host,
    shared: &'m Rc<RefCell<Shared<T>>>,
    mount: u64,
    window_transform: kurbo::Affine,
    display_scale: f64,
    graveyard: &'m mut Vec<Box<dyn Any>>,
    stats: &'m mut MountStats,
    wakes: &'m mut dyn FnMut(&Rc<NodeCell>) -> ProducerWake,
}

impl<T: LayerTarget> CommitCx<'_, '_, '_, T> {
    fn layer(&mut self) -> Layer {
        self.stats.created += 1;
        Shared::layer(self.shared)
    }

    fn retire_layer(&mut self, layer: Layer) {
        self.stats.removed += 1;
        self.graveyard.push(Box::new(layer));
    }

    fn retire_run(&mut self, run: RunLayer) {
        if let Some(layer) = run.layer {
            self.retire_layer(layer);
        }
        self.graveyard.push(Box::new(run.held));
    }

    fn retire_scope(&mut self, scope: ScopeLayer) {
        for run in scope.runs {
            self.retire_run(run);
        }
        self.retire_layer(scope.layer);
    }

    fn retire_layers(&mut self, layers: NodeLayers) {
        let NodeLayers {
            frame,
            inner,
            install,
            runs,
            inner_runs,
            scopes,
            content_held,
            material,
            ..
        } = layers;
        for run in runs.into_iter().chain(inner_runs) {
            self.retire_run(run);
        }
        for scope in scopes {
            self.retire_scope(scope);
        }
        for (layer, _) in inner.into_iter().chain(install) {
            self.retire_layer(layer);
        }
        self.retire_layer(frame);
        self.graveyard.push(Box::new((content_held, material)));
    }
}

impl<T: LayerTarget> Mount<T> {
    pub fn new(shared: Rc<RefCell<Shared<T>>>) -> Self {
        let root = Shared::root(&shared);
        let window = Shared::layer(&shared);
        Self {
            id: NEXT_MOUNT.fetch_add(1, Ordering::Relaxed),
            shared,
            root,
            window,
            window_props: LayerProps::DEFAULT,
            window_committed: Vec::new(),
            window_token: Rc::new(LayerToken { owner: Weak::new() }),
            attached: false,
            stats: MountStats {
                created: 1,
                ..MountStats::default()
            },
        }
    }

    #[cfg(test)]
    pub const fn window(&self) -> &Layer {
        &self.window
    }

    pub fn take_stats(&mut self) -> MountStats {
        std::mem::take(&mut self.stats)
    }

    /// Lowers every pending program under `roots` (the window's fixed
    /// host frames, in paint order) in one transaction.
    pub fn commit(
        &mut self,
        host: &T::Host,
        window_transform: kurbo::Affine,
        display_scale: f64,
        roots: &[Rc<NodeCell>],
        wakes: &mut dyn FnMut(&Rc<NodeCell>) -> ProducerWake,
    ) {
        let shared = Rc::clone(&self.shared);
        let mut graveyard: Vec<Box<dyn Any>> = Vec::new();
        let Self {
            id,
            root,
            window,
            window_props,
            window_committed,
            window_token,
            attached,
            stats,
            ..
        } = self;
        Shared::run_transaction(&shared, None, |tx| {
            if !*attached {
                tx[&*root].push(window);
                *attached = true;
            }
            write_props(
                tx,
                window,
                window_props,
                LayerProps {
                    transform: window_transform,
                    ..LayerProps::DEFAULT
                },
            );
            let mut cx = CommitCx {
                tx,
                host,
                shared: &shared,
                mount: *id,
                window_transform,
                display_scale,
                graveyard: &mut graveyard,
                stats,
                wakes,
            };
            for cell in roots {
                commit_cell(&mut cx, cell);
            }
            let retained: Vec<_> = roots.iter().map(|cell| cell.retained()).collect();
            let guards: Vec<_> = retained.iter().map(|r| r.layers.borrow()).collect();
            let wants = guards
                .iter()
                .map(|guard| node_want(guard))
                .collect::<Vec<_>>();
            reconcile(cx.tx, window, &wants, window_committed, window_token);
        });
        drop(graveyard);
    }
}

/// `stats` with the live mounted counts under `roots` filled in.
pub fn census(roots: &[Rc<NodeCell>], mut stats: MountStats) -> MountStats {
    for cell in roots {
        census_node(&cell.retained(), 0, &mut stats);
    }
    stats
}

fn census_clip(clipped: bool, depth: u32, stats: &mut MountStats) -> u32 {
    if !clipped {
        return depth;
    }
    stats.clip_layers += 1;
    stats.max_clip_depth = stats.max_clip_depth.max(depth + 1);
    depth + 1
}

fn census_runs(runs: &[RunLayer], stats: &mut MountStats) {
    for _ in runs.iter().filter(|run| run.held.is_some()) {
        stats.scene_layers += 1;
        stats.scene_segments += 1;
    }
}

fn census_children(
    committed: &Committed,
    own: &Rc<LayerToken>,
    depth: u32,
    stats: &mut MountStats,
) {
    for (_, token) in committed {
        let Some(token) = token.upgrade() else {
            continue;
        };
        if Rc::ptr_eq(&token, own) {
            continue;
        }
        if let Some(retained) = token.owner.upgrade().and_then(|cell| cell.try_retained()) {
            census_node(&retained, depth, stats);
        }
    }
}

fn census_node(retained: &NodeRetained, depth: u32, stats: &mut MountStats) {
    let guard = retained.layers.borrow();
    let Some(layers) = guard.as_ref() else {
        return;
    };
    let depth = census_clip(layers.frame_props.clip.is_some(), depth, stats);
    if layers.frame_props.filter.is_some() {
        stats.filtered += 1;
    }
    if layers.content_held.is_some() {
        stats.scene_layers += 1;
    }
    if layers.install.is_some() {
        stats.scene_layers += 1;
        if layers.install_gpu {
            stats.gpu_content += 1;
        }
    }
    census_runs(&layers.runs, stats);
    census_runs(&layers.inner_runs, stats);
    census_children(&layers.committed, &layers.token, depth, stats);
    if let Some((_, props)) = &layers.inner {
        let inner = census_clip(props.clip.is_some(), depth, stats);
        census_children(&layers.inner_committed, &layers.token, inner, stats);
    }
    for scope in &layers.scopes {
        let inner = census_clip(scope.props.clip.is_some(), depth, stats);
        census_runs(&scope.runs, stats);
        census_children(&scope.committed, &layers.token, inner, stats);
    }
}

fn node_want<'a>(guard: &'a std::cell::Ref<'_, Option<NodeLayers>>) -> Want<'a> {
    let layers = guard
        .as_ref()
        .expect("hydrolysis commit: a committed node has no layers");
    Want {
        id: layers.frame.id(),
        token: Rc::downgrade(&layers.token),
        layer: &layers.frame,
        attached: Some(&layers.attached),
    }
}

fn gpu_content_pixels(
    transform: kurbo::Affine,
    bounds: kurbo::Rect,
    display_scale: f64,
) -> (u32, u32) {
    let [a, b, c, d, _, _] = transform.as_coeffs();
    let x_scale = a.hypot(b) * display_scale;
    let y_scale = c.hypot(d) * display_scale;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "pixel extents are positive and far below u32::MAX"
    )]
    let size = (
        (bounds.width() * x_scale).round().max(1.0) as u32,
        (bounds.height() * y_scale).round().max(1.0) as u32,
    );
    size
}

fn install_transform(bounds: kurbo::Rect, pixels: (u32, u32)) -> kurbo::Affine {
    kurbo::Affine::translate((bounds.x0, bounds.y0))
        * kurbo::Affine::scale_non_uniform(
            bounds.width() / f64::from(pixels.0),
            bounds.height() / f64::from(pixels.1),
        )
}

/// Commits one node: its frame props, producer, and every list.
pub fn commit_cell<T: LayerTarget>(cx: &mut CommitCx<'_, '_, '_, T>, cell: &Rc<NodeCell>) {
    let retained = cell.retained();
    let mut program = retained.pending.borrow_mut().take();
    let anchor = retained.anchor.borrow().clone();
    let current = retained.layers.borrow_mut().take();
    let mut layers = match current {
        Some(layers) if layers.mount == cx.mount => layers,
        stale => {
            if let Some(stale) = stale {
                cx.retire_layers(stale);
            }
            if program.is_none() {
                // A cell re-committed on a mount its layers never saw —
                // a device remount or an unmounted subtree re-attaching —
                // re-lowers the program it last lowered: `layers` gone
                // leaves `lowered` as the only record of what it held.
                program = retained.lowered.borrow_mut().take();
            }
            let frame = cx.layer();
            NodeLayers {
                token: Rc::new(LayerToken {
                    owner: Rc::downgrade(cell),
                }),
                mount: cx.mount,
                frame,
                frame_props: LayerProps::DEFAULT,
                inner: None,
                install: None,
                runs: Vec::new(),
                inner_runs: Vec::new(),
                scopes: Vec::new(),
                committed: Vec::new(),
                inner_committed: Vec::new(),
                content_held: None,
                install_gpu: false,
                #[cfg(test)]
                material_runtime: None,
                material: None,
                attached: Cell::new(None),
            }
        }
    };
    let transform = anchor.as_ref().map_or(kurbo::Affine::IDENTITY, |anchor| {
        cell.placement().paint_delta_to(anchor)
    });
    let Some(program) = program else {
        let props = LayerProps {
            transform,
            ..layers.frame_props.clone()
        };
        write_props(cx.tx, &layers.frame, &mut layers.frame_props, props);
        *retained.layers.borrow_mut() = Some(layers);
        return;
    };
    lower_program(cx, cell, &mut layers, transform, &program);
    *retained.lowered.borrow_mut() = Some(program);
    *retained.layers.borrow_mut() = Some(layers);
}

fn lower_program<T: LayerTarget>(
    cx: &mut CommitCx<'_, '_, '_, T>,
    cell: &Rc<NodeCell>,
    layers: &mut NodeLayers,
    transform: kurbo::Affine,
    program: &Program,
) {
    let Program {
        opacity,
        clip,
        filter,
        material,
        producer,
        inner,
        items,
    } = program;
    let filter = filter
        .as_ref()
        .map(|runtime| T::filter(cx.host, &mut runtime.borrow_mut()));
    let clip = material.as_ref().map_or_else(
        || clip.clone(),
        |request| Some(ShapeData::of(&request.bounds)),
    );
    write_props(
        cx.tx,
        &layers.frame,
        &mut layers.frame_props,
        LayerProps {
            transform,
            opacity: *opacity,
            clip,
            filter,
            scroll: kurbo::Vec2::ZERO,
        },
    );
    commit_material(cx, layers, material.as_ref());

    let mut scopes_old = std::mem::take(&mut layers.scopes);
    let mut scopes_new = Vec::new();
    lower_inner(cx, layers, inner.as_ref(), &mut scopes_old, &mut scopes_new);

    assert!(
        items.is_empty() || !matches!(producer, Some(ProducerContent::Scene(_))),
        "hydrolysis commit: a scene view's frame holds only its scene"
    );
    let held_content = commit_producer(cx, cell, layers, producer.as_ref());
    if let Some(old) = std::mem::replace(&mut layers.content_held, held_content) {
        cx.graveyard.push(Box::new(old));
    }

    let leading: Vec<(LayerId, &Layer)> = layers
        .inner
        .iter()
        .chain(layers.install.iter())
        .map(|(layer, _)| (layer.id(), layer))
        .collect();
    lower_list(
        cx,
        &layers.frame,
        &leading,
        items,
        &mut layers.runs,
        &mut scopes_old,
        &mut scopes_new,
        &mut layers.committed,
        &layers.token,
    );
    for scope in scopes_old {
        cx.retire_scope(scope);
    }
    layers.scopes = scopes_new;
}

/// Mounts or releases the frame's backdrop membership through the target.
fn commit_material<T: LayerTarget>(
    cx: &mut CommitCx<'_, '_, '_, T>,
    layers: &mut NodeLayers,
    material: Option<&MaterialRequest>,
) {
    if let Some(request) = material {
        #[cfg(test)]
        {
            layers.material_runtime = Some(Rc::clone(&request.runtime));
        }
        let mut state = layers.material.take().map(|state| {
            *state
                .downcast::<T::Material>()
                .expect("hydrolysis layers: material state of another target")
        });
        T::mount_material(
            cx.host,
            cx.tx,
            &layers.frame,
            &request.runtime,
            cx.display_scale,
            request.visible,
            &mut state,
        );
        layers.material = state.map(|state| Box::new(state) as Box<dyn Any>);
    } else if let Some(state) = layers.material.take() {
        cx.graveyard.push(state);
    }
}

/// Lowers a scroll node's inner layer — clipped to the viewport and offset
/// by its `ScrollOffset` — or retires it when the node has none.
fn lower_inner<T: LayerTarget>(
    cx: &mut CommitCx<'_, '_, '_, T>,
    layers: &mut NodeLayers,
    inner: Option<&InnerProgram>,
    scopes_old: &mut Vec<ScopeLayer>,
    scopes_new: &mut Vec<ScopeLayer>,
) {
    if let Some(inner) = inner {
        if layers.inner.is_none() {
            layers.inner = Some((cx.layer(), LayerProps::DEFAULT));
        }
        let (inner_layer, inner_props) = layers
            .inner
            .as_mut()
            .expect("the inner layer was just ensured");
        write_props(
            cx.tx,
            inner_layer,
            inner_props,
            LayerProps {
                clip: Some(ShapeData::of(&inner.viewport)),
                scroll: inner.offset,
                ..LayerProps::DEFAULT
            },
        );
        lower_list(
            cx,
            inner_layer,
            &[],
            &inner.items,
            &mut layers.inner_runs,
            scopes_old,
            scopes_new,
            &mut layers.inner_committed,
            &layers.token,
        );
    } else {
        if let Some((layer, _)) = layers.inner.take() {
            cx.retire_layer(layer);
        }
        for run in std::mem::take(&mut layers.inner_runs) {
            cx.retire_run(run);
        }
        layers.inner_committed.clear();
    }
}

/// Installs or ticks the node's producer; returns the resources a scene
/// view's recording holds.
fn commit_producer<T: LayerTarget>(
    cx: &mut CommitCx<'_, '_, '_, T>,
    cell: &Rc<NodeCell>,
    layers: &mut NodeLayers,
    producer: Option<&ProducerContent>,
) -> Option<HeldResources> {
    match producer {
        Some(ProducerContent::Scene(source)) => {
            return Some(mount_scene(cx, cell, &layers.frame, source));
        }
        Some(ProducerContent::Gpu {
            runtime,
            bounds,
            visible,
        }) => mount_gpu(cx, cell, layers, runtime, *bounds, *visible),
        Some(ProducerContent::External {
            runtime,
            bounds,
            visible,
        }) => mount_external(cx, cell, layers, runtime, *bounds, *visible),
        None => {
            if let Some((layer, _)) = layers.install.take() {
                cx.retire_layer(layer);
            }
        }
    }
    None
}

/// Records a scene view's content into its frame against the target's
/// resource table, rebuilding it when the table changed since its last
/// record.
fn mount_scene<T: LayerTarget>(
    cx: &mut CommitCx<'_, '_, '_, T>,
    cell: &Rc<NodeCell>,
    frame: &Layer,
    source: &SceneContentSource,
) -> HeldResources {
    let resources = T::resources(cx.host);
    let stale = source.association.borrow().as_ref().is_some_and(|weak| {
        weak.upgrade()
            .is_none_or(|table| !Rc::ptr_eq(&table, resources))
    });
    if stale {
        let mut content = source.content.borrow_mut();
        content.rebuild_for_engine();
        content.set_invalidator(Some(Rc::clone(&source.invalidator)));
    }
    *source.association.borrow_mut() = Some(Rc::downgrade(resources));
    let mut names = resources.waterui().recording();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "scene content sizes are logical points well inside f32"
    )]
    let (width, height) = (source.bounds.width() as f32, source.bounds.height() as f32);
    let content = Rc::clone(&source.content);
    let owner = Rc::clone(cell);
    cx.tx[frame].record(|recorder| {
        if content
            .borrow_mut()
            .build_scene(recorder, &mut names, width, height)
        {
            owner.mark(super::Dirty::PAINT);
        }
    });
    cx.stats.installs += 1;
    names.finish()
}

/// Ensures the node's install layer; returns whether it was just created.
fn ensure_install<T: LayerTarget>(
    cx: &mut CommitCx<'_, '_, '_, T>,
    layers: &mut NodeLayers,
    gpu: bool,
) -> bool {
    let fresh = layers.install.is_none();
    if fresh {
        layers.install = Some((cx.layer(), LayerProps::DEFAULT));
    }
    layers.install_gpu = gpu;
    fresh
}

/// Binds a GPU producer to the install layer at the node's pixel size.
fn mount_gpu<T: LayerTarget>(
    cx: &mut CommitCx<'_, '_, '_, T>,
    cell: &Rc<NodeCell>,
    layers: &mut NodeLayers,
    runtime: &RefCell<crate::gpu_view::GpuContentRuntime>,
    bounds: kurbo::Rect,
    visible: bool,
) {
    let fresh = ensure_install(cx, layers, true);
    let world = cx.window_transform * cell.placement().resolved_transform(false);
    let pixels = gpu_content_pixels(world, bounds, cx.display_scale);
    let (install, props) = layers.install.as_mut().expect("install ensured");
    let mut runtime = runtime.borrow_mut();
    if fresh {
        runtime.binding = None;
    }
    if visible {
        if runtime.producer.is_none() {
            cx.stats.installs += 1;
        }
        let wake = (cx.wakes)(cell);
        T::mount_gpu_content(cx.host, cx.tx, install, &mut runtime, pixels, wake);
    }
    write_props(
        cx.tx,
        install,
        props,
        LayerProps {
            transform: install_transform(bounds, pixels),
            ..LayerProps::DEFAULT
        },
    );
}

/// Starts or ticks an external-frame source on the install layer.
fn mount_external<T: LayerTarget>(
    cx: &mut CommitCx<'_, '_, '_, T>,
    cell: &Rc<NodeCell>,
    layers: &mut NodeLayers,
    runtime: &RefCell<crate::gpu_view::ExternalFrameRuntime>,
    bounds: kurbo::Rect,
    visible: bool,
) {
    let fresh = ensure_install(cx, layers, false);
    let (install, props) = layers.install.as_mut().expect("install ensured");
    let mut runtime = runtime.borrow_mut();
    if fresh {
        runtime.binding = None;
    }
    if !visible {
        return;
    }
    if runtime.receiver.is_none() {
        cx.stats.installs += 1;
    }
    let wake = (cx.wakes)(cell);
    if let Some(pixels) = T::mount_external_frame(cx.host, cx.tx, install, &mut runtime, wake) {
        write_props(
            cx.tx,
            install,
            props,
            LayerProps {
                transform: install_transform(bounds, pixels),
                ..LayerProps::DEFAULT
            },
        );
    }
}

/// One child a list wants, in order.
struct Want<'a> {
    id: LayerId,
    token: Weak<LayerToken>,
    layer: &'a Layer,
    attached: Option<&'a Cell<Option<LayerId>>>,
}

enum Slot {
    Run(usize),
    Scope(usize),
    Node(Rc<NodeCell>),
}

#[expect(
    clippy::too_many_arguments,
    reason = "one list's lowering reads the node's run, scope and committed state together"
)]
fn lower_list<T: LayerTarget>(
    cx: &mut CommitCx<'_, '_, '_, T>,
    parent: &Layer,
    leading: &[(LayerId, &Layer)],
    items: &[Item],
    runs: &mut Vec<RunLayer>,
    scopes_old: &mut Vec<ScopeLayer>,
    scopes_new: &mut Vec<ScopeLayer>,
    committed: &mut Committed,
    token: &Rc<LayerToken>,
) {
    let mut old_runs = std::mem::take(runs);
    let had_own = old_runs.iter().any(|run| run.layer.is_none());
    let mut own_used = false;
    let keys: Vec<_> = items.iter().map(Item::key).collect();
    let mut slots = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        match item {
            Item::Run(recording) => {
                let key = keys
                    .get(index + 1)
                    .cloned()
                    .flatten()
                    .map_or(RunKey::Tail, RunKey::Before);
                let own = index == 0 && leading.is_empty();
                runs.push(lower_run(cx, parent, &mut old_runs, key, own, recording));
                if own {
                    own_used = true;
                } else {
                    slots.push(Slot::Run(runs.len() - 1));
                }
            }
            Item::Node(child) => {
                commit_cell(cx, child);
                slots.push(Slot::Node(Rc::clone(child)));
            }
            Item::Scope {
                key,
                props,
                placement,
                items,
            } => {
                let at = lower_scope(
                    cx,
                    (*key, props, placement, items),
                    scopes_old,
                    scopes_new,
                    token,
                );
                slots.push(Slot::Scope(at));
            }
        }
    }
    if had_own && !own_used {
        cx.tx[parent].clear_content();
    }
    for run in old_runs {
        cx.retire_run(run);
    }

    let retained: Vec<_> = slots
        .iter()
        .filter_map(|slot| match slot {
            Slot::Node(cell) => Some(cell.retained()),
            _ => None,
        })
        .collect();
    let guards: Vec<_> = retained.iter().map(|r| r.layers.borrow()).collect();
    let own = Rc::downgrade(token);
    let mut wants: Vec<Want<'_>> = leading
        .iter()
        .map(|&(id, layer)| Want {
            id,
            token: own.clone(),
            layer,
            attached: None,
        })
        .collect();
    let mut nodes = guards.iter();
    for slot in &slots {
        wants.push(match slot {
            Slot::Run(at) => {
                let layer = runs[*at].layer.as_ref().expect("a listed run has a layer");
                Want {
                    id: layer.id(),
                    token: own.clone(),
                    layer,
                    attached: None,
                }
            }
            Slot::Scope(at) => {
                let layer = &scopes_new[*at].layer;
                Want {
                    id: layer.id(),
                    token: own.clone(),
                    layer,
                    attached: None,
                }
            }
            Slot::Node(_) => node_want(nodes.next().expect("one guard per node slot")),
        });
    }
    reconcile(cx.tx, parent, &wants, committed, token);
}

/// Lowers one run: the list's first item draws into the parent's own
/// content, any other gets a run layer, kept by `key` across commits.
fn lower_run<T: LayerTarget>(
    cx: &mut CommitCx<'_, '_, '_, T>,
    parent: &Layer,
    old_runs: &mut Vec<RunLayer>,
    key: RunKey,
    own: bool,
    recording: &crate::renderer::recording::Recording,
) -> RunLayer {
    let mut run = old_runs.iter().position(|run| run.key == key).map_or_else(
        || RunLayer {
            key,
            layer: None,
            held: None,
            #[cfg(test)]
            recording: None,
        },
        |at| old_runs.remove(at),
    );
    if own {
        if let Some(layer) = run.layer.take() {
            cx.retire_layer(layer);
        }
    } else if run.layer.is_none() {
        run.layer = Some(cx.layer());
    }
    #[cfg(test)]
    {
        run.recording = Some(recording.clone());
    }
    let resources = T::resources(cx.host);
    let mut held = None;
    cx.tx[run.layer.as_ref().unwrap_or(parent)].record(|recorder| {
        held = Some(recording.record_on(recorder, resources));
    });
    if let Some(old) = std::mem::replace(&mut run.held, held) {
        cx.graveyard.push(Box::new(old));
    }
    cx.stats.installs += 1;
    run
}

/// Lowers one scope, kept by its key across commits; returns its index in
/// `scopes_new`.
fn lower_scope<T: LayerTarget>(
    cx: &mut CommitCx<'_, '_, '_, T>,
    (key, props, placement, items): (ScopeKey, &ScopeProps, &Rc<Placement>, &[Item]),
    scopes_old: &mut Vec<ScopeLayer>,
    scopes_new: &mut Vec<ScopeLayer>,
    token: &Rc<LayerToken>,
) -> usize {
    let mut scope = scopes_old
        .iter()
        .position(|scope| scope.key == key)
        .map_or_else(
            || ScopeLayer {
                key,
                layer: cx.layer(),
                placement: Rc::clone(placement),
                props: LayerProps::DEFAULT,
                runs: Vec::new(),
                committed: Vec::new(),
            },
            |at| scopes_old.swap_remove(at),
        );
    scope.placement = Rc::clone(placement);
    write_props(
        cx.tx,
        &scope.layer,
        &mut scope.props,
        LayerProps {
            transform: props.transform,
            opacity: props.alpha,
            clip: props.clip.clone(),
            ..LayerProps::DEFAULT
        },
    );
    let ScopeLayer {
        layer,
        runs,
        committed,
        ..
    } = &mut scope;
    lower_list(
        cx,
        layer,
        &[],
        items,
        runs,
        scopes_old,
        scopes_new,
        committed,
        token,
    );
    scopes_new.push(scope);
    scopes_new.len() - 1
}

/// Brings `parent`'s engine child list to `wants`, emitting only the
/// moves, inserts and removes the difference needs.
///
/// `committed` mirrors the engine's list as of the last commit. A dead
/// token's layer already left (its drop queued the removal ahead of this
/// transaction's edits). An own layer no longer wanted is retiring: its
/// drop after the body removes it, so it stays in the working list until
/// then and the indices written here match the engine's. A live child
/// frame no longer wanted is detached here when it is still attached
/// under `parent` — a reparented frame was already moved by its insert.
fn reconcile<T: LayerTarget>(
    tx: &mut Transaction<'_, T>,
    parent: &Layer,
    wants: &[Want<'_>],
    committed: &mut Committed,
    token: &Rc<LayerToken>,
) {
    let own = Rc::as_ptr(token);
    let is_wanted = |id: LayerId| wants.iter().any(|want| want.id == id);
    // (id, retiring)
    let mut current: Vec<(LayerId, bool)> = Vec::with_capacity(committed.len());
    for (id, weak) in committed.drain(..) {
        if weak.strong_count() == 0 {
            continue;
        }
        if is_wanted(id) {
            current.push((id, false));
        } else if std::ptr::eq(weak.as_ptr(), own) {
            current.push((id, true));
        } else if let Some(retained) = weak
            .upgrade()
            .and_then(|token| token.owner.upgrade())
            .and_then(|cell| cell.try_retained())
            && let Some(layers) = retained.layers.borrow().as_ref()
            && layers.frame.id() == id
            && layers.attached.get() == Some(parent.id())
        {
            tx[parent].remove(&layers.frame);
            layers.attached.set(None);
        }
    }
    let mut at = 0;
    for want in wants {
        while current.get(at).is_some_and(|&(_, retiring)| retiring) {
            at += 1;
        }
        if current.get(at).is_none_or(|&(id, _)| id != want.id) {
            if let Some(from) = current.iter().position(|&(id, _)| id == want.id) {
                current.remove(from);
                if from < at {
                    at -= 1;
                }
            }
            tx[parent].insert(at, want.layer);
            current.insert(at, (want.id, false));
        }
        at += 1;
        if let Some(attached) = want.attached {
            attached.set(Some(parent.id()));
        }
    }
    *committed = current
        .into_iter()
        .filter(|&(_, retiring)| !retiring)
        .map(|(id, _)| {
            let want = wants
                .iter()
                .find(|want| want.id == id)
                .expect("a kept child is wanted");
            (id, want.token.clone())
        })
        .collect();
}

/// What [`visit`] reports while walking the mounted layers.
#[cfg(test)]
pub trait LayerVisitor {
    /// A node's frame: `world` is its content space in window points,
    /// `alphas` the opacities below one on the way down (its own included).
    fn node(&mut self, _layers: &NodeLayers, _world: kurbo::Affine, _alphas: &[f32]) {}
    /// A recorded run, painted in `world`.
    fn run(&mut self, _world: kurbo::Affine, _recording: &crate::renderer::recording::Recording) {}
    /// A layer's clip opens around its content, in the layer's own space
    /// `world` (before its scroll offset).
    fn open_clip(&mut self, _world: kurbo::Affine, _clip: &ShapeData) {}
    /// Closes the clip [`Self::open_clip`] opened.
    fn close_clip(&mut self) {}
}

/// Walks the committed layers under `roots` in paint order, composing each
/// layer's props the way the engine does.
#[cfg(test)]
pub fn visit(roots: &[Rc<NodeCell>], visitor: &mut dyn LayerVisitor) {
    let mut alphas = Vec::new();
    for cell in roots {
        visit_node(
            &cell.retained(),
            kurbo::Affine::IDENTITY,
            &mut alphas,
            visitor,
        );
    }
}

#[cfg(test)]
fn props_world(parent: kurbo::Affine, props: &LayerProps) -> kurbo::Affine {
    parent * props.transform * kurbo::Affine::translate(-props.scroll)
}

#[cfg(test)]
fn visit_node(
    retained: &NodeRetained,
    parent: kurbo::Affine,
    alphas: &mut Vec<f32>,
    visitor: &mut dyn LayerVisitor,
) {
    let guard = retained.layers.borrow();
    let Some(layers) = guard.as_ref() else {
        return;
    };
    let world = props_world(parent, &layers.frame_props);
    let faded = layers.frame_props.opacity < 1.0;
    if faded {
        alphas.push(layers.frame_props.opacity);
    }
    let clip = open_clip(parent, &layers.frame_props, visitor);
    visitor.node(layers, world, alphas);
    visit_list(
        layers,
        &layers.runs,
        &layers.committed,
        world,
        alphas,
        visitor,
    );
    if clip {
        visitor.close_clip();
    }
    if faded {
        alphas.pop();
    }
}

#[cfg(test)]
fn open_clip(parent: kurbo::Affine, props: &LayerProps, visitor: &mut dyn LayerVisitor) -> bool {
    let Some(clip) = &props.clip else {
        return false;
    };
    visitor.open_clip(parent * props.transform, clip);
    true
}

#[cfg(test)]
fn visit_list(
    layers: &NodeLayers,
    runs: &[RunLayer],
    committed: &Committed,
    world: kurbo::Affine,
    alphas: &mut Vec<f32>,
    visitor: &mut dyn LayerVisitor,
) {
    for run in runs {
        if let Some(recording) = &run.recording {
            visitor.run(world, recording);
        }
    }
    for (id, token) in committed {
        let Some(token) = token.upgrade() else {
            continue;
        };
        if !Rc::ptr_eq(&token, &layers.token) {
            if let Some(retained) = token.owner.upgrade().and_then(|cell| cell.try_retained()) {
                visit_node(&retained, world, alphas, visitor);
            }
            continue;
        }
        if let Some((layer, props)) = &layers.inner
            && layer.id() == *id
        {
            let inner = props_world(world, props);
            let clip = open_clip(world, props, visitor);
            visit_list(
                layers,
                &layers.inner_runs,
                &layers.inner_committed,
                inner,
                alphas,
                visitor,
            );
            if clip {
                visitor.close_clip();
            }
        } else if let Some(scope) = layers.scopes.iter().find(|scope| scope.layer.id() == *id) {
            let inner = props_world(world, &scope.props);
            let faded = scope.props.opacity < 1.0;
            if faded {
                alphas.push(scope.props.opacity);
            }
            let clip = open_clip(world, &scope.props, visitor);
            visit_list(
                layers,
                &scope.runs,
                &scope.committed,
                inner,
                alphas,
                visitor,
            );
            if clip {
                visitor.close_clip();
            }
            if faded {
                alphas.pop();
            }
        }
    }
}
