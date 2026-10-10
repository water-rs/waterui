//! A surface's layer tree as its consumer holds it: the layer graph,
//! every layer property, its animation track and the sampled value for
//! the current frame.
//!
//! The consumer never receives property ops; it reads the sampled tree.

mod components;
mod projective;

use crate::Instant;
use rustc_hash::FxHashMap;

use kurbo::{Affine, Vec2};

use crate::animation::{
    Animatable, Animation, AnimationTrack, Lanes, clamp_to_rect, eval_lanes, rubber_band_spring,
};
use crate::backdrop::BackdropSample;
use crate::display_list::{Operand, SlotUpdate, blends_within, translucent_within};
use crate::frame::RefreshRange;
use crate::ops::{ContentOp, Install, LayerId, LayerOp, Op, Prop};
use crate::projective::{Projective, ProjectiveError};
use crate::shape::ShapeData;
use crate::style::{BlendMode, FilterId};
use crate::target::Target;

/// The fast rate class: springs, curves and fast decays run here.
pub const RATE_FAST: RefreshRange = 60..=120;
/// The slow rate class: only decays slower than one device pixel per frame
/// at 60 Hz remain.
pub const RATE_SLOW: RefreshRange = 30..=60;

/// Complete affine and opacity tracks available for compositor handoff.
#[derive(Clone, Debug)]
pub struct LayerAnimations {
    /// The affine motion, when running.
    pub transform: Option<AnimationTrack<Affine>>,
    /// The opacity motion, when running.
    pub opacity: Option<AnimationTrack<f32>>,
}

/// A surface's layer tree as its consumer holds it.
#[derive(Debug)]
pub struct SurfaceTree {
    nodes: FxHashMap<u64, LayerNode>,
    root: LayerId,
    /// Projective layers' state, by layer. Empty for an affine-only tree.
    projective: FxHashMap<u64, projective::State>,
    /// The last change stamp handed out; stamps only grow.
    clock: u64,
}

/// What applying one [`Op`] asks the consumer to realise — the return of
/// [`SurfaceTree::apply_op`]. The tree side of the op is already applied;
/// these are the parts only the consumer can do.
pub enum Realize<T: Target> {
    /// `layer` left the tree: drop the consumer's caches keyed on it.
    Remove(LayerId),
    /// `layer`'s content changed: hand `content` to the consumer's content
    /// slot — `None` clears it.
    Content(LayerId, Option<ContentOp>),
    /// A sealed install payload for `layer`: the consumer runs it and
    /// notes the alpha it reports back through
    /// [`SurfaceTree::note_installed`].
    Install(LayerId, Install<T>),
    /// A tree mutation the consumer does not mirror.
    Applied,
}

impl<T: Target> std::fmt::Debug for Realize<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Remove(layer) => f.debug_tuple("Remove").field(layer).finish(),
            Self::Content(layer, content) => f
                .debug_tuple("Content")
                .field(layer)
                .field(content)
                .finish(),
            Self::Install(layer, install) => f
                .debug_tuple("Install")
                .field(layer)
                .field(install)
                .finish(),
            Self::Applied => f.write_str("Applied"),
        }
    }
}

/// One layer's sampled state for the current frame.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is an independent, orthogonal state bit of a sampled layer"
)]
pub struct LayerNode {
    /// The local transform.
    pub transform: Affine,
    /// The opacity.
    pub opacity: f32,
    /// The scroll offset, snapped to the display's device-pixel grid.
    pub scroll_offset: Vec2,
    /// The clip shape, in the layer's own space.
    pub clip: Option<ShapeData>,
    /// The blend mode the layer composites onto its parent with.
    pub blend: BlendMode,
    /// The filter applied to this layer's subtree.
    pub filter: Option<FilterId>,
    /// The backdrop group (and optional per-member effect) this layer
    /// samples.
    pub backdrop: Option<BackdropSample>,
    /// The child layers, in paint order.
    pub children: Vec<LayerId>,
    /// Counts direct children that blend; each such child isolates itself so
    /// ancestors above it receive a normal composite.
    blending_children: u32,
    /// Content updates conservatively retain this until replacement; extra
    /// pass-through isolation is output-equivalent.
    content_blends: bool,
    /// Content updates conservatively retain this until replacement: the
    /// recorded commands may paint a pixel of alpha below one, so the
    /// layer's output is not known to be opaque. Installed content —
    /// a `GpuContent` or external frame the engine does not record —
    /// counts by the alpha contract the producer declared at install.
    content_translucent: bool,
    /// Recorded or installed content of its own exists — a pure container,
    /// which only composites children, has none.
    has_content: bool,
    parent: Option<LayerId>,
    transform_track: Option<Track<Affine>>,
    components: Option<Box<components::Components>>,
    opacity_track: Option<Track<f32>>,
    scroll_track: Option<Track<Vec2>>,
    projective: bool,
    animation_rate: Option<RefreshRange>,
    /// The stamp of the last change to what this layer draws in its own
    /// space: clip, scroll offset, filter, backdrop, content, children.
    inner_stamp: u64,
    /// The stamp of the last change to how it composes into its parent:
    /// transform, components, projection, opacity, blend.
    outer_stamp: u64,
}

impl std::fmt::Debug for LayerNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayerNode")
            .field("transform", &self.transform)
            .field("opacity", &self.opacity)
            .field("scroll_offset", &self.scroll_offset)
            .field("clip", &self.clip)
            .field("blend", &self.blend)
            .field("filter", &self.filter)
            .field("backdrop", &self.backdrop)
            .field("children", &self.children)
            .finish_non_exhaustive()
    }
}

impl LayerNode {
    /// The affine track, including its original start and velocity.
    /// Component transforms are returned only when their sole moving
    /// component is translation, which is affine-linear in the same lanes.
    #[must_use]
    fn transform_animation(&self) -> Option<AnimationTrack<Affine>> {
        match &self.components {
            None => self.transform_track.as_ref()?.description(),
            Some(components) if self.transform_track.is_none() => {
                components.translation_animation()
            }
            Some(_) => None,
        }
    }

    /// Describes all running tracks together. Returns `None` for scroll,
    /// projective or nonlinear component motion, or a track whose start is
    /// still unknown — one that carries no host-given start and has never
    /// been sampled. An empty description means that no property is moving.
    #[must_use]
    pub fn animations(&self) -> Option<LayerAnimations> {
        if self.projective || self.scroll_track.is_some() {
            return None;
        }
        let transform = self.transform_animation();
        if self.animating() && transform.is_none() {
            return None;
        }
        let opacity = match &self.opacity_track {
            Some(track) => Some(track.description()?),
            None => None,
        };
        Some(LayerAnimations { transform, opacity })
    }

    /// Records a sampled change: `outer` for how the layer composes,
    /// `inner` for what it draws in its own space.
    const fn restamp(&mut self, clock: &mut u64, outer: bool, inner: bool) {
        if outer {
            *clock += 1;
            self.outer_stamp = *clock;
        }
        if inner {
            *clock += 1;
            self.inner_stamp = *clock;
        }
    }

