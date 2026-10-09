//! The frame encoder: a [`SurfaceTree`] mirror whose layers are
//! `RenderNode`s.
//!
//! Each layer owns an outer node carrying its transform, alpha, clip and
//! composite as node properties; a layer that has scrolled gains an inner
//! node translated by the negative scroll offset, so scrolling moves node
//! properties and never re-records. A node's display list is the layer's
//! content then a `DrawNode` per child in paint order. Property changes
//! cross as node ops; content, child-list and non-property clip or
//! transform changes re-record the affected lists.
//!
//! Node local space is the layer's space shifted to the integer origin of
//! its ink, so a node's bounds cover what it draws: the fractional part
//! of a translation stays a translation, and the pivot is the layer's
//! origin.

use std::collections::BTreeMap;

use waterui_graphics::draw::color::WorkingColor;
use waterui_graphics::draw::kurbo::{Affine, Rect, Vec2};
use waterui_graphics::draw::tree::Sampling;
use waterui_graphics::draw::{
    BlendMode, ChangeSet, ContentOp, LayerId, LayerNode, LayerOp, Op as TreeOp, Picture, Realize,
    ShapeData, SurfaceTree,
};
use waterui_graphics::draw::{ResourceId, TextLayoutId};

use super::buffer::{CommandBuffer, Field};
use super::geometry::{
    Decomposed, Tilted, decompose, floor_i32, homography_matrix, intersect, is_integral,
    project_rect, tilt, union,
};
use super::lower::{
    Caches, GroupNodes, Lower, Owned, Pools, Scratch, blend_code, bounds, clear_color, content_ink,
    extent, set_position,
};
use super::protocol::{Op, clip as clip_kind, host};
use super::resources::SharedRegistry;
use super::text::TextLayoutIds;
use super::{HwuiError, HwuiTarget};

/// One entry of the host's top-level draw order, bottom first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostEntry {
    /// A layer's node run.
    Layer(LayerId),
    /// An embedded platform view, by placement id.
    PlatformView(u64),
}

/// A committed change set's buffers, for `Shared::recycle`.
#[derive(Debug)]
pub struct Recycle {
    /// The emptied op vector.
    pub ops: Vec<TreeOp<HwuiTarget>>,
    /// The pictures this commit replaced.
    pub recycled: Vec<(LayerId, Picture)>,
}

/// One encoded frame.
#[derive(Debug)]
pub struct Frame<'a> {
    /// The command buffer, header included.
    pub words: &'a [u32],
    /// What sampling the tree's animations reported.
    pub sampling: Sampling,
    /// Whether recorded-content operands still animate.
    pub content_animating: bool,
}

/// How a layer's node sits in its parent's list.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Placement {
    /// Node properties carry the transform.
    Node(Decomposed),
    /// Node properties carry the transform through the node's camera.
    Camera(Tilted),
    /// The parent concatenates this plane homography before `DrawNode`.
    Concat([[f64; 3]; 3]),
}

/// How a layer's clip lowers.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ClipMode {
    None,
    /// A node clip: kind, integer bounds in layer space, radius.
    Node(u32, [i32; 4], f32),
    /// Recorded into the list.
    List,
}

#[derive(Debug)]
enum Content {
    None,
    Live(Picture),
    Shared(Picture),
}

impl Content {
    fn draws(&self, resource: ResourceId) -> bool {
        match self {
            Self::None => false,
            Self::Live(picture) | Self::Shared(picture) => {
                picture.display_list().references(resource)
            }
        }
    }
}

#[derive(Debug, Default, PartialEq)]
struct Sent {
    position: Option<[i32; 5]>,
    transform: Option<[f32; 10]>,
    inner_position: Option<[i32; 4]>,
    inner_transform: Option<[f32; 2]>,
    alpha: Option<f32>,
    composite: Option<(u32, bool)>,
    clip: Option<(u32, [i32; 4], f32)>,
}

#[derive(Debug)]
struct LayerState {
    outer: u32,
    inner: Option<u32>,
    content: Content,
    content_ink: Option<Rect>,
    parent: Option<LayerId>,
    children: Vec<LayerId>,
    clip: Option<ShapeData>,
    clip_mode: ClipMode,
    scroll: Vec2,
    placement: Placement,
    ink: Option<Rect>,
    inner_ink: Option<Rect>,
    origin: [i32; 2],
    size: [i32; 2],
    inner_origin: [i32; 2],
    inner_size: [i32; 2],
    recorded: bool,
    record_dirty: bool,
    ink_dirty: bool,
    /// The tree's `(inner, outer)` stamps when this layer was last
    /// diffed; `None` until its first frame.
    encoded: Option<(u64, u64)>,
    owned: Owned,
    sent: Sent,
}

