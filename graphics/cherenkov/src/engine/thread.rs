//! The render thread's loop: owns the backend renderer and one
//! [`SurfaceTree`] per surface, applies commits, samples animations at the
//! frame time, renders, and answers with [`Next`] and the [`FrameStats`].

use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::backend::{
    Backend, Display, Frame, Redraw, Renderer, SurfaceFrame, SurfaceInfo, Visibility,
};
use crate::engine::{CompletionWaker, SurfaceWaker};
use crate::error::{EngineError, RenderError, ResourceError, SurfaceError};
use crate::frame::{FrameId, FrameStats, Next, RefreshRange};
use crate::image::ImageUpload;
use crate::message::{
    BackdropShaderId, ChangeSet, LayerId, LayerOp, Message, Op, ResOp, SurfaceId,
};
use crate::paint::ImageId;
use crate::resource::ResourceId;
use crate::tree::SurfaceTree;
use crate::{BackdropEffect, WorkingColor};

/// Whether a surface's frames can ask the backend to present, and the
/// pending presentation state when they can. Only a surface the backend
/// reported as presenting carries the flag — a pending present cannot
/// exist for an offscreen target (#98).
enum Presentation {
    /// The surface retains pixels; there is no swapchain to present to,
    /// so a `Display` update never marks a present.
    Retained,
    /// The surface presents to a display; `pending` asks for a frame even
    /// without `changed` — a headroom-only `Display` update reaches the
    /// swapchain without touching the layer tree or any content cache.
    Presenting {
        /// Whether the surface's window should present without new
        /// content.
        pending: bool,
    },
}

/// What the commits since the last render changed. Ordered, so an
/// install raises `Clean` to `Installs` and anything else forces `Other`
/// — `changed` is `commits != Clean`, and the frame's `plane_frames` is
/// `plane_frames` only when `commits == Installs` (#90).
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Commits {
    /// Nothing committed.
    Clean,
    /// Only external-frame installs — `plane_frames` names their layers.
    Installs,
    /// A layer op, a clear, a resize, a display scale change or an image
    /// replacement landed too.
    Other,
}

/// One surface's render-thread state.
struct SurfaceState {
    tree: SurfaceTree,
    size: (u32, u32),
    display: Display,
    clear: WorkingColor,
    /// Whether a property op, a content op or an animation step touched the
    /// surface since the last render — and whether only installs did.
    commits: Commits,
    /// The layers whose external frame was installed since the last
    /// render. The set lives for the surface's life: a render clears it
    /// rather than reallocating it, so a steady stream of plane-only
    /// frames allocates nothing (#90).
    plane_frames: FxHashSet<LayerId>,
    /// Whether the surface presents, and whether a present is pending (#98).
    presentation: Presentation,
    /// Whether the host announced the surface moved to another display
    /// since the previous frame — the frame's `display_moved` (#98).
    display_moved: bool,
    /// Whether the surface's recorded contents still run operand animations
    /// on the UI thread. The tracks live there; they need the next frame's
    /// sample at the fast rate class.
    content_animating: bool,
    /// Track cadence before the backend accepts this frame's handoffs.
    sampled_rate: Option<RefreshRange>,
    /// Whether the host announced the surface hidden. A hidden surface is
    /// left out of every frame: its tree is not sampled, it is not drawn,
    /// and its per-frame state waits for the frame that shows it.
    visibility: Visibility,
    /// The surface's host wake-up, shared with its UI-thread handle.
    waker: Arc<SurfaceWaker>,
}

impl SurfaceState {
    /// Whether the frame should ask the backend to present.
    const fn present_pending(&self) -> bool {
        matches!(
            self.presentation,
            Presentation::Presenting { pending: true }
        )
    }

    /// Marks the next frame for presentation; a no-op on a retained
    /// surface, which cannot hold the flag.
    const fn mark_present(&mut self) {
        if let Presentation::Presenting { pending } = &mut self.presentation {
            *pending = true;
        }
    }

    /// Consumes the pending present after a render.
    const fn presented(&mut self) {
        if let Presentation::Presenting { pending } = &mut self.presentation {
            *pending = false;
        }
    }
}

/// A rejection the backend reported after the resource's handle was
/// returned.
struct Rejection {
    reason: Arc<ResourceError>,
    /// Whether the backend still holds the resource: true after a rejected
    /// image replacement, which keeps the previous pixels, false after a
    /// rejected registration, which committed nothing.
    held: bool,
}