    fn new() -> Self {
        Self {
            transform: Affine::IDENTITY,
            opacity: 1.0,
            scroll_offset: Vec2::ZERO,
            clip: None,
            blend: BlendMode::default(),
            filter: None,
            backdrop: None,
            children: Vec::new(),
            blending_children: 0,
            content_blends: false,
            content_translucent: false,
            has_content: false,
            parent: None,
            transform_track: None,
            components: None,
            opacity_track: None,
            scroll_track: None,
            projective: false,
            animation_rate: None,
            inner_stamp: 0,
            outer_stamp: 0,
        }
    }

    /// `transform * translate(-scroll_offset)`: the space of the content
    /// and children. The clip applies in `transform` space, so scrolling
    /// moves content and children inside the clip and never re-records
    /// anything.
    #[must_use]
    pub fn content_transform(&self) -> Affine {
        self.transform * Affine::translate(-self.scroll_offset)
    }

    /// Whether a child layer, or a group in this layer's own content, composites
    /// onto this layer with a non-`Normal` blend. Such a layer isolates like a
    /// group with a blended descendant.
    #[must_use]
    pub const fn blends_within(&self) -> bool {
        self.blending_children > 0 || self.content_blends
    }

    /// Whether the layer's recorded or installed content may paint a pixel
    /// of alpha below one — `false` only while the layer has no content or
    /// every command it draws is provably opaque.
    #[must_use]
    pub const fn content_translucent(&self) -> bool {
        self.content_translucent
    }

    /// Whether the layer has recorded or installed content of its own —
    /// what distinguishes a layer that paints from a pure container.
    #[must_use]
    pub const fn has_content(&self) -> bool {
        self.has_content
    }

    fn classify_rate(&self, scale: f64, components_running: bool) -> Option<RefreshRange> {
        if components_running {
            return Some(RATE_FAST);
        }
        let mut rate = None;
        for fast in [
            self.transform_track.as_ref().map(|t| t.is_fast(scale)),
            self.opacity_track.as_ref().map(|t| t.is_fast(scale)),
            self.scroll_track.as_ref().map(|t| t.is_fast(scale)),
        ]
        .into_iter()
        .flatten()
        {
            if fast {
                return Some(RATE_FAST);
            }
            rate = Some(RATE_SLOW);
        }
        rate
    }

    /// Whether an engine-driven track moved this layer this frame: a
    /// running transform, component or scroll track. False on the frame
    /// a track settles.
    #[must_use]
    pub const fn animating(&self) -> bool {
        self.transform_track.is_some()
            || self.scroll_track.is_some()
            || match &self.components {
                Some(components) => components.animating(),
                None => false,
            }
    }
}

/// One running animation track.
struct Track<T: Animatable> {
    /// The position the track started from (its retarget snapshot).
    from: T::Lanes,
    /// The velocity the track started with.
    velocity: T::Lanes,
    /// The value the track moves toward.
    target: T,
    /// The animation driving the track.
    animation: Animation,
    /// The time the track started. `Some` when the prop gave a start on
    /// the host's clock; `None` until first sampled, so a track committed
    /// between frames starts at the next presentation time.
    start: Option<Instant>,
    /// The last sampled `(time, position, velocity)`, for retarget
    /// continuity.
    last: Option<(Instant, T::Lanes, T::Lanes)>,
}

impl<T: Animatable> std::fmt::Debug for Track<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Track")
            .field("animation", &self.animation)
            .field("start", &self.start)
            .finish_non_exhaustive()
    }
}

impl<T: Animatable> Track<T> {
    fn description(&self) -> Option<AnimationTrack<T>> {
        Some(AnimationTrack {
            from: T::from_lanes(self.from),
            velocity: self.velocity,
            target: self.target,
            animation: self.animation,
            start: self.start?,
        })
    }

    fn new(
        from: T::Lanes,
        velocity: T::Lanes,
        target: T,
        animation: Animation,
        start: Option<Instant>,
    ) -> Self {
        debug_assert!(
            !matches!(animation, Animation::Decay(_)) || T::Lanes::N == 2,
            "Decay is only legal on scroll_offset"
        );
        Self {
            from,
            velocity,
            target,
            animation,
            start,
            last: None,
        }
    }

    /// Evaluates the track at `t`, returning `(position, velocity, settled)`
    /// in lane space. A settled spring reports its target exactly.
    fn sample(&mut self, t: Instant) -> (T::Lanes, T::Lanes, bool) {
        let start = *self.start.get_or_insert(t);
        let dt = t.duration_since(start).as_secs_f64();
        let (pos, vel, done) = eval_lanes(
            self.from,
            self.velocity,
            self.target.into_lanes(),
            &self.animation,
            dt,
        );
        self.last = Some((t, pos, vel));
        (pos, vel, done)
    }

    /// The `(position, velocity)` the track's evaluation reports at `t`,
    /// without sampling it: what a retarget with a host-given start
    /// continues from. An unstarted track is evaluated as though `t` were
    /// its first frame.
    fn value_at(&self, t: Instant) -> (T::Lanes, T::Lanes) {
        let dt = self
            .start
            .map_or(0.0, |start| t.duration_since(start).as_secs_f64());
        let (pos, vel, _) = eval_lanes(
            self.from,
            self.velocity,
            self.target.into_lanes(),
            &self.animation,
            dt,
        );
        (pos, vel)
    }

    /// Whether the still-running track needs the fast rate class. A decay
    /// needs only the slow class once it runs slower than one device pixel
    /// per frame at 60 Hz (`scale * 60` logical pixels per second).
    fn is_fast(&self, scale: f64) -> bool {
        match self.animation {
            Animation::Decay(_) => {
                self.last.map_or(self.velocity, |(_, _, vel)| vel).max_abs() >= scale * 60.0
            }
            Animation::Spring(_) | Animation::Curve(_) => true,
        }
    }
}

/// The outcome of [`SurfaceTree::sample`].
#[derive(Clone, Debug)]
pub struct Sampling {
    /// Whether any animation step touched the tree.
    pub stepped: bool,
    /// The refresh rate still-running animations need.
    pub rate: Option<RefreshRange>,
}

impl Default for SurfaceTree {
    fn default() -> Self {
        Self::new()
    }
}

impl SurfaceTree {
    /// Refreshes compositor-owned motion before a new transaction retargets
    /// it. Without engine frames, `last` otherwise describes the handoff
    /// frame rather than the position and velocity currently on screen.
    pub fn sample_owned(&mut self, time: Instant, owns: impl Fn(LayerId) -> bool) {
        for (&raw, node) in &mut self.nodes {
            if !owns(LayerId::new(raw)) {
                continue;
            }
            if let Some(track) = &mut node.transform_track {
                let (position, _, _) = track.sample(time);
                node.transform = Affine::from_lanes(position);
            }
            if let Some(components) = &mut node.components {
                components.sample(time);
                node.transform = components.matrix();
                // Component sampling clears settled tracks. The ordinary
                // sampler can no longer observe that final movement, so
                // preserve its dependency change before a possible demotion.
                node.restamp(&mut self.clock, true, false);
            }
            if let Some(track) = &mut node.opacity_track {
                let (position, _, _) = track.sample(time);
                node.opacity = f32::from_lanes(position);
            }
        }
    }