impl LayerState {
    fn new(outer: u32) -> Self {
        Self {
            outer,
            inner: None,
            content: Content::None,
            content_ink: None,
            parent: None,
            children: Vec::new(),
            clip: None,
            clip_mode: ClipMode::None,
            scroll: Vec2::ZERO,
            placement: Placement::Node(IDENTITY_PARTS),
            ink: None,
            inner_ink: None,
            origin: [0, 0],
            size: [0, 0],
            inner_origin: [0, 0],
            inner_size: [0, 0],
            recorded: false,
            record_dirty: true,
            ink_dirty: true,
            encoded: None,
            owned: Owned::default(),
            sent: Sent {
                alpha: Some(1.0),
                composite: Some((blend_code(BlendMode::Normal), false)),
                clip: Some((clip_kind::NONE, [0; 4], 0.0)),
                ..Sent::default()
            },
        }
    }
}

/// A layer whose tree stamps moved since it was last diffed.
#[derive(Debug, Clone, Copy)]
struct Changed {
    raw: u64,
    /// The tree's `(inner, outer)` stamps now.
    stamps: (u64, u64),
    /// What the layer draws in its own space changed.
    inner: bool,
    /// How the layer composes into its parent changed.
    outer: bool,
}

/// The layers one frame visits, kept across frames for their storage.
///
/// A frame diffs only the layers whose tree stamps moved past the ones it
/// last encoded. The tree's clock stamps sampled animation steps as well
/// as committed ops, so a running animation needs no separate path.
#[derive(Debug, Default)]
struct Pending {
    changed: Vec<Changed>,
    /// Layers whose node properties or display lists may have changed.
    touched: Vec<u64>,
    /// Parents whose isolation may follow a child's blend change.
    recomposite: Vec<u64>,
    reparent: Vec<(LayerId, Vec<LayerId>)>,
    moved_parents: Vec<LayerId>,
    ink_dirty: Vec<LayerId>,
}

impl Pending {
    fn reset(&mut self) {
        self.changed.clear();
        self.touched.clear();
        self.recomposite.clear();
        self.reparent.clear();
        self.moved_parents.clear();
        self.ink_dirty.clear();
    }
}

const IDENTITY_PARTS: Decomposed = Decomposed {
    translation: (0.0, 0.0),
    scale: (1.0, 1.0),
    rotation: 0.0,
};

/// The HWUI frame encoder.
#[derive(Debug)]
pub struct Encoder {
    tree: SurfaceTree,
    buffer: CommandBuffer,
    open: bool,
    pools: Pools,
    caches: Caches,
    registry: SharedRegistry,
    text_layouts: TextLayoutIds,
    scratch: Scratch,
    layers: BTreeMap<u64, LayerState>,
    pending: Pending,
    host: Vec<HostEntry>,
    host_dirty: bool,
    clear: Option<WorkingColor>,
    content_animating: bool,
    api_level: u32,
}

impl Encoder {
    /// An encoder for a device at `api_level`, mirroring an empty tree.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Encoding`] if the root node cannot be created.
    pub fn new(api_level: u32) -> Result<Self, HwuiError> {
        let tree = SurfaceTree::new();
        let root = tree.root();
        let mut encoder = Self {
            tree,
            buffer: CommandBuffer::new(),
            open: false,
            pools: Pools::new(),
            caches: Caches::default(),
            registry: SharedRegistry::new(),
            text_layouts: TextLayoutIds::new(),
            scratch: Scratch::default(),
            layers: BTreeMap::new(),
            pending: Pending::default(),
            host: vec![HostEntry::Layer(root)],
            host_dirty: true,
            clear: None,
            content_animating: false,
            api_level,
        };
        encoder.create_layer(root)?;
        Ok(encoder)
    }

    /// The mirrored tree.
    #[must_use]
    pub const fn tree(&self) -> &SurfaceTree {
        &self.tree
    }

    /// The resource tables registrations go through, shared with the
    /// resource owner ([`HwuiResources`](super::HwuiResources)).
    #[must_use]
    pub(crate) const fn registry(&self) -> &SharedRegistry {
        &self.registry
    }

    /// The platform text layout ids, shared with every layout the text
    /// engine shapes; a dropped layout's release joins the next frame.
    #[must_use]
    pub const fn text_layouts(&self) -> &TextLayoutIds {
        &self.text_layouts
    }

    /// The last clear colour committed, as a `ColorLong`.
    #[must_use]
    pub fn clear_color(&self) -> Option<u64> {
        self.clear.map(clear_color)
    }