/// A released resource that installed content still draws. Its backend
/// removal waits until no surface's installed content draws it (#199).
struct PendingRelease<B: Backend> {
    remove: ResOp<B>,
    /// The surfaces whose installed content draws the resource.
    surfaces: FxHashSet<SurfaceId>,
}

/// The render loop's per-resource bookkeeping, shared by the native render
/// thread and the browser executor.
///
/// - A rejection the backend reported fails every render that draws the
///   resource with [`RenderError::Rejected`], until a replacement succeeds
///   or the resource is freed.
/// - A released resource is freed only once no surface's installed
///   content draws it: the release waits while one does, and is carried
///   out when a commit, a layer removal or a surface's destruction leaves
///   no surface drawing it. Ids are never reused, so a pending id cannot
///   name another resource.
struct Resources<B: Backend> {
    rejections: FxHashMap<ResourceId, Rejection>,
    pending: FxHashMap<ResourceId, PendingRelease<B>>,
}

impl<B: Backend> Default for Resources<B> {
    fn default() -> Self {
        Self {
            rejections: FxHashMap::default(),
            pending: FxHashMap::default(),
        }
    }
}

impl<B: Backend> Resources<B> {
    /// Records the outcome of registering `resource`.
    fn register(&mut self, resource: ResourceId, result: Result<(), ResourceError>) {
        debug_assert!(
            !self.pending.contains_key(&resource),
            "{resource} registered while its release is pending: ids are never reused"
        );
        if let Err(reason) = result {
            tracing::debug!(%resource, %reason, "backend rejected a registration");
            self.rejections.insert(
                resource,
                Rejection {
                    reason: Arc::new(reason),
                    held: false,
                },
            );
        }
    }

    /// Releases `resource`, whose last handle dropped: frees it now when no
    /// surface's installed content draws it, and otherwise records the
    /// release as pending.
    fn release(
        &mut self,
        renderer: &mut B::Renderer,
        surfaces: &FxHashMap<SurfaceId, SurfaceState>,
        resource: ResourceId,
        remove: ResOp<B>,
    ) {
        let drawing: FxHashSet<SurfaceId> = surfaces
            .iter()
            .filter(|(surface, state)| draws(renderer, **surface, state, resource))
            .map(|(surface, _)| *surface)
            .collect();
        if drawing.is_empty() {
            free::<B>(&mut self.rejections, renderer, resource, remove);
        } else {
            tracing::debug!(%resource, surfaces = drawing.len(), "release waits for installed content");
            self.pending.insert(
                resource,
                PendingRelease {
                    remove,
                    surfaces: drawing,
                },
            );
        }
    }

    /// Updates the pending releases after commits: a surface that changed
    /// is drawing a pending resource exactly when its installed content now
    /// names it. Frees every resource no surface draws any more.
    fn settle(
        &mut self,
        renderer: &mut B::Renderer,
        surfaces: &FxHashMap<SurfaceId, SurfaceState>,
    ) {
        if self.pending.is_empty() {
            return;
        }
        for (resource, pending) in &mut self.pending {
            for (surface, state) in surfaces
                .iter()
                .filter(|(_, state)| state.commits != Commits::Clean)
            {
                if draws(renderer, *surface, state, *resource) {
                    pending.surfaces.insert(*surface);
                } else {
                    pending.surfaces.remove(surface);
                }
            }
        }
        self.free_settled(renderer);
    }

    /// Surface `id` is destroyed: it draws nothing any more.
    fn surface_destroyed(&mut self, renderer: &mut B::Renderer, id: SurfaceId) {
        if self.pending.is_empty() {
            return;
        }
        for pending in self.pending.values_mut() {
            pending.surfaces.remove(&id);
        }
        self.free_settled(renderer);
    }

    /// Carries out every pending release that no surface draws any more.
    fn free_settled(&mut self, renderer: &mut B::Renderer) {
        let Self {
            rejections,
            pending,
        } = self;
        for (resource, release) in pending.extract_if(|_, release| release.surfaces.is_empty()) {
            tracing::debug!(%resource, "pending release carried out");
            free::<B>(rejections, renderer, resource, release.remove);
        }
    }

    /// Fails when a surface that changed since the last render draws a
    /// rejected resource. An unchanged surface cannot: a rejected
    /// registration's id reaches content only in a commit, and a rejected
    /// replacement marks every surface sampling the image changed.
    fn check(
        &self,
        renderer: &B::Renderer,
        surfaces: &FxHashMap<SurfaceId, SurfaceState>,
    ) -> Result<(), RenderError> {
        if self.rejections.is_empty() {
            return Ok(());
        }
        // A hidden surface is not drawn: it fails no frame until the one
        // that shows it, when its changes are still pending.
        for (surface, state) in surfaces.iter().filter(|(_, state)| {
            state.commits != Commits::Clean && state.visibility == Visibility::Visible
        }) {
            for (resource, rejection) in &self.rejections {
                if draws(renderer, *surface, state, *resource) {
                    return Err(RenderError::Rejected {
                        resource: *resource,
                        reason: Arc::clone(&rejection.reason),
                    });
                }
            }
        }
        Ok(())
    }
}