    /// Rate required by tracks the backend has not accepted. Called after
    /// presentation so a handoff suppresses the very next frame, and a
    /// demotion resumes scheduling immediately.
    pub fn animation_rate(&self, owns: impl Fn(LayerId) -> bool) -> Option<RefreshRange> {
        let mut running = false;
        for (&raw, node) in &self.nodes {
            if owns(LayerId::new(raw)) {
                continue;
            }
            if node.animation_rate == Some(RATE_FAST) {
                return Some(RATE_FAST);
            }
            running |= node.animation_rate.is_some();
        }
        running.then_some(RATE_SLOW)
    }

    /// An empty tree holding only its root layer (`LayerId(0)`).
    #[must_use]
    pub fn new() -> Self {
        let mut nodes = FxHashMap::default();
        nodes.insert(0, LayerNode::new());
        Self {
            nodes,
            root: LayerId::new(0),
            projective: FxHashMap::default(),
            clock: 0,
        }
    }

    /// Whether any layer in the tree is projective. An affine-only tree
    /// takes the backends' existing affine traversal unchanged.
    #[must_use]
    pub fn has_projective(&self) -> bool {
        !self.projective.is_empty()
    }

    /// The complete sampled local-to-parent pose of a projective layer
    /// (its projection composed with its tilt, depth and affine
    /// components), or `None` for an affine layer. A projective layer's
    /// placement is this pose; its [`LayerNode::transform`] is not applied
    /// as well. The pose is validated after composition: an invalid
    /// composition is an error, never an identity.
    #[must_use]
    pub fn projective_pose(&self, id: LayerId) -> Option<Result<Projective, ProjectiveError>> {
        self.projective.get(&id.raw()).map(|state| state.pose)
    }

    /// A value that changes whenever anything a projective layer's local
    /// image depends on changes: every property and content of every
    /// descendant, and the layer's own clip, scroll offset, filter,
    /// backdrop, content and children. The layer's own pose, opacity and
    /// blend are excluded: they only change how the image is composed.
    #[must_use]
    pub fn content_stamp(&self, id: LayerId) -> u64 {
        let node = self.layer(id);
        node.children.iter().fold(node.inner_stamp, |stamp, child| {
            stamp.max(self.subtree_stamp(*child))
        })
    }

    /// Version of the pixels composited by the engine, excluding only the
    /// outer properties of layers whose pixels belong to system planes.
    /// Content, clips, child order and every unpromoted property remain
    /// dependencies. Backends separately validate plane eligibility and
    /// resource changes before reusing the engine parts.
    #[must_use]
    pub fn composition_stamp(&self, promoted: impl Fn(LayerId) -> bool) -> u64 {
        self.nodes.iter().fold(0, |stamp, (&raw, node)| {
            stamp
                .max(node.inner_stamp)
                .max(if promoted(LayerId::new(raw)) {
                    0
                } else {
                    node.outer_stamp
                })
        })
    }

    fn subtree_stamp(&self, id: LayerId) -> u64 {
        let node = self.layer(id);
        node.children
            .iter()
            .fold(node.inner_stamp.max(node.outer_stamp), |stamp, child| {
                stamp.max(self.subtree_stamp(*child))
            })
    }

    fn stamp(&mut self, id: LayerId, inner: bool) {
        self.clock += 1;
        let clock = self.clock;
        let node = self.node_mut(id);
        if inner {
            node.inner_stamp = clock;
        } else {
            node.outer_stamp = clock;
        }
    }

    fn projective_mut(&mut self, id: LayerId) -> &mut projective::State {
        assert!(
            self.nodes.contains_key(&id.raw()),
            "layer {} is not in the tree",
            id.raw()
        );
        self.node_mut(id).projective = true;
        self.projective
            .entry(id.raw())
            .or_insert_with(projective::State::new)
    }

    /// Sets `id`'s blend, keeping its parent's blending-child count.
    fn set_blend(&mut self, id: LayerId, blend: BlendMode) {
        let node = self.layer(id);
        let parent = node.parent;
        let was_blending = node.blend != BlendMode::Normal;
        let is_blending = blend != BlendMode::Normal;
        if was_blending != is_blending
            && let Some(parent) = parent
        {
            let count = &mut self.node_mut(parent).blending_children;
            if is_blending {
                *count += 1;
            } else {
                *count -= 1;
            }
        }
        self.node_mut(id).blend = blend;
    }

    /// Applies a projection, tilt, depth or clear-projection op.
    fn apply_projective(&mut self, op: LayerOp) {
        match op {
            LayerOp::Projection(id, base) => self.projective_mut(id).set_projection(base),
            LayerOp::Tilt(id, prop) => self.projective_mut(id).set_tilt(&prop),
            LayerOp::Depth(id, prop) => self.projective_mut(id).set_depth(&prop),
            LayerOp::ClearProjection(id) => {
                assert!(
                    self.nodes.contains_key(&id.raw()),
                    "layer {} is not in the tree",
                    id.raw()
                );
                self.projective.remove(&id.raw());
                self.node_mut(id).projective = false;
            }
            _ => unreachable!("only projective ops reach apply_projective"),
        }
    }

    /// Steps every projective layer's tilt and depth tracks and
    /// recomposes its pose from the freshly sampled affine components.
    /// Returns `(stepped, still running)`.
    fn sample_projective(&mut self, time: Instant) -> (bool, bool) {
        let (mut stepped, mut running) = (false, false);
        for (id, state) in &mut self.projective {
            let (step, run) = state.sample(time);
            stepped |= step;
            running |= run;
            let node = self
                .nodes
                .get_mut(id)
                .expect("projective state belongs to a layer in the tree");
            if step {
                self.clock += 1;
                node.outer_stamp = self.clock;
            }
            state.refresh(node);
            if run {
                node.animation_rate = Some(RATE_FAST);
            }
        }
        (stepped, running)
    }

    fn refresh_projective(&mut self, id: LayerId) {
        if let Some(state) = self.projective.get_mut(&id.raw()) {
            let node = self
                .nodes
                .get(&id.raw())
                .unwrap_or_else(|| panic!("layer {} is not in the tree", id.raw()));
            state.refresh(node);
        }
    }

    /// The root layer.
    #[must_use]
    pub const fn root(&self) -> LayerId {
        self.root
    }

    /// The sampled node of layer `id`.
    ///
    /// # Panics
    /// Panics when `id` is not in the tree.
    #[must_use]
    pub fn layer(&self, id: LayerId) -> &LayerNode {
        self.nodes
            .get(&id.raw())
            .unwrap_or_else(|| panic!("layer {} is not in the tree", id.raw()))
    }