    /// The outer node of `layer`.
    #[must_use]
    pub fn node(&self, layer: LayerId) -> Option<u32> {
        self.layers.get(&layer.raw()).map(|state| state.outer)
    }

    /// Takes the contract-test event log of the frames encoded so far.
    #[cfg(test)]
    pub fn take_log(&mut self) -> Vec<String> {
        self.buffer.take_log()
    }

    /// Sets the host's top-level draw order.
    pub fn set_host_order(&mut self, order: &[HostEntry]) {
        if self.host != order {
            self.host.clear();
            self.host.extend_from_slice(order);
            self.host_dirty = true;
        }
    }

    fn ensure_open(&mut self) {
        if !self.open {
            self.buffer.begin_frame();
            self.open = true;
        }
    }

    const fn lower(&mut self, layer: u64) -> Lower<'_> {
        Lower {
            buffer: &mut self.buffer,
            pools: &mut self.pools,
            caches: &mut self.caches,
            registry: &self.registry,
            text_layouts: &self.text_layouts,
            scratch: &mut self.scratch,
            api_level: self.api_level,
            layer,
        }
    }

    fn create_layer(&mut self, id: LayerId) -> Result<(), HwuiError> {
        self.ensure_open();
        let outer = self.lower(id.raw()).create_node()?;
        self.layers.insert(id.raw(), LayerState::new(outer));
        Ok(())
    }

    fn state(&mut self, id: LayerId) -> Result<&mut LayerState, HwuiError> {
        self.layers
            .get_mut(&id.raw())
            .ok_or_else(|| HwuiError::Encoding {
                op: "commit",
                reason: format!("layer {} is not mirrored", id.raw()),
            })
    }

    /// Applies a committed change set to the mirror, writing node creates
    /// and releases into the open frame. Hand the returned buffers to
    /// `Shared::recycle`.
    ///
    /// # Errors
    ///
    /// A [`HwuiError`] when the change set names what this target cannot
    /// mirror.
    pub fn commit(&mut self, mut changes: ChangeSet<HwuiTarget>) -> Result<Recycle, HwuiError> {
        self.ensure_open();
        if let Some(clear) = changes.clear {
            self.clear = Some(clear);
        }
        self.content_animating = changes.animating;
        let mut recycled = std::mem::take(&mut changes.recycled);
        for op in changes.ops.drain(..) {
            let created = match &op {
                TreeOp::Layer(LayerOp::Create(id)) => Some(*id),
                _ => None,
            };
            match self.tree.apply_op(op) {
                Realize::Applied => {
                    if let Some(id) = created {
                        self.create_layer(id)?;
                    }
                }
                Realize::Remove(id) => self.remove_layer(id)?,
                Realize::Content(id, content) => self.set_content(id, content, &mut recycled)?,
                #[expect(
                    unreachable_code,
                    reason = "`Install` is `Infallible` on this target: the arm is uninhabited"
                )]
                Realize::Install(_, install) => match install.into_inner() {},
            }
        }
        Ok(Recycle {
            ops: changes.ops,
            recycled,
        })
    }

    fn remove_layer(&mut self, id: LayerId) -> Result<(), HwuiError> {
        let state = self
            .layers
            .remove(&id.raw())
            .ok_or_else(|| HwuiError::Encoding {
                op: "commit",
                reason: format!("removed layer {} is not mirrored", id.raw()),
            })?;
        for child in &state.children {
            if let Some(child) = self.layers.get_mut(&child.raw())
                && child.parent == Some(id)
            {
                child.parent = None;
            }
        }
        if let Some(parent) = state.parent
            && let Some(parent) = self.layers.get_mut(&parent.raw())
        {
            parent.ink_dirty = true;
        }
        let mut lower = self.lower(id.raw());
        lower.release(state.owned)?;
        lower.release_node(state.outer)?;
        if let Some(inner) = state.inner {
            lower.release_node(inner)?;
        }
        Ok(())
    }

    fn set_content(
        &mut self,
        id: LayerId,
        content: Option<ContentOp>,
        recycled: &mut Vec<(LayerId, Picture)>,
    ) -> Result<(), HwuiError> {
        let text_layouts = self.text_layouts.clone();
        let registry = self.registry.clone();
        let state = self.state(id)?;
        let next = match content {
            None => Content::None,
            Some(ContentOp::Replace(picture)) => Content::Live(picture),
            Some(ContentOp::Picture(picture)) => Content::Shared(picture),
            Some(ContentOp::Update(updates)) => {
                let Content::Live(picture) = &mut state.content else {
                    return Err(HwuiError::Encoding {
                        op: "commit",
                        reason: format!("layer {} updates slots of no live content", id.raw()),
                    });
                };
                picture.apply(updates);
                Content::Live(picture.clone())
            }
        };
        let previous = std::mem::replace(&mut state.content, next);
        if let (Content::Live(old), Content::Live(new)) = (&previous, &state.content)
            && std::ptr::eq(old.display_list(), new.display_list())
        {
            // A slot update keeps the same picture.
        } else if let Content::Live(old) = previous {
            recycled.push((id, old));
        }
        state.content_ink = match &state.content {
            Content::None => None,
            Content::Live(picture) | Content::Shared(picture) => {
                let list = picture.display_list();
                content_ink(
                    list,
                    0..list.commands().len(),
                    &text_layouts,
                    &registry.lock(),
                )
            }
        };
        state.record_dirty = true;
        state.ink_dirty = true;
        Ok(())
    }

    /// Samples the tree's animations at `time` and writes the frame:
    /// changed node properties, re-recorded lists, the host order and the
    /// queued releases.
    ///
    /// # Errors
    ///
    /// A [`HwuiError`] naming the first layer this target cannot render.
    pub fn frame(
        &mut self,
        time: waterui_graphics::draw::Instant,
        scale: f64,
    ) -> Result<Frame<'_>, HwuiError> {
        self.ensure_open();
        let sampling = self.tree.sample(time, scale);
        let mut pending = std::mem::take(&mut self.pending);
        let layers = self.encode_layers(&mut pending);
        pending.reset();
        self.pending = pending;
        layers?;
        self.registry.drain_releases(&mut self.buffer)?;
        let layers = &self.layers;
        self.text_layouts.drain_releases(&mut self.buffer, |raw| {
            let resource = ResourceId::TextLayout(TextLayoutId::new(raw));
            layers.values().any(|layer| layer.content.draws(resource))
        })?;
        self.open = false;
        let words = self.buffer.finish()?;
        tracing::trace!(bytes = words.len() * 4, "HWUI frame encoded");
        Ok(Frame {
            words,
            sampling,
            content_animating: self.content_animating,
        })
    }

    /// Writes the frame's node ops, re-recorded lists and host order for
    /// the layers whose tree stamps moved; in layer order, as a full diff
    /// would.
    fn encode_layers(&mut self, pending: &mut Pending) -> Result<(), HwuiError> {
        self.changed(pending);
        self.diff(pending)?;
        let root = self.tree.root();
        self.update_ink(root, &mut pending.touched)?;
        pending.touched.sort_unstable();
        pending.touched.dedup();
        for &raw in &pending.touched {
            if self.layers.contains_key(&raw) {
                self.properties(raw)?;
            }
        }
        for &raw in &pending.touched {
            if self
                .layers
                .get(&raw)
                .is_some_and(|state| state.record_dirty)
            {
                self.record(raw)?;
            }
        }
        if self.host_dirty {
            self.host_order()?;
        }
        Ok(())
    }

    /// Lists, in layer order, the layers whose tree stamps moved past the
    /// ones last diffed.
    fn changed(&self, pending: &mut Pending) {
        for (&raw, state) in &self.layers {
            let node = self.tree.layer(LayerId::new(raw));
            let stamps = (node.inner_stamp(), node.outer_stamp());
            let (inner, outer) = state.encoded.map_or((true, true), |(inner, outer)| {
                (stamps.0 != inner, stamps.1 != outer)
            });
            if inner || outer {
                pending.changed.push(Changed {
                    raw,
                    stamps,
                    inner,
                    outer,
                });
            }
        }
    }

    /// Compares the changed layers with the tree: the outer stamp gates the
    /// placement and opacity, the inner stamp the children, clip and scroll.
    /// Writes alpha and composite changes and marks what must re-record or
    /// re-measure.
    fn diff(&mut self, pending: &mut Pending) -> Result<(), HwuiError> {
        for index in 0..pending.changed.len() {
            let Changed {
                raw,
                stamps,
                inner,
                outer,
            } = pending.changed[index];
            pending.touched.push(raw);
            let id = LayerId::new(raw);
            if inner {
                self.refuse(id)?;
            }
            let placement = if outer {
                Some(self.placement(id)?)
            } else {
                None
            };
            let node = self.tree.layer(id);
            let opacity = node.opacity;
            let blend = blend_code(node.blend);
            {
                let state = self.layers.get_mut(&raw).expect("listed above");
                state.encoded = Some(stamps);
                if let Some(placement) = placement
                    && placement != state.placement
                {
                    let props_only = property_only(state.placement, placement);
                    state.placement = placement;
                    if !props_only && let Some(parent) = state.parent {
                        pending.moved_parents.push(parent);
                    }
                    pending.ink_dirty.push(id);
                }
                if outer
                    && state.sent.composite.map(|(sent, _)| sent) != Some(blend)
                    && let Some(parent) = state.parent
                {
                    pending.recomposite.push(parent.raw());
                }
                if inner {
                    diff_inner(id, state, node, pending);
                }
            }
            let needs_inner = {
                let state = &self.layers[&raw];
                state.inner.is_none() && state.scroll != Vec2::ZERO
            };
            let mut lower = Lower {
                buffer: &mut self.buffer,
                pools: &mut self.pools,
                caches: &mut self.caches,
                registry: &self.registry,
                text_layouts: &self.text_layouts,
                scratch: &mut self.scratch,
                api_level: self.api_level,
                layer: raw,
            };
            let inner = if needs_inner {
                Some(lower.create_node()?)
            } else {
                None
            };
            let state = self.layers.get_mut(&raw).expect("listed above");
            if let Some(inner) = inner {
                state.inner = Some(inner);
                state.record_dirty = true;
            }
            if state.sent.alpha != Some(opacity) {
                lower.buffer.op(
                    Op::SetAlpha,
                    &[Field::U("node", state.outer), Field::F("alpha", opacity)],
                )?;
                state.sent.alpha = Some(opacity);
            }
            self.composite(raw)?;
        }
        for index in 0..pending.recomposite.len() {
            self.composite(pending.recomposite[index])?;
        }
        self.settle(pending)
    }

    /// Writes `raw`'s blend and isolation when they changed: its own blend,
    /// or a blend within it, which a child's blend or content changes.
    fn composite(&mut self, raw: u64) -> Result<(), HwuiError> {
        let Some(state) = self.layers.get_mut(&raw) else {
            return Ok(());
        };
        let node = self.tree.layer(LayerId::new(raw));
        let composite = (
            blend_code(node.blend),
            node.blend != BlendMode::Normal || node.blends_within(),
        );
        if state.sent.composite != Some(composite) {
            self.buffer.op(
                Op::SetComposite,
                &[
                    Field::U("node", state.outer),
                    Field::U("blend", composite.0),
                    Field::U("layer", u32::from(composite.1)),
                ],
            )?;
            state.sent.composite = Some(composite);
        }
        Ok(())
    }

    /// Refuses what the target cannot mount in layer `id`'s own space.
    fn refuse(&self, id: LayerId) -> Result<(), HwuiError> {
        let raw = id.raw();
        let node = self.tree.layer(id);
        if node.filter.is_some() {
            return Err(HwuiError::Unsupported {
                layer: raw,
                what:
                    "filters its subtree; filter lowering to RenderEffect is water-rs/waterui#1750"
                        .to_owned(),
            });
        }
        if node.backdrop.is_some() {
            return Err(HwuiError::Unsupported {
                layer: raw,
                what: "samples a backdrop; the HWUI target does not implement BackdropSampling"
                    .to_owned(),
            });
        }
        Ok(())
    }

    /// Where layer `id` sits in its parent.
    fn placement(&self, id: LayerId) -> Result<Placement, HwuiError> {
        let raw = id.raw();
        let node = self.tree.layer(id);
        Ok(match self.tree.projective_pose(id) {
            Some(Ok(pose)) => {
                let rows = pose.plane_homography();
                flat_affine(rows)
                    .and_then(decompose)
                    .map(Placement::Node)
                    .or_else(|| tilt(rows).map(Placement::Camera))
                    .unwrap_or(Placement::Concat(rows))
            }
            Some(Err(source)) => return Err(HwuiError::Projective { layer: raw, source }),
            None => decompose(node.transform).map_or_else(
                || Placement::Concat(affine_rows(node.transform)),
                Placement::Node,
            ),
        })
    }

    /// Re-links reparented children and propagates re-record and ink marks
    /// up to the root.
    fn settle(&mut self, pending: &mut Pending) -> Result<(), HwuiError> {
        for (parent, old) in &pending.reparent {
            for child in old {
                if let Some(state) = self.layers.get_mut(&child.raw())
                    && state.parent == Some(*parent)
                {
                    state.parent = None;
                }
            }
        }
        for (parent, _) in &pending.reparent {
            for index in 0..self.layers[&parent.raw()].children.len() {
                let child = self.layers[&parent.raw()].children[index];
                self.state(child)?.parent = Some(*parent);
            }
        }
        for parent in &pending.moved_parents {
            if let Some(state) = self.layers.get_mut(&parent.raw()) {
                state.record_dirty = true;
                pending.touched.push(parent.raw());
            }
        }
        for &id in &pending.ink_dirty {
            let mut at = Some(id);
            while let Some(layer) = at {
                let Some(state) = self.layers.get_mut(&layer.raw()) else {
                    break;
                };
                state.ink_dirty = true;
                at = state.parent;
            }
        }
        Ok(())
    }

    /// Recomputes the ink of `id`'s dirty subtree, bottom-up, and the node
    /// extents it implies, adding each layer it recomputes to `touched`.
    fn update_ink(&mut self, id: LayerId, touched: &mut Vec<u64>) -> Result<(), HwuiError> {
        let Some(state) = self.layers.get(&id.raw()) else {
            return Ok(());
        };
        if !state.ink_dirty {
            return Ok(());
        }
        let mut inner = state.content_ink;
        for index in 0..state.children.len() {
            let child = self.layers[&id.raw()].children[index];
            self.update_ink(child, touched)?;
            let child_state = &self.layers[&child.raw()];
            let Some(ink) = child_state.ink else {
                continue;
            };
            let image = match child_state.placement {
                Placement::Node(_) => {
                    Some(self.tree.layer(child).transform.transform_rect_bbox(ink))
                }
                Placement::Camera(Tilted { rows, .. }) | Placement::Concat(rows) => {
                    project_rect(rows, ink)
                }
            };
            inner = union(inner, image);
        }
        let state = self.layers.get_mut(&id.raw()).expect("checked above");
        let shifted = inner.map(|rect| rect - state.scroll);
        let outer = state
            .clip
            .as_ref()
            .map_or(shifted, |clip| intersect(shifted, clip.bounds()));
        state.ink = outer;
        state.inner_ink = inner;
        state.ink_dirty = false;
        touched.push(id.raw());
        let recorded = state.recorded;
        let (origin, size, grew) = fit(recorded, state.origin, state.size, outer);
        state.origin = origin;
        state.size = size;
        state.record_dirty |= grew;
        if state.inner.is_some() {
            let (origin, size, grew) = fit(
                recorded && state.inner_size != [0, 0],
                state.inner_origin,
                state.inner_size,
                inner,
            );
            state.inner_origin = origin;
            state.inner_size = size;
            state.record_dirty |= grew;
        }
        Ok(())
    }

    /// Writes `id`'s changed node properties.
    fn properties(&mut self, raw: u64) -> Result<(), HwuiError> {
        let state = self.layers.get_mut(&raw).expect("listed");
        let (left, transform) = node_transform(state.placement, state.origin);
        let [l, t, r, b] = bounds(left, state.size);
        let clips = !matches!(state.content, Content::None) && state.children.is_empty();
        let position = [l, t, r, b, i32::from(clips)];
        let clip = match state.clip_mode {
            ClipMode::Node(kind, [cl, ct, cr, cb], radius) => {
                let [ox, oy] = state.origin;
                (kind, [cl - ox, ct - oy, cr - ox, cb - oy], radius)
            }
            ClipMode::None | ClipMode::List => (clip_kind::NONE, [0; 4], 0.0),
        };
        let buffer = &mut self.buffer;
        let node = state.outer;
        if state.sent.position != Some(position) {
            set_position(buffer, node, [l, t, r, b], clips)?;
            state.sent.position = Some(position);
        }
        if state.sent.transform != Some(transform) {
            set_transform(buffer, node, &transform)?;
            state.sent.transform = Some(transform);
        }
        if state.sent.clip != Some(clip) {
            let (kind, [cl, ct, cr, cb], radius) = clip;
            buffer.op(
                Op::SetClip,
                &[
                    Field::U("node", node),
                    Field::U("kind", kind),
                    Field::I("left", cl),
                    Field::I("top", ct),
                    Field::I("right", cr),
                    Field::I("bottom", cb),
                    Field::F("radius", radius),
                ],
            )?;
            state.sent.clip = Some(clip);
        }
        if let Some(inner) = state.inner {
            let position = bounds(state.inner_origin, state.inner_size);
            if state.sent.inner_position != Some(position) {
                set_position(buffer, inner, position, false)?;
                state.sent.inner_position = Some(position);
            }
            let scroll = [f32_of(-state.scroll.x), f32_of(-state.scroll.y)];
            if state.sent.inner_transform != Some(scroll) {
                set_transform(
                    buffer,
                    inner,
                    &[scroll[0], scroll[1], 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                )?;
                state.sent.inner_transform = Some(scroll);
            }
        }
        Ok(())
    }

    /// Re-records `id`'s display lists.
    fn record(&mut self, raw: u64) -> Result<(), HwuiError> {
        let Self {
            layers,
            buffer,
            pools,
            caches,
            registry,
            text_layouts,
            scratch,
            api_level,
            ..
        } = self;
        let state = &layers[&raw];
        let mut lower = Lower {
            buffer,
            pools,
            caches,
            registry,
            text_layouts,
            scratch,
            api_level: *api_level,
            layer: raw,
        };
        let mut owned = Owned::default();
        let mut groups = GroupNodes::new();
        let shared = match &state.content {
            Content::None => None,
            Content::Live(picture) => {
                let list = picture.display_list();
                lower.prepare(list, 0..list.commands().len(), &mut owned, &mut groups)?;
                None
            }
            Content::Shared(picture) => Some(lower.share(picture, &mut owned)?),
        };
        lower.open_at(state.outer, state.origin, state.size)?;
        if state.clip_mode == ClipMode::List
            && let Some(clip) = &state.clip
        {
            lower.clip(clip, &mut owned)?;
        }
        if let Some(inner) = state.inner {
            lower.draw_node(inner, None)?;
            lower.close()?;
            lower.open_at(inner, state.inner_origin, state.inner_size)?;
        }
        match (&state.content, shared) {
            (Content::Live(picture), _) => {
                let list = picture.display_list();
                lower.emit(list, 0..list.commands().len(), &groups, &mut owned)?;
            }
            (_, Some(node)) => lower.draw_node(node, None)?,
            _ => {}
        }
        for child in &state.children {
            let child = &layers[&child.raw()];
            match child.placement {
                Placement::Node(_) | Placement::Camera(_) => lower.draw_node(child.outer, None)?,
                Placement::Concat(rows) => {
                    lower.draw_node(child.outer, Some(&homography_matrix(rows)))?;
                }
            }
        }
        lower.close()?;
        let state = layers.get_mut(&raw).expect("listed");
        let previous = std::mem::replace(&mut state.owned, owned);
        state.recorded = true;
        state.record_dirty = false;
        lower.release(previous)
    }

    fn host_order(&mut self) -> Result<(), HwuiError> {
        let mut words = Vec::with_capacity(self.host.len() * 3);
        for entry in &self.host {
            let (kind, id) = match *entry {
                HostEntry::Layer(layer) => (
                    host::NODE,
                    u64::from(
                        self.layers
                            .get(&layer.raw())
                            .ok_or_else(|| HwuiError::Encoding {
                                op: "HostOrder",
                                reason: format!("layer {} is not mirrored", layer.raw()),
                            })?
                            .outer,
                    ),
                ),
                HostEntry::PlatformView(id) => (host::PLATFORM_VIEW, id),
            };
            let (lo, hi) = split(id);
            words.extend([kind, lo, hi]);
        }
        let count = u32::try_from(self.host.len()).map_err(|_| HwuiError::Encoding {
            op: "HostOrder",
            reason: "its entries exceed u32".to_owned(),
        })?;
        self.buffer.op(
            Op::HostOrder,
            &[Field::U("count", count), Field::Us("entries", &words)],
        )?;
        self.host_dirty = false;
        Ok(())
    }
}