/// Frees `resource`: clears its rejection and runs the backend's removal
/// unless the backend never committed the resource.
fn free<B: Backend>(
    rejections: &mut FxHashMap<ResourceId, Rejection>,
    renderer: &mut B::Renderer,
    resource: ResourceId,
    remove: ResOp<B>,
) {
    if rejections
        .remove(&resource)
        .is_none_or(|rejection| rejection.held)
    {
        remove(renderer);
    }
}

/// Whether surface `surface`'s installed content draws `resource`. The
/// backend answers for content; backdrop shaders are sampled through the
/// layer tree.
fn draws<R: Renderer>(
    renderer: &R,
    surface: SurfaceId,
    state: &SurfaceState,
    resource: ResourceId,
) -> bool {
    match resource {
        ResourceId::BackdropShader(id) => samples_backdrop_shader(&state.tree, id),
        resource => renderer.samples(surface, resource),
    }
}

/// Whether a layer of `tree` samples its backdrop through backdrop shader
/// `id`.
fn samples_backdrop_shader(tree: &SurfaceTree, id: BackdropShaderId) -> bool {
    tree.layers().any(|(_, node)| {
        matches!(
            node.backdrop.as_ref().and_then(crate::BackdropSample::effect),
            Some(BackdropEffect::Shader(effect)) if effect.shader == id
        )
    })
}

/// The render loop: runs on the `"cherenkov-render"` thread until
/// [`Message::Shutdown`] or channel disconnect.
///
/// `retire_rx` carries producer retirements on their own unbounded
/// queue — a binding's last handle can die inside this thread's own
/// work (`unbind`, surface destroy, `drain_gpu_producers`), and a
/// retirement sent on the bounded `rx` channel would block this loop
/// on a channel it alone drains. The queue drains after each applied
/// message.
#[cfg(not(target_arch = "wasm32"))]
pub fn run<B: Backend>(
    config: B::Config,
    rx: &Receiver<Message<B>>,
    retire_rx: &Receiver<crate::message::ResOp<B>>,
    init_reply: &Sender<Result<B::Info, EngineError>>,
) {
    let (mut renderer, info) = match B::init(config) {
        Ok(pair) => pair,
        Err(error) => {
            let _ = init_reply.send(Err(error));
            return;
        }
    };
    let _ = init_reply.send(Ok(info));
    let mut surfaces: FxHashMap<SurfaceId, SurfaceState> = FxHashMap::default();
    let mut resources = Resources::<B>::default();
    let mut next_frame = 0u64;
    while let Ok(message) = rx.recv() {
        match message {
            Message::CreateSurface {
                id,
                target,
                waker,
                reply,
            } => {
                let _ = reply.send(create_surface::<B>(
                    &mut renderer,
                    &mut surfaces,
                    id,
                    target,
                    waker,
                ));
            }
            Message::ResizeSurface { id, size } => {
                resize_surface::<B>(&mut renderer, &mut surfaces, id, size);
            }
            Message::DestroySurface { id } => {
                destroy_surface::<B>(&mut renderer, &mut surfaces, &mut resources, id);
            }
            Message::Display { id, display } => set_display(&mut surfaces, id, display),
            Message::DisplayMoved { id } => set_display_moved(&mut surfaces, id),
            Message::Visibility { id, visibility } => {
                set_visibility::<B>(&mut renderer, &mut surfaces, id, visibility);
            }
            Message::Resource(op) => op(&mut renderer),
            Message::ProducerFrame { opaque, apply, .. } => {
                producer_frame::<B>(&mut renderer, &mut surfaces, opaque, apply);
            }
            Message::Register { resource, op } => {
                resources.register(resource, op(&mut renderer));
            }
            Message::Release { resource, op } => {
                resources.release(&mut renderer, &surfaces, resource, op);
            }
            Message::ReplaceImage { id, image } => {
                replace_image::<B>(&mut renderer, &mut surfaces, &mut resources, id, image);
            }
            Message::Apply { id, mut changes } => apply_hidden::<B>(
                &mut renderer,
                &mut surfaces,
                &mut resources,
                id,
                &mut changes,
            ),
            Message::Render {
                time,
                mut commits,
                reply,
            } => {
                let id = FrameId(next_frame);
                next_frame += 1;
                let result = render::<B>(
                    &mut renderer,
                    &mut surfaces,
                    &mut resources,
                    id,
                    time.0,
                    &mut commits,
                );
                // This frame's queued retirements belong to its batch.
                drain_retire::<B>(retire_rx, &mut renderer);
                let sender = reply.clone();
                let _ = sender.send(crate::message::RenderReply {
                    result,
                    commits,
                    sender: reply,
                });
            }
            Message::FinishTimings { reply } => {
                let _ = reply.send(renderer.finish_timings());
            }
            Message::Readback { surface, reply } => {
                let _ = reply.send(renderer.readback(surface));
            }
            Message::Memory { reply } => {
                let sender = reply.clone();
                let _ = sender.send(crate::message::MemoryReply {
                    usage: renderer.memory(),
                    sender: reply,
                });
            }
            Message::Trim(pressure) => renderer.trim(pressure),
            Message::Shutdown => break,
        }
        drain_retire::<B>(retire_rx, &mut renderer);
    }
}