    /// Records whether the layer's content contains a non-`Normal` group
    /// or may paint a pixel of alpha below one.
    pub fn note_content(&mut self, id: LayerId, content: Option<&ContentOp>) {
        let node = self.node_mut(id);
        node.has_content = content.is_some();
        match content {
            Some(ContentOp::Replace(picture) | ContentOp::Picture(picture)) => {
                let list = picture.display_list();
                node.content_blends = blends_within(list, 0..list.len());
                node.content_translucent = translucent_within(list, 0..list.len());
            }
            Some(ContentOp::Update(updates)) => {
                node.content_blends |= updates.iter().any(|SlotUpdate { value, .. }| {
                    matches!(value, Operand::Group(group) if group.blend != BlendMode::Normal)
                });
                node.content_translucent |=
                    updates.iter().any(|SlotUpdate { value, .. }| match value {
                        Operand::Paint(_) | Operand::Shadow(_) | Operand::Run(_) => true,
                        Operand::Group(group) => {
                            group.opacity < 1.0
                                || group.blend != BlendMode::Normal
                                || group.filter.is_some()
                        }
                        Operand::Shape(_)
                        | Operand::Stroke(_)
                        | Operand::Transform(_)
                        | Operand::Rect(_) => false,
                    });
            }
            None => {
                node.content_blends = false;
                node.content_translucent = false;
            }
        }
    }

    /// Records installed render-side content (`Op::Install`,
    /// `Message::ProducerFrame`) replacing what the layer drew: no
    /// recorded groups remain, and the installed pixels' alpha is the
    /// producer's to declare — `opaque` is that declaration, `false`
    /// where the producer declares none, so the layer is not known to
    /// be opaque.
    pub fn note_installed(&mut self, id: LayerId, opaque: bool) {
        let node = self.node_mut(id);
        node.content_blends = false;
        node.content_translucent = !opaque;
        node.has_content = true;
    }