/// Compares what layer `id` draws in its own space with the tree: its
/// children, clip and scroll offset.
fn diff_inner(id: LayerId, state: &mut LayerState, node: &LayerNode, pending: &mut Pending) {
    if node.children != state.children {
        pending.reparent.push((
            id,
            std::mem::replace(&mut state.children, node.children.clone()),
        ));
        state.record_dirty = true;
        pending.ink_dirty.push(id);
    }
    if node.clip != state.clip {
        let mode = clip_mode(node.clip.as_ref());
        if mode == ClipMode::List || state.clip_mode == ClipMode::List {
            state.record_dirty = true;
        }
        state.clip.clone_from(&node.clip);
        state.clip_mode = mode;
        pending.ink_dirty.push(id);
    }
    if node.scroll_offset != state.scroll {
        state.scroll = node.scroll_offset;
        pending.ink_dirty.push(id);
    }
    if state.ink_dirty {
        pending.ink_dirty.push(id);
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "splits the u64 into its two words on purpose"
)]
const fn split(id: u64) -> (u32, u32) {
    (id as u32, (id >> 32) as u32)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "node properties are f32 on the platform"
)]
const fn f32_of(value: f64) -> f32 {
    value as f32
}

fn set_transform(
    buffer: &mut CommandBuffer,
    node: u32,
    values: &[f32; 10],
) -> Result<(), HwuiError> {
    let names = [
        "translation_x",
        "translation_y",
        "scale_x",
        "scale_y",
        "rotation",
        "pivot_x",
        "pivot_y",
        "rotation_x",
        "rotation_y",
        "camera_distance",
    ];
    let mut fields = super::buffer::Fields::new();
    fields.push(Field::U("node", node));
    for (name, value) in names.into_iter().zip(values) {
        fields.push(Field::F(name, *value));
    }
    buffer.op(Op::SetTransform, fields.as_slice())
}