/// Applies every queued producer retirement (`thread::run`'s `retire_rx`
/// drain after each applied message).
#[cfg(not(target_arch = "wasm32"))]
fn drain_retire<B: Backend>(rx: &Receiver<crate::message::ResOp<B>>, renderer: &mut B::Renderer) {
    while let Ok(retire) = rx.try_recv() {
        retire(renderer);
    }
}

/// The submitted frame applied on the renderer names the layers it lands
/// on; each gets the frame's declared alpha contract noted and counts as
/// a frame swap — a planes-capable backend presents those alone when they
/// are the surface's only change (#90).
fn producer_frame<B: Backend>(
    renderer: &mut B::Renderer,
    surfaces: &mut FxHashMap<SurfaceId, SurfaceState>,
    opaque: bool,
    apply: crate::message::ProducerApply<B>,
) {
    for (surface, layer) in apply(renderer) {
        if let Some(state) = surfaces.get_mut(&surface) {
            state.tree.note_installed(layer, opaque);
            state.commits = state.commits.max(Commits::Installs);
            state.plane_frames.insert(layer);
        }
    }
}

/// Creates surface `id`'s render-side state and its layer tree.
fn create_surface<B: Backend>(
    renderer: &mut B::Renderer,
    surfaces: &mut FxHashMap<SurfaceId, SurfaceState>,
    id: SurfaceId,
    target: B::Target,
    waker: Arc<SurfaceWaker>,
) -> Result<SurfaceInfo, SurfaceError> {
    let info = renderer.create_surface(id, target, CompletionWaker::new(&waker))?;
    surfaces.insert(
        id,
        SurfaceState {
            tree: SurfaceTree::new(),
            size: info.size,
            display: Display::default(),
            clear: WorkingColor::TRANSPARENT,
            commits: Commits::Other,
            plane_frames: FxHashSet::default(),
            presentation: if info.presents {
                Presentation::Presenting { pending: false }
            } else {
                Presentation::Retained
            },
            display_moved: false,
            content_animating: false,
            sampled_rate: None,
            visibility: Visibility::Visible,
            waker,
        },
    );
    Ok(info)
}

fn resize_surface<B: Backend>(
    renderer: &mut B::Renderer,
    surfaces: &mut FxHashMap<SurfaceId, SurfaceState>,
    id: SurfaceId,
    size: (u32, u32),
) {
    renderer.resize_surface(id, size);
    if let Some(state) = surfaces.get_mut(&id) {
        state.size = size;
        state.commits = Commits::Other;
    } else {
        tracing::trace!(surface = id.raw(), "resize of unknown surface");
    }
}

/// Destroys surface `id`, then carries out the pending releases only its
/// content still drew.
fn destroy_surface<B: Backend>(
    renderer: &mut B::Renderer,
    surfaces: &mut FxHashMap<SurfaceId, SurfaceState>,
    resources: &mut Resources<B>,
    id: SurfaceId,
) {
    if surfaces.remove(&id).is_some() {
        renderer.destroy_surface(id);
        resources.surface_destroyed(renderer, id);
    } else {
        // Nothing was committed — a dropped `Engine::surface` future whose
        // create failed may still send this (#150).
        tracing::trace!(surface = id.raw(), "destroy of unknown surface");
    }
}