    /// Every layer in the tree. Order is unspecified.
    pub fn layers(&self) -> impl Iterator<Item = (LayerId, &LayerNode)> + '_ {
        self.nodes
            .iter()
            .map(|(id, node)| (LayerId::new(*id), node))
    }

    /// Removes `id` — only `id`. Its children stay in the tree, detached
    /// and undrawn, until their own `Remove` or re-attachment: a `Layer`
    /// handle owns exactly its own layer.
    ///
    /// # Panics
    /// Panics when `id` is not in the tree or is the root.
    pub fn remove(&mut self, id: LayerId) {
        assert!(
            self.nodes.contains_key(&id.raw()),
            "removing unknown layer {}",
            id.raw()
        );
        assert_ne!(id, self.root, "the root layer cannot be removed");
        self.detach(id);
        let node = self.nodes.remove(&id.raw()).expect("checked above");
        for child in node.children {
            self.node_mut(child).parent = None;
        }
        self.projective.remove(&id.raw());
    }

    /// Applies one committed layer op.
    ///
    /// # Panics
    /// Panics on an op naming a layer that is not in the tree: after
    /// `Create` ordering is respected, an unknown layer is an invariant
    /// violation. Attaching the root or closing a cycle also panics before
    /// mutating the tree.
    pub fn apply(&mut self, op: LayerOp) {
        let touched = match &op {
            LayerOp::Create(_) | LayerOp::Remove(_) => None,
            LayerOp::Transform(id, _)
            | LayerOp::Translation(id, _)
            | LayerOp::Rotation(id, _)
            | LayerOp::Scale(id, _)
            | LayerOp::Skew(id, _)
            | LayerOp::Pivot(id, _)
            | LayerOp::Projection(id, _)
            | LayerOp::Tilt(id, _)
            | LayerOp::Depth(id, _)
            | LayerOp::ClearProjection(id)
            | LayerOp::Opacity(id, _)
            | LayerOp::Blend(id, _) => Some((*id, false)),
            LayerOp::ScrollOffset(id, _)
            | LayerOp::Clip(id, _)
            | LayerOp::Filter(id, _)
            | LayerOp::Backdrop(id, _)
            | LayerOp::Content(id, _) => Some((*id, true)),
            LayerOp::Push { parent, .. }
            | LayerOp::Insert { parent, .. }
            | LayerOp::Detach { parent, .. } => Some((*parent, true)),
        };
        self.apply_layer_op(op);
        if let Some((id, inner)) = touched {
            self.stamp(id, inner);
            self.refresh_projective(id);
        }
    }

    /// Applies one op from a drained [`ChangeSet`] and returns what it
    /// asks the consumer to realise. Removes route through
    /// [`remove`](Self::remove) — [`apply`](Self::apply) panics on them —
    /// and a content op notes its picture slots on the layer before it
    /// is handed over, so a consumer mirrors a commit with this one
    /// call, then notes an install's reported alpha back through
    /// [`note_installed`](Self::note_installed).
    pub fn apply_op<T: Target>(&mut self, op: Op<T>) -> Realize<T> {
        match op {
            Op::Layer(LayerOp::Remove(layer)) => {
                self.remove(layer);
                Realize::Remove(layer)
            }
            Op::Layer(LayerOp::Content(layer, content)) => {
                self.apply(LayerOp::Content(layer, None));
                self.note_content(layer, content.as_ref());
                Realize::Content(layer, content)
            }
            Op::Layer(op) => {
                self.apply(op);
                Realize::Applied
            }
            Op::Install(layer, install) => Realize::Install(layer, install),
        }
    }

    fn apply_layer_op(&mut self, op: LayerOp) {
        match op {
            LayerOp::Create(id) => {
                assert!(
                    self.nodes.insert(id.raw(), LayerNode::new()).is_none(),
                    "layer {} created twice",
                    id.raw()
                );
            }
            LayerOp::Remove(_) => unreachable!("Remove goes through `apply_op` or `remove`"),
            LayerOp::Transform(id, prop) => {
                let node = self.node_mut(id);
                if let Some(components) = &mut node.components {
                    set_prop(&mut node.transform_track, &mut components.base, &prop);
                    node.transform = components.matrix();
                } else {
                    set_prop(&mut node.transform_track, &mut node.transform, &prop);
                }
            }
            LayerOp::Translation(id, prop) => self.node_mut(id).set_translation(prop),
            LayerOp::Rotation(id, prop) => self.node_mut(id).set_rotation(prop),
            LayerOp::Scale(id, prop) => self.node_mut(id).set_scale(prop),
            LayerOp::Skew(id, prop) => self.node_mut(id).set_skew(prop),
            LayerOp::Pivot(id, prop) => self.node_mut(id).set_pivot(prop),
            op @ (LayerOp::Projection(..)
            | LayerOp::Tilt(..)
            | LayerOp::Depth(..)
            | LayerOp::ClearProjection(_)) => self.apply_projective(op),
            LayerOp::Opacity(id, prop) => {
                let node = self.node_mut(id);
                set_prop(&mut node.opacity_track, &mut node.opacity, &prop);
            }
            LayerOp::ScrollOffset(id, prop) => {
                let node = self.node_mut(id);
                set_prop(&mut node.scroll_track, &mut node.scroll_offset, &prop);
            }
            LayerOp::Clip(id, clip) => self.node_mut(id).clip = clip,
            LayerOp::Blend(id, blend) => self.set_blend(id, blend),
            LayerOp::Filter(id, filter) => self.node_mut(id).filter = filter,
            LayerOp::Backdrop(id, backdrop) => self.node_mut(id).backdrop = backdrop,
            LayerOp::Content(id, _) => {
                // Validated here; forwarded to the renderer by the loop.
                assert!(
                    self.nodes.contains_key(&id.raw()),
                    "content on unknown layer {}",
                    id.raw()
                );
            }
            LayerOp::Push { parent, child } => {
                assert!(
                    self.nodes.contains_key(&child.raw()),
                    "pushing unknown layer {}",
                    child.raw()
                );
                self.assert_attachment(parent, child);
                self.detach(child);
                let blends = self.layer(child).blend != BlendMode::Normal;
                self.node_mut(child).parent = Some(parent);
                let parent_node = self.node_mut(parent);
                parent_node.children.push(child);
                parent_node.blending_children += u32::from(blends);
            }
            LayerOp::Insert {
                parent,
                index,
                child,
            } => {
                assert!(
                    self.nodes.contains_key(&child.raw()),
                    "inserting unknown layer {}",
                    child.raw()
                );
                self.assert_attachment(parent, child);
                self.detach(child);
                let blends = self.layer(child).blend != BlendMode::Normal;
                self.node_mut(child).parent = Some(parent);
                let node = self.node_mut(parent);
                let index = index.min(node.children.len());
                node.children.insert(index, child);
                node.blending_children += u32::from(blends);
            }
            LayerOp::Detach { parent, child } => {
                assert_eq!(
                    self.layer(child).parent,
                    Some(parent),
                    "detaching from the wrong parent"
                );
                self.detach(child);
            }
        }
    }

    fn assert_attachment(&self, parent: LayerId, child: LayerId) {
        assert_ne!(child, self.root, "the root layer cannot be attached");
        let mut cursor = Some(parent);
        while let Some(id) = cursor {
            assert_ne!(id, child, "layer attachment would create a cycle");
            cursor = self.layer(id).parent;
        }
    }

    fn detach(&mut self, child: LayerId) {
        if let Some(parent) = self.node_mut(child).parent.take() {
            self.stamp(parent, true);
            let blends = self.layer(child).blend != BlendMode::Normal;
            let parent_node = self.node_mut(parent);
            parent_node.children.retain(|c| *c != child);
            parent_node.blending_children -= u32::from(blends);
        }
    }

    fn node_mut(&mut self, id: LayerId) -> &mut LayerNode {
        self.nodes
            .get_mut(&id.raw())
            .unwrap_or_else(|| panic!("layer {} is not in the tree", id.raw()))
    }

    /// Samples every animation track at `time`, updating the layers'
    /// sampled properties. `scale` snaps the scroll offset to the
    /// device-pixel grid and classifies slow decays.
    pub fn sample(&mut self, time: Instant, scale: f64) -> Sampling {
        let mut stepped = false;
        let mut fast = false;
        let mut slow = false;
        for node in self.nodes.values_mut() {
            let mut node_fast = false;
            // Opacity is outer and scroll inner: any running track steps.
            let mut outer_changed = node.opacity_track.is_some();
            let inner_changed = node.scroll_track.is_some();
            let mut transform_changed = false;
            if let Some(track) = &mut node.transform_track {
                stepped = true;
                transform_changed = true;
                let (pos, _vel, done) = track.sample(time);
                let matrix = if done {
                    track.target
                } else {
                    Affine::from_lanes(pos)
                };
                if let Some(components) = &mut node.components {
                    components.base = matrix;
                } else {
                    node.transform = matrix;
                }
                if done {
                    node.transform_track = None;
                }
            }
            if let Some(components) = &mut node.components {
                let (component_step, running) = components.sample(time);
                stepped |= component_step;
                node_fast |= running;
                if transform_changed || component_step {
                    node.transform = components.matrix();
                }
                outer_changed |= component_step;
            }
            outer_changed |= transform_changed;
            if let Some(track) = &mut node.opacity_track {
                stepped = true;
                let (pos, _vel, done) = track.sample(time);
                node.opacity = f32::from_lanes(pos);
                if done {
                    node.opacity = track.target;
                    node.opacity_track = None;
                }
            }
            if let Some(track) = &mut node.scroll_track {
                stepped = true;
                let (pos, vel, done) = track.sample(time);
                // Rubber-band handoff: the moment the offset leaves the
                // bounds, the remaining motion becomes a critically damped
                // spring back to the nearest point of the bounds, so the
                // overshoot and the return are one continuous motion.
                if let Animation::Decay(decay) = track.animation
                    && let Some(bounds) = decay.rubber_band
                {
                    let offset = Vec2::from_lanes(pos);
                    // "Outside the bounds" = clamping changes the offset:
                    // `Rect::contains` is half-open and never holds on a
                    // degenerate edge (a zero-width bounds rect), while a
                    // scroll axis exactly on the bound must stay in.
                    let clamped = clamp_to_rect(offset, bounds);
                    if clamped != offset {
                        *track = Track {
                            from: pos,
                            velocity: vel,
                            target: clamped,
                            animation: Animation::Spring(rubber_band_spring()),
                            start: Some(time),
                            last: track.last,
                        };
                    }
                }
                // A decay keeps where it stopped; a settled spring (including
                // the rubber-band handoff) reports its target exactly.
                node.scroll_offset = snap(Vec2::from_lanes(pos), scale);
                if done {
                    node.scroll_track = None;
                }
            }
            node.restamp(&mut self.clock, outer_changed, inner_changed);
            node.animation_rate = node.classify_rate(scale, node_fast);
            fast |= node.animation_rate == Some(RATE_FAST);
            slow |= node.animation_rate == Some(RATE_SLOW);
        }
        let (projective_step, projective_running) = self.sample_projective(time);
        stepped |= projective_step;
        fast |= projective_running;
        let rate = if fast {
            Some(RATE_FAST)
        } else if slow {
            Some(RATE_SLOW)
        } else {
            None
        };
        Sampling { stepped, rate }
    }
}