/// The outer node's integer offset and `SetTransform` properties for a
/// layer at `origin`; a `Concat` placement leaves the node untransformed.
fn node_transform(placement: Placement, origin: [i32; 2]) -> ([i32; 2], [f32; 10]) {
    let (translation, scale, rotation, pivot, camera) = match placement {
        Placement::Node(parts) => (
            parts.translation,
            parts.scale,
            parts.rotation,
            (0.0, 0.0),
            [0.0; 3],
        ),
        Placement::Camera(tilted) => (
            tilted.translation,
            tilted.scale,
            tilted.rotation,
            tilted.pivot,
            [tilted.rotation_x, tilted.rotation_y, tilted.camera_distance],
        ),
        Placement::Concat(_) => {
            return (origin, [0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        }
    };
    let origin = [f64::from(origin[0]), f64::from(origin[1])];
    let (tx, ty) = (translation.0 + origin[0], translation.1 + origin[1]);
    let left = [floor_i32(tx), floor_i32(ty)];
    (
        left,
        [
            f32_of(tx - f64::from(left[0])),
            f32_of(ty - f64::from(left[1])),
            f32_of(scale.0),
            f32_of(scale.1),
            f32_of(rotation),
            f32_of(pivot.0 - origin[0]),
            f32_of(pivot.1 - origin[1]),
            f32_of(camera[0]),
            f32_of(camera[1]),
            f32_of(camera[2]),
        ],
    )
}

/// Whether moving from `from` to `to` changes only node properties, so the
/// parent's display list stays valid.
const fn property_only(from: Placement, to: Placement) -> bool {
    matches!(
        (from, to),
        (
            Placement::Node(_) | Placement::Camera(_),
            Placement::Node(_) | Placement::Camera(_)
        )
    )
}

/// `rows` as an affine map when its projective row is `(0, 0, w > 0)`.
#[expect(
    clippy::many_single_char_names,
    reason = "a–f are the affine coefficients as kurbo names them, g, h, w the projective row"
)]
fn flat_affine(rows: [[f64; 3]; 3]) -> Option<Affine> {
    let [[a, c, e], [b, d, f], [g, h, w]] = rows;
    (g == 0.0 && h == 0.0 && w > 0.0)
        .then(|| Affine::new([a / w, b / w, c / w, d / w, e / w, f / w]))
}