fn set_display(surfaces: &mut FxHashMap<SurfaceId, SurfaceState>, id: SurfaceId, display: Display) {
    if let Some(state) = surfaces.get_mut(&id) {
        if state.display != display {
            // A scale change reshapes the content; a headroom-only update
            // re-presents without touching it (#98). Only a presenting
            // surface can be pending a present.
            let scale_changed = state.display.scale.to_bits() != display.scale.to_bits();
            if scale_changed {
                state.commits = Commits::Other;
            }
            state.mark_present();
            state.display = display;
        }
    } else {
        tracing::trace!(surface = id.raw(), "display of unknown surface");
    }
}

/// The host announced surface `id`'s visibility, which differs from the
/// previous one. The surface's frame state survives while it is hidden; the
/// frame that shows it again redraws it whole from the current state —
/// whatever the backend let lapse while it was hidden (a producer, a
/// filter's parameters, a dropped drawable) is current again — and
/// presents it.
fn set_visibility<B: Backend>(
    renderer: &mut B::Renderer,
    surfaces: &mut FxHashMap<SurfaceId, SurfaceState>,
    id: SurfaceId,
    visibility: Visibility,
) {
    let state = surfaces
        .get_mut(&id)
        .expect("visibility announced through a live surface handle");
    state.visibility = visibility;
    if visibility == Visibility::Visible {
        state.commits = Commits::Other;
        state.mark_present();
    }
    renderer.set_visibility(id, visibility);
}

fn set_display_moved(surfaces: &mut FxHashMap<SurfaceId, SurfaceState>, id: SurfaceId) {
    if let Some(state) = surfaces.get_mut(&id) {
        // A move re-enumerates output negotiation, where a headroom-only
        // `Display` update never does — and presents, since a
        // reconfigured swapchain must be shown (#98).
        state.display_moved = true;
        state.mark_present();
    } else {
        tracing::trace!(surface = id.raw(), "display move of unknown surface");
    }
}

/// Replaces image `id`'s pixels and marks changed only the surfaces whose
/// content samples the image, waking the host through each of them — a
/// hidden one wakes nothing; the next render redraws the visible ones with
/// the new pixels and leaves every other surface's skip intact. An image
/// whose registration was rejected is registered with the new pixels
/// instead. A rejection is recorded, and the marked surfaces then fail
/// their render.
fn replace_image<B: Backend>(
    renderer: &mut B::Renderer,
    surfaces: &mut FxHashMap<SurfaceId, SurfaceState>,
    resources: &mut Resources<B>,
    id: ImageId,
    image: ImageUpload,
) {
    let resource = ResourceId::Image(id);
    debug_assert!(
        !resources.pending.contains_key(&resource),
        "{resource} replaced after its release: a replacement needs a live handle"
    );
    let rejections = &mut resources.rejections;
    let held = rejections
        .get(&resource)
        .is_none_or(|rejection| rejection.held);
    let result = if held {
        renderer.replace_image(id, image)
    } else {
        renderer.add_image(id, image)
    };
    match result {
        Ok(()) => {
            rejections.remove(&resource);
        }
        Err(reason) => {
            tracing::debug!(%resource, %reason, "backend rejected an image replacement");
            rejections.insert(
                resource,
                Rejection {
                    reason: Arc::new(reason),
                    held,
                },
            );
        }
    }
    for (surface, state) in surfaces {
        if renderer.samples(*surface, resource) {
            state.commits = Commits::Other;
            state.waker.wake();
        }
    }
}

/// Applies one surface's committed change set into its tree, forwarding
/// content ops and install closures to the renderer in order.
fn commit<B: Backend>(
    renderer: &mut B::Renderer,
    state: &mut SurfaceState,
    surface: SurfaceId,
    changes: &mut ChangeSet<B>,
) {
    let ChangeSet {
        clear,
        ops,
        recycled,
        animating,
    } = changes;
    if let Some(clear) = clear.take() {
        state.clear = clear;
        state.commits = Commits::Other;
    }
    state.content_animating = *animating;
    for op in ops.drain(..) {
        match op {
            Op::Layer(LayerOp::Remove(layer)) => {
                state.commits = Commits::Other;
                for removed in state.tree.remove(layer) {
                    renderer.remove_layer(surface, removed);
                }
            }
            Op::Layer(LayerOp::Content(layer, content)) => {
                state.commits = Commits::Other;
                state.tree.apply(LayerOp::Content(layer, None));
                state.tree.note_content(layer, content.as_ref());
                if let Some(mut old) = renderer.set_content(surface, layer, content)
                    && old.try_recycle()
                {
                    recycled.push((layer, old));
                }
            }
            Op::Layer(op) => {
                state.commits = Commits::Other;
                state.tree.apply(op);
            }
            Op::Install(layer, install) => {
                state.commits = Commits::Other;
                // The install reports its content's declared alpha —
                // `None` before its first frame — noted on the layer.
                state
                    .tree
                    .note_installed(layer, install(&mut *renderer).unwrap_or(false));
            }
        }
    }
}