/// Applies a property change to a track: an animated change retargets from
/// the old track's last sampled position and velocity (its start state when
/// never sampled), so a spring retargeted mid-flight is continuous in
/// position and velocity and a curve restarts from its current value. A
/// change without an animation snaps and drops the track.
fn set_prop<T: Animatable>(track: &mut Option<Track<T>>, value: &mut T, prop: &Prop<T>) {
    match prop.animation {
        None => {
            *value = prop.target;
            *track = None;
        }
        Some(animation @ Animation::Decay(decay)) => {
            // A decay is a fling: it starts AT the committed value with its
            // own velocity; the previous track is irrelevant.
            let mut velocity = T::Lanes::zero();
            for (i, v) in [decay.velocity.x, decay.velocity.y]
                .into_iter()
                .take(T::Lanes::N)
                .enumerate()
            {
                velocity.set(i, v);
            }
            *track = Some(Track::new(
                prop.target.into_lanes(),
                velocity,
                prop.target,
                animation,
                prop.start,
            ));
        }
        Some(animation) => {
            let (from, velocity, last) = track.as_ref().map_or_else(
                || ((*value).into_lanes(), T::Lanes::zero(), None),
                |old| {
                    prop.start.map_or_else(
                        || {
                            (
                                old.last.map_or(old.from, |(_, pos, _)| pos),
                                old.last.map_or(old.velocity, |(_, _, vel)| vel),
                                old.last,
                            )
                        },
                        // A host-given start anchors the new track to `t`, so
                        // it continues from the previous track evaluated at
                        // `t`, not from its last sample — the sampler may not
                        // have drawn a frame since the host's clock issued the
                        // change.
                        |start| {
                            let (pos, vel) = old.value_at(start);
                            (pos, vel, None)
                        },
                    )
                },
            );
            let mut next = Track::new(from, velocity, prop.target, animation, prop.start);
            next.last = last;
            *track = Some(next);
        }
    }
}

/// Snaps an offset to the device-pixel grid: `round(offset · scale) /
/// scale`. The track itself is not snapped, so a slow decay still settles
/// smoothly.
fn snap(offset: Vec2, scale: f64) -> Vec2 {
    if scale <= 0.0 {
        return offset;
    }
    Vec2::new(
        (offset.x * scale).round() / scale,
        (offset.y * scale).round() / scale,
    )
}

/// Places a device transform's translation on the ¼-pixel grid, the
/// placement of content under an animating layer.
#[must_use]
#[expect(
    clippy::many_single_char_names,
    reason = "a/b/c/d are the conventional affine matrix coefficient names"
)]
pub const fn snap_animating(t: Affine) -> Affine {
    let [a, b, c, d, e, f] = t.as_coeffs();
    Affine::new([a, b, c, d, (e * 4.0).round() / 4.0, (f * 4.0).round() / 4.0])
}

#[cfg(test)]
mod hierarchy_tests {
    use super::*;
    use crate::display_list::{Operand, Picture, SlotUpdate};
    use crate::style::Group;
    use crate::{Curve, Decay, Spring};
    use crate::{Draw, WorkingColor};
    use kurbo::Rect;
    use std::time::Duration;

    fn tree() -> SurfaceTree {
        let mut tree = SurfaceTree::new();
        for id in 1..=3 {
            tree.apply(LayerOp::Create(LayerId::new(id)));
        }
        tree.apply(LayerOp::Push {
            parent: tree.root(),
            child: LayerId::new(1),
        });
        tree.apply(LayerOp::Push {
            parent: LayerId::new(1),
            child: LayerId::new(2),
        });
        tree.apply(LayerOp::Push {
            parent: tree.root(),
            child: LayerId::new(3),
        });
        tree
    }

    #[test]
    #[should_panic(expected = "layer attachment would create a cycle")]
    fn attaching_a_layer_under_its_subtree_is_a_cycle() {
        let mut tree = tree();
        tree.apply(LayerOp::Push {
            parent: LayerId::new(2),
            child: LayerId::new(1),
        });
    }

    #[test]
    #[should_panic(expected = "layer attachment would create a cycle")]
    fn inserting_a_layer_under_itself_is_a_cycle() {
        let mut tree = tree();
        tree.apply(LayerOp::Insert {
            parent: LayerId::new(1),
            child: LayerId::new(1),
            index: 0,
        });
    }

    #[test]
    #[should_panic(expected = "the root layer cannot be attached")]
    fn attaching_the_root_is_rejected() {
        let mut tree = tree();
        tree.apply(LayerOp::Push {
            parent: LayerId::new(2),
            child: tree.root(),
        });
    }

    #[test]
    fn animating_reports_a_running_track() {
        let mut tree = SurfaceTree::new();
        let start = Instant::now();
        tree.apply(LayerOp::Transform(
            tree.root(),
            Prop {
                target: Affine::translate((10., 0.)),
                animation: Some(Curve::linear(Duration::from_secs(1)).into()),
                start: None,
            },
        ));
        tree.sample(start, 1.0);
        tree.sample(start + Duration::from_millis(500), 1.0);
        assert!(tree.layer(tree.root()).animating());
        tree.sample(start + Duration::from_secs(1), 1.0);
        assert!(!tree.layer(tree.root()).animating());
    }

    #[test]
    fn a_running_scroll_decay_counts_as_animating() {
        let mut tree = SurfaceTree::new();
        let start = Instant::now();
        tree.apply(LayerOp::ScrollOffset(
            tree.root(),
            Prop {
                target: Vec2::ZERO,
                animation: Some(Decay::new(Vec2::new(0., 400.)).into()),
                start: None,
            },
        ));
        tree.sample(start + Duration::from_millis(100), 1.0);
        assert!(tree.layer(tree.root()).animating());
        tree.sample(start + Duration::from_secs(10), 1.0);
        assert!(!tree.layer(tree.root()).animating());
    }

    #[test]
    fn opacity_animation_does_not_count_as_animating() {
        let mut tree = SurfaceTree::new();
        let start = Instant::now();
        tree.apply(LayerOp::Opacity(
            tree.root(),
            Prop {
                target: 0.5,
                animation: Some(Curve::linear(Duration::from_secs(1)).into()),
                start: None,
            },
        ));
        tree.sample(start + Duration::from_millis(500), 1.0);
        assert!(!tree.layer(tree.root()).animating());
    }

    #[test]
    fn animation_description_rejects_incomplete_and_projective_tracks() {
        let mut tree = SurfaceTree::new();
        let root = tree.root();
        tree.apply(LayerOp::Opacity(
            root,
            Prop {
                target: 0.5,
                animation: Some(Curve::linear(Duration::from_secs(1)).into()),
                start: None,
            },
        ));
        assert!(tree.layer(root).animations().is_none());
        tree.sample(Instant::now(), 1.0);
        let tracks = tree.layer(root).animations().expect("sampled opacity");
        assert!(tracks.transform.is_none());
        assert!(tracks.opacity.is_some());
        tree.apply(LayerOp::Depth(
            root,
            Prop {
                target: 2.0,
                animation: None,
                start: None,
            },
        ));
        assert!(tree.layer(root).animations().is_none());
        tree.apply(LayerOp::ClearProjection(root));
        assert!(tree.layer(root).animations().is_some());
    }

    #[test]
    fn a_track_given_a_start_samples_identically_to_the_shared_evaluation() {
        let mut tree = SurfaceTree::new();
        let root = tree.root();
        let start = Instant::now() + Duration::from_millis(10);
        tree.apply(LayerOp::Transform(
            root,
            Prop {
                target: Affine::translate((100., 20.)),
                animation: Some(Curve::linear(Duration::from_millis(500)).into()),
                start: Some(start),
            },
        ));
        // The description is complete before the first sample: the track's
        // start was given, not taken from a frame.
        let track = tree
            .layer(root)
            .animations()
            .and_then(|a| a.transform)
            .expect("a track with a start is describable unsampled");
        assert_eq!(track.start, start);
        // Before `start` and on every frame to completion, the sampled
        // value is the shared evaluation at that instant.
        let t0 = start.checked_sub(Duration::from_millis(10)).unwrap();
        for step in 0..=11 {
            let t = t0 + Duration::from_millis(step * 50);
            tree.sample(t, 1.0);
            let (position, _, done) = track.sample(t);
            assert_eq!(tree.layer(root).transform, position, "at {t:?}");
            assert_eq!(done, step == 11, "a curve is done at start + duration");
        }
    }