#[expect(
    clippy::many_single_char_names,
    reason = "a–f are the affine coefficients as kurbo and android.graphics.Matrix name them"
)]
const fn affine_rows(affine: Affine) -> [[f64; 3]; 3] {
    let [a, b, c, d, e, f] = affine.as_coeffs();
    [[a, c, e], [b, d, f], [0.0, 0.0, 1.0]]
}

/// A node extent covering `ink`, kept while it still fits so growth
/// re-records only when the origin must move; `true` when it must.
fn fit(
    recorded: bool,
    origin: [i32; 2],
    size: [i32; 2],
    ink: Option<Rect>,
) -> ([i32; 2], [i32; 2], bool) {
    let (needed_origin, needed_size) = extent(ink);
    if !recorded {
        return (needed_origin, needed_size, true);
    }
    if ink.is_none() {
        return (origin, size, false);
    }
    if needed_origin[0] < origin[0] || needed_origin[1] < origin[1] {
        return (needed_origin, needed_size, true);
    }
    let end = [
        needed_origin[0] + needed_size[0],
        needed_origin[1] + needed_size[1],
    ];
    let size = [
        size[0].max(end[0] - origin[0]),
        size[1].max(end[1] - origin[1]),
    ];
    (origin, size, false)
}

fn clip_mode(clip: Option<&ShapeData>) -> ClipMode {
    let integral = |rect: Rect| {
        [rect.x0, rect.y0, rect.x1, rect.y1]
            .into_iter()
            .all(is_integral)
            .then(|| {
                [
                    floor_i32(rect.x0.round()),
                    floor_i32(rect.y0.round()),
                    floor_i32(rect.x1.round()),
                    floor_i32(rect.y1.round()),
                ]
            })
    };
    match clip {
        None => ClipMode::None,
        Some(ShapeData::Rect(rect)) => {
            integral(*rect).map_or(ClipMode::List, |r| ClipMode::Node(clip_kind::RECT, r, 0.0))
        }
        Some(ShapeData::RoundedRect(rounded)) => {
            match (integral(rounded.rect()), rounded.radii().as_single_radius()) {
                (Some(r), Some(radius)) => ClipMode::Node(clip_kind::ROUND_RECT, r, f32_of(radius)),
                _ => ClipMode::List,
            }
        }
        Some(_) => ClipMode::List,
    }
}