/// Applies hidden surface `id`'s changes, sent as they were made: the tree
/// and the installed content follow them, nothing is sampled or drawn, and
/// the pending releases they settle are carried out. The surface stays
/// changed, so the frame that shows it redraws it whole and checks it for
/// rejected resources.
fn apply_hidden<B: Backend>(
    renderer: &mut B::Renderer,
    surfaces: &mut FxHashMap<SurfaceId, SurfaceState>,
    resources: &mut Resources<B>,
    id: SurfaceId,
    changes: &mut ChangeSet<B>,
) {
    let Some(state) = surfaces.get_mut(&id) else {
        // A layer or binding that outlived its surface may still send.
        tracing::trace!(surface = id.raw(), "changes for unknown surface");
        return;
    };
    debug_assert_eq!(
        state.visibility,
        Visibility::Hidden,
        "only a hidden surface sends its changes as it makes them"
    );
    commit(renderer, state, id, changes);
    resources.settle(renderer, surfaces);
}

/// Applies every surface's commit, then carries out the pending releases
/// no installed content draws any more, before the frame can draw them.
///
/// # Errors
/// [`RenderError::Rejected`] when a changed surface draws a resource the
/// backend rejected.
fn apply_commits<B: Backend>(
    renderer: &mut B::Renderer,
    surfaces: &mut FxHashMap<SurfaceId, SurfaceState>,
    resources: &mut Resources<B>,
    commits: &mut [(SurfaceId, ChangeSet<B>)],
) -> Result<(), RenderError> {
    for (surface, changes) in commits.iter_mut() {
        if let Some(state) = surfaces.get_mut(surface) {
            commit(renderer, state, *surface, changes);
        } else {
            // A dropped surface may still have queued ops: legal, ignore.
            tracing::trace!(surface = surface.raw(), "commit for unknown surface");
            changes.clear = None;
            changes.ops.clear();
        }
    }
    resources.settle(renderer, surfaces);
    resources.check(renderer, surfaces)
}

/// Samples compositor-owned tracks before a commit can retarget them.
/// Hidden surfaces keep their state until they become visible again.
fn sample_owned<B: Backend>(
    renderer: &B::Renderer,
    surfaces: &mut FxHashMap<SurfaceId, SurfaceState>,
    time: crate::Instant,
) {
    for (id, state) in surfaces
        .iter_mut()
        .filter(|(_, state)| state.visibility == Visibility::Visible)
    {
        let owned = renderer.owned_animations(*id);
        if !owned.is_empty() {
            state
                .tree
                .sample_owned(time, |layer| owned.contains(&layer));
        }
    }
}

/// Samples and lists visible surfaces, retaining their cadence until the
/// backend has accepted or withdrawn this frame's animation handoffs.
fn sample_frames(
    surfaces: &mut FxHashMap<SurfaceId, SurfaceState>,
    time: crate::Instant,
) -> Vec<SurfaceFrame<'_>> {
    let mut frames: Vec<SurfaceFrame<'_>> = Vec::with_capacity(surfaces.len());
    for (id, state) in surfaces
        .iter_mut()
        .filter(|(_, state)| state.visibility == Visibility::Visible)
    {
        let sampling = state.tree.sample(time, state.display);
        let changed = state.commits != Commits::Clean || sampling.stepped;
        state.sampled_rate = sampling.rate;
        frames.push(SurfaceFrame {
            id: *id,
            size: state.size,
            display: state.display,
            clear: state.clear,
            changed,
            // A stepped animation changed the sampled tree: the frame is
            // never plane-only, however it was committed.
            plane_frames: if sampling.stepped || state.commits != Commits::Installs {
                None
            } else {
                Some(&state.plane_frames).filter(|frames| !frames.is_empty())
            },
            present_pending: state.present_pending(),
            display_moved: state.display_moved,
            tree: &state.tree,
        });
    }
    frames
}