    #[test]
    fn a_retarget_with_a_start_continues_from_the_previous_track_at_that_instant() {
        let mut tree = SurfaceTree::new();
        let root = tree.root();
        let start = Instant::now();
        tree.apply(LayerOp::Transform(
            root,
            Prop {
                target: Affine::translate((100., 0.)),
                animation: Some(Curve::linear(Duration::from_secs(1)).into()),
                start: Some(start),
            },
        ));
        let first = tree
            .layer(root)
            .animations()
            .and_then(|a| a.transform)
            .expect("running track");
        // Sampled once, then retargeted at a later instant on the host's
        // clock: the new track's start state is the old track's evaluation
        // there, not its last sample.
        tree.sample(start + Duration::from_millis(100), 1.0);
        let retarget_at = start + Duration::from_millis(250);
        tree.apply(LayerOp::Transform(
            root,
            Prop {
                target: Affine::translate((50., 0.)),
                animation: Some(Curve::linear(Duration::from_millis(500)).into()),
                start: Some(retarget_at),
            },
        ));
        let track = tree
            .layer(root)
            .animations()
            .and_then(|a| a.transform)
            .expect("retargeted track");
        let (position, velocity, _) = first.sample(retarget_at);
        assert_eq!(track.from, position);
        // The retarget took its velocity from the same `eval_lanes` call on
        // the same track at the same instant: bit-identical.
        assert_eq!(track.velocity.map(f64::to_bits), velocity.map(f64::to_bits));
        assert_eq!(track.start, retarget_at);
        // And the new track keeps sampling identically to its description.
        let t = retarget_at + Duration::from_millis(200);
        tree.sample(t, 1.0);
        assert_eq!(tree.layer(root).transform, track.sample(t).0);
    }

    #[test]
    fn an_unstarted_old_track_evaluates_at_the_retarget_instant_as_its_start() {
        let mut tree = SurfaceTree::new();
        let root = tree.root();
        let start = Instant::now();
        tree.apply(LayerOp::Transform(
            root,
            Prop {
                target: Affine::translate((100., 0.)),
                animation: Some(Curve::linear(Duration::from_secs(1)).into()),
                start: None,
            },
        ));
        // The first track was never sampled: a retarget with a start
        // continues from where it would have begun.
        let retarget_at = start + Duration::from_millis(250);
        tree.apply(LayerOp::Transform(
            root,
            Prop {
                target: Affine::translate((50., 0.)),
                animation: Some(Spring::smooth().into()),
                start: Some(retarget_at),
            },
        ));
        let track = tree
            .layer(root)
            .animations()
            .and_then(|a| a.transform)
            .expect("retargeted track");
        assert_eq!(track.from, Affine::IDENTITY);
        // An unstarted curve evaluates at its first instant, where
        // `curve_slope` returns exactly zero; scaling the non-negative Δ by
        // it leaves +0.0 in every lane.
        assert_eq!(
            track.velocity.map(f64::to_bits),
            [0.0_f64; 6].map(f64::to_bits)
        );
        assert_eq!(track.start, retarget_at);
    }

    #[test]
    fn ownership_filter_keeps_the_sampled_slow_rate() {
        let mut tree = SurfaceTree::new();
        let root = tree.root();
        let owned = LayerId::new(1);
        let slow = LayerId::new(2);
        for child in [owned, slow] {
            tree.apply(LayerOp::Create(child));
            tree.apply(LayerOp::Push {
                parent: root,
                child,
            });
        }
        // Decay is the only slow class, and it is legal only on scroll offset.
        tree.apply(LayerOp::ScrollOffset(
            slow,
            Prop {
                target: Vec2::ZERO,
                animation: Some(Decay::new(Vec2::new(0., 10.)).into()),
                start: None,
            },
        ));
        tree.apply(LayerOp::Transform(
            owned,
            Prop {
                target: Affine::translate((100., 0.)),
                animation: Some(Curve::linear(std::time::Duration::from_secs(1)).into()),
                start: None,
            },
        ));
        let sampled = tree.sample(Instant::now(), 1.0);
        assert_eq!(sampled.rate, Some(RATE_FAST));
        assert_eq!(tree.animation_rate(|layer| layer == owned), Some(RATE_SLOW));
        assert_eq!(tree.animation_rate(|_| false), sampled.rate);
        assert_eq!(tree.animation_rate(|_| true), None);
    }

    #[test]
    fn detach_and_remove_preserve_parent_links() {
        let mut tree = tree();
        tree.apply(LayerOp::Push {
            parent: LayerId::new(3),
            child: LayerId::new(1),
        });
        assert_eq!(tree.layer(tree.root()).children, [LayerId::new(3)]);
        assert_eq!(tree.layer(LayerId::new(1)).parent, Some(LayerId::new(3)));
        // Removing a layer removes only it: its children stay in the
        // tree, detached, until their own `Remove` or re-attachment.
        tree.remove(LayerId::new(1));
        assert!(tree.layers().all(|(id, _)| id != LayerId::new(1)));
        assert_eq!(tree.layer(LayerId::new(3)).children, []);
        assert!(tree.layer(LayerId::new(2)).parent.is_none());
        tree.remove(LayerId::new(2));
    }

    #[test]
    fn attached_child_blend_changes_are_counted() {
        let mut tree = SurfaceTree::new();
        let child = LayerId::new(1);
        tree.apply(LayerOp::Create(child));
        tree.apply(LayerOp::Push {
            parent: tree.root(),
            child,
        });

        assert!(!tree.layer(tree.root()).blends_within());
        tree.apply(LayerOp::Blend(child, BlendMode::DestOut));
        assert!(tree.layer(tree.root()).blends_within());
        tree.apply(LayerOp::Blend(child, BlendMode::Normal));
        assert!(!tree.layer(tree.root()).blends_within());
    }

    #[test]
    fn blending_child_count_tracks_push_insert_detach_and_remove() {
        let mut tree = SurfaceTree::new();
        let first_parent = LayerId::new(1);
        let second_parent = LayerId::new(2);
        let first_child = LayerId::new(3);
        let second_child = LayerId::new(4);
        for id in [first_parent, second_parent, first_child, second_child] {
            tree.apply(LayerOp::Create(id));
        }
        for parent in [first_parent, second_parent] {
            tree.apply(LayerOp::Push {
                parent: tree.root(),
                child: parent,
            });
        }
        tree.apply(LayerOp::Blend(first_child, BlendMode::DestOut));
        tree.apply(LayerOp::Push {
            parent: first_parent,
            child: first_child,
        });
        assert!(tree.layer(first_parent).blends_within());

        tree.apply(LayerOp::Blend(second_child, BlendMode::Multiply));
        tree.apply(LayerOp::Insert {
            parent: first_parent,
            index: 0,
            child: second_child,
        });
        tree.apply(LayerOp::Detach {
            parent: first_parent,
            child: first_child,
        });
        assert!(tree.layer(first_parent).blends_within());

        tree.remove(second_child);
        assert!(!tree.layer(first_parent).blends_within());
    }