/// Consumes the per-frame state of every surface the frame listed, and
/// answers when the next frame is needed: the animations' refresh class
/// combined with the backend's.
fn finish_frame<B: Backend>(
    renderer: &B::Renderer,
    surfaces: &mut FxHashMap<SurfaceId, SurfaceState>,
    time: crate::Instant,
    redraw: Redraw,
) -> Next {
    let mut rate = None;
    for (id, state) in surfaces
        .iter_mut()
        .filter(|(_, state)| state.visibility == Visibility::Visible)
    {
        let owned = renderer.owned_animations(*id);
        let running = if state.content_animating {
            Some(crate::tree::RATE_FAST)
        } else if owned.is_empty() {
            state.sampled_rate.take()
        } else {
            state.tree.animation_rate(|layer| owned.contains(&layer))
        };
        match running {
            Some(r) if r == crate::tree::RATE_FAST => rate = Some(crate::tree::RATE_FAST),
            Some(r) => rate = rate.or(Some(r)),
            None => {}
        }
        state.commits = Commits::Clean;
        state.plane_frames.clear();
        state.display_moved = false;
        state.presented();
    }
    let rate = match redraw {
        Redraw::None => rate,
        Redraw::Wanted { rate: backend_rate } => Some(rate.map_or_else(
            || backend_rate.clone(),
            |r| (*r.start()).min(*backend_rate.start())..=(*r.end()).max(*backend_rate.end()),
        )),
    };
    rate.map_or(Next::Idle, |rate| Next::At {
        time: time + Duration::from_secs_f64(1.0 / f64::from(*rate.end())),
        rate,
    })
}

/// One frame: apply every commit, sample, render, answer.
#[cfg(not(target_arch = "wasm32"))]
fn render<B: Backend>(
    renderer: &mut B::Renderer,
    surfaces: &mut FxHashMap<SurfaceId, SurfaceState>,
    resources: &mut Resources<B>,
    id: FrameId,
    time: crate::Instant,
    commits: &mut [(SurfaceId, ChangeSet<B>)],
) -> Result<(Next, FrameStats), RenderError> {
    sample_owned::<B>(renderer, surfaces, time);
    apply_commits(renderer, surfaces, resources, commits)?;
    let frames = sample_frames(surfaces, time);
    let mut stats = FrameStats::default();
    let redraw = renderer.render(
        &Frame {
            id,
            time: crate::frame::FrameTime(time),
            surfaces: &frames,
        },
        &mut stats,
    )?;
    drop(frames);
    Ok((finish_frame::<B>(renderer, surfaces, time, redraw), stats))
}

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn render_local<B: Backend>(
    renderer: &mut B::Renderer,
    surfaces: &mut FxHashMap<SurfaceId, SurfaceState>,
    resources: &mut Resources<B>,
    id: FrameId,
    time: crate::Instant,
    commits: &mut [(SurfaceId, ChangeSet<B>)],
) -> Result<(Next, FrameStats), RenderError> {
    sample_owned::<B>(renderer, surfaces, time);
    apply_commits(renderer, surfaces, resources, commits)?;
    let frames = sample_frames(surfaces, time);
    let mut stats = FrameStats::default();
    let redraw = renderer
        .render(
            &Frame {
                id,
                time: crate::frame::FrameTime(time),
                surfaces: &frames,
            },
            &mut stats,
        )
        .await?;
    drop(frames);
    Ok((finish_frame::<B>(renderer, surfaces, time, redraw), stats))
}

#[cfg(all(test, feature = "testing", not(target_arch = "wasm32")))]
mod tests {
    use super::{Commits, Presentation, SurfaceState, commit};
    use crate::backend::Visibility;
    use crate::backend::{Backend, Display};
    use crate::display_list::{DisplayList, Picture};
    use crate::engine::{SurfaceWaker, Waker};
    use crate::message::{ChangeSet, ContentOp, LayerId, LayerOp, Op, SurfaceId};
    use crate::testing::{Null, NullConfig};
    use crate::tree::SurfaceTree;
    use crate::{Draw, WorkingColor};