    /// An installed frame's declared alpha decides `content_translucent`:
    /// an opaque frame keeps the layer's coverage provable — a second
    /// opaque video above a promoted one does not block it (#90) — while
    /// a producer that declares no opacity leaves the layer translucent.
    #[test]
    fn note_installed_counts_the_frames_declared_opacity() {
        let mut tree = tree();
        tree.note_installed(LayerId::new(1), true);
        assert!(!tree.layer(LayerId::new(1)).content_translucent());
        tree.note_installed(LayerId::new(2), false);
        assert!(tree.layer(LayerId::new(2)).content_translucent());
        // Recorded content replaces the install: its own analysis takes over.
        let picture = Picture::record(|c| {
            c.fill(
                Rect::new(0., 0., 8., 8.),
                WorkingColor::new([0.5, 0.5, 0.5, 0.5]),
            );
        });
        tree.note_content(LayerId::new(1), Some(&ContentOp::Picture(picture)));
        assert!(tree.layer(LayerId::new(1)).content_translucent());
        tree.note_content(LayerId::new(2), None);
        assert!(!tree.layer(LayerId::new(2)).content_translucent());
    }

    #[test]
    fn repushing_a_blending_child_updates_both_parents() {
        let mut tree = SurfaceTree::new();
        let first_parent = LayerId::new(1);
        let second_parent = LayerId::new(2);
        let child = LayerId::new(3);
        for id in [first_parent, second_parent, child] {
            tree.apply(LayerOp::Create(id));
        }
        for parent in [first_parent, second_parent] {
            tree.apply(LayerOp::Push {
                parent: tree.root(),
                child: parent,
            });
        }
        tree.apply(LayerOp::Blend(child, BlendMode::DestOut));
        tree.apply(LayerOp::Push {
            parent: first_parent,
            child,
        });
        tree.apply(LayerOp::Push {
            parent: second_parent,
            child,
        });

        assert!(!tree.layer(first_parent).blends_within());
        assert!(tree.layer(second_parent).blends_within());
    }

    #[test]
    fn blend_set_before_attachment_is_counted() {
        let mut tree = SurfaceTree::new();
        let parent = LayerId::new(1);
        let child = LayerId::new(2);
        tree.apply(LayerOp::Create(parent));
        tree.apply(LayerOp::Create(child));
        tree.apply(LayerOp::Push {
            parent: tree.root(),
            child: parent,
        });
        tree.apply(LayerOp::Blend(child, BlendMode::DestOut));
        assert!(!tree.layer(parent).blends_within());

        tree.apply(LayerOp::Push { parent, child });
        assert!(tree.layer(parent).blends_within());
    }

    #[test]
    fn grandchild_blend_does_not_count_on_grandparent() {
        let mut tree = SurfaceTree::new();
        let grandparent = LayerId::new(1);
        let parent = LayerId::new(2);
        let grandchild = LayerId::new(3);
        for id in [grandparent, parent, grandchild] {
            tree.apply(LayerOp::Create(id));
        }
        tree.apply(LayerOp::Push {
            parent: tree.root(),
            child: grandparent,
        });
        tree.apply(LayerOp::Push {
            parent: grandparent,
            child: parent,
        });
        tree.apply(LayerOp::Push {
            parent,
            child: grandchild,
        });
        tree.apply(LayerOp::Blend(grandchild, BlendMode::DestOut));

        assert!(tree.layer(parent).blends_within());
        assert!(!tree.layer(grandparent).blends_within());
    }

    #[test]
    fn content_blends_track_replace_update_and_clear() {
        let mut tree = SurfaceTree::new();
        let layer = LayerId::new(1);
        tree.apply(LayerOp::Create(layer));
        let picture = Picture::record(|recorder| {
            recorder.group(Group::new().blend(BlendMode::DestOut), |recorder| {
                recorder.fill(Rect::new(0.0, 0.0, 1.0, 1.0), WorkingColor::WHITE);
            });
        });

        tree.note_content(layer, Some(&ContentOp::Replace(picture.clone())));
        assert!(tree.layer(layer).blends_within());
        tree.note_content(layer, Some(&ContentOp::Picture(picture)));
        assert!(tree.layer(layer).blends_within());

        tree.note_content(
            layer,
            Some(&ContentOp::Update(vec![SlotUpdate {
                command: 0,
                value: Operand::Group(Group::new()),
            }])),
        );
        assert!(tree.layer(layer).blends_within());

        tree.note_content(layer, None);
        assert!(!tree.layer(layer).blends_within());
    }

    #[test]
    fn owned_motion_retargets_at_the_current_presentation_time() {
        let mut tree = SurfaceTree::new();
        let layer = tree.root();
        let start = Instant::now();
        tree.apply(LayerOp::Opacity(
            layer,
            Prop {
                target: 0.0,
                animation: Some(Curve::linear(Duration::from_secs(2)).into()),
                start: None,
            },
        ));
        tree.sample(start, 1.0);
        assert_eq!(tree.animation_rate(|_| true), None);
        assert_eq!(tree.animation_rate(|_| false), Some(RATE_FAST));
        tree.sample_owned(start + Duration::from_secs(1), |_| true);
        tree.apply(LayerOp::Opacity(
            layer,
            Prop {
                target: 0.8,
                animation: Some(Spring::smooth().into()),
                start: None,
            },
        ));
        tree.sample(start + Duration::from_secs(1), 1.0);
        let track = tree
            .layer(layer)
            .animations()
            .expect("describable tracks")
            .opacity
            .expect("running spring");
        assert!((track.from - 0.5).abs() < 1e-6);
        assert!((track.velocity[0] + 0.5).abs() < 1e-6);
        assert_eq!(track.start, start + Duration::from_secs(1));
    }

    #[test]
    fn plane_composition_version_excludes_only_the_promoted_outer_state() {
        let mut tree = SurfaceTree::new();
        let layer = LayerId::new(1);
        tree.apply(LayerOp::Create(layer));
        tree.apply(LayerOp::Push {
            parent: tree.root(),
            child: layer,
        });
        let stamp = tree.composition_stamp(|id| id == layer);
        tree.apply(LayerOp::Transform(
            layer,
            Prop {
                target: Affine::translate((37., -12.)),
                animation: None,
                start: None,
            },
        ));
        assert_eq!(stamp, tree.composition_stamp(|id| id == layer));
        tree.apply(LayerOp::Clip(
            layer,
            Some(ShapeData::Rect(Rect::new(0., 0., 20., 20.))),
        ));
        assert_ne!(stamp, tree.composition_stamp(|id| id == layer));
    }
}