    #[test]
    fn caller_shared_picture_is_not_recycled() {
        let (events, _receiver) = std::sync::mpsc::channel();
        let (mut renderer, ()) = <Null as Backend>::init(NullConfig {
            events,
            reject: std::collections::HashSet::new(),
        })
        .expect("null backend");
        let surface = SurfaceId::new(1);
        let layer = LayerId::new(0);
        let caller_picture = Picture::record(|c| {
            c.fill(crate::kurbo::Rect::new(0., 0., 1., 1.), WorkingColor::WHITE);
        });
        let mut state = SurfaceState {
            tree: SurfaceTree::new(),
            size: (1, 1),
            display: Display::default(),
            clear: WorkingColor::TRANSPARENT,
            commits: Commits::Clean,
            plane_frames: rustc_hash::FxHashSet::default(),
            presentation: Presentation::Retained,
            display_moved: false,
            content_animating: false,
            sampled_rate: None,
            visibility: Visibility::Visible,
            waker: std::sync::Arc::new(SurfaceWaker::new(std::sync::Arc::new(Waker::new()))),
        };

        let mut first = ChangeSet::<Null> {
            clear: None,
            ops: vec![Op::Layer(LayerOp::Content(
                layer,
                Some(ContentOp::Picture(caller_picture.clone())),
            ))],
            recycled: Vec::new(),
            animating: false,
        };
        commit(&mut renderer, &mut state, surface, &mut first);
        let mut second = ChangeSet::<Null> {
            clear: None,
            ops: vec![Op::Layer(LayerOp::Content(
                layer,
                Some(ContentOp::Picture(Picture::from_list(
                    DisplayList::default(),
                ))),
            ))],
            recycled: Vec::new(),
            animating: false,
        };

        commit(&mut renderer, &mut state, surface, &mut second);

        assert_eq!(second.recycled, []);
        assert_eq!(caller_picture.display_list().len(), 1);
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) async fn local<B: Backend>(
    config: B::Config,
) -> Result<(crate::local::Sender<Message<B>>, B::Info), EngineError> {
    use std::cell::RefCell;
    use std::rc::Rc;
    let (renderer, info) = B::init(config).await?;
    let state = Rc::new(RefCell::new(Some(LocalState::<B> {
        renderer,
        surfaces: FxHashMap::default(),
        resources: Resources::default(),
        next_frame: 0,
    })));
    let tx = crate::local::Sender::new(move |message| {
        let mut owned = state.borrow_mut().take().expect("serial local executor");
        let state = Rc::clone(&state);
        Box::pin(async move {
            let live = owned.apply(message).await;
            *state.borrow_mut() = Some(owned);
            live
        })
    });
    Ok((tx, info))
}

#[cfg(target_arch = "wasm32")]
struct LocalState<B: Backend> {
    renderer: B::Renderer,
    surfaces: FxHashMap<SurfaceId, SurfaceState>,
    resources: Resources<B>,
    next_frame: u64,
}
#[cfg(target_arch = "wasm32")]
impl<B: Backend> LocalState<B> {
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn apply(&mut self, message: Message<B>) -> bool {
        let Self {
            renderer,
            surfaces,
            resources,
            next_frame,
        } = self;
        match message {
            Message::CreateSurface {
                id,
                target,
                waker,
                reply,
            } => {
                let _ = reply.send(create_surface::<B>(renderer, surfaces, id, target, waker));
            }
            Message::ResizeSurface { id, size } => {
                resize_surface::<B>(renderer, surfaces, id, size);
            }
            Message::DestroySurface { id } => {
                destroy_surface::<B>(renderer, surfaces, resources, id);
            }
            Message::Display { id, display } => set_display(surfaces, id, display),
            Message::DisplayMoved { id } => set_display_moved(surfaces, id),
            Message::Visibility { id, visibility } => {
                set_visibility::<B>(renderer, surfaces, id, visibility);
            }
            Message::Resource(op) => op(renderer),
            Message::ProducerFrame { opaque, apply, .. } => {
                producer_frame::<B>(renderer, surfaces, opaque, apply);
            }
            Message::Register { resource, op } => {
                let result = op(renderer).await;
                resources.register(resource, result);
            }
            Message::Release { resource, op } => {
                resources.release(renderer, surfaces, resource, op);
            }
            Message::ReplaceImage { id, image } => {
                replace_image::<B>(renderer, surfaces, resources, id, image);
            }
            Message::Apply { id, mut changes } => {
                apply_hidden::<B>(renderer, surfaces, resources, id, &mut changes);
            }
            Message::Render {
                time,
                mut commits,
                reply,
            } => {
                let id = FrameId(*next_frame);
                *next_frame += 1;
                let result =
                    render_local::<B>(renderer, surfaces, resources, id, time.0, &mut commits)
                        .await;
                let _ = reply.send(crate::message::RenderReply { result, commits });
            }
            Message::FinishTimings { reply } => {
                let _ = reply.send(renderer.finish_timings().await);
            }
            Message::Readback { surface, reply } => {
                let _ = reply.send(renderer.readback(surface).await);
            }
            Message::Memory { reply } => {
                let _ = reply.send(crate::message::MemoryReply {
                    usage: renderer.memory(),
                });
            }
            Message::Trim(pressure) => renderer.trim(pressure),
            Message::Shutdown => return false,
        }
        true
    }
}
