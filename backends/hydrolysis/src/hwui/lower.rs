//! Display-list lowering: a Cherenkov [`DisplayList`] range into recording
//! ops, after the resources it names are defined.
//!
//! Lowering runs in two passes. [`Lower::prepare`] defines what the range
//! draws by reference — shared pictures and group nodes, each its own
//! `RenderNode` recorded before the caller opens its own recording, since
//! one display list is open at a time. [`Lower::emit`] then writes the
//! range into the open recording.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::Arc;

use waterui_graphics::draw::color::WorkingColor;
use waterui_graphics::draw::kurbo::{
    Affine, BezPath, Cap, Join, PathEl, Point, Rect, Shape as _, Stroke, Vec2,
};
use waterui_graphics::draw::{
    BlendMode, Command, DisplayList, Extend, FillRule, GlyphRun, GlyphStyle, Group, ImageId,
    ImagePattern, MeshColorInterpolation, MeshGradient, PATH_TOLERANCE, Paint, Picture, Sampling,
    ShaderPaint, Shadow, ShapeData, SweepGradient, blends_within,
};

use waterui_graphics::draw::TextLayoutId;

use super::HwuiError;
use super::buffer::{CommandBuffer, Field, Fields};
use super::color::{
    MAX_SWEEP_STOPS, color_long, gradient_stops, premultiplied_linear_srgb, sweep_turn,
};
use super::geometry::{Matrix, affine_matrix, ceil_i32, floor_i32, intersect, is_identity, union};
use super::ids::IdPool;
use super::protocol::{
    MESH_BAND_PATCHES, MESH_PATCH_FLOATS, Op, blend, cap, fill_type, glyph_style, join,
    mesh_interpolation, paint as paint_kind, sampling as sampling_kind, shader as shader_kind,
    shape as shape_kind, tile, verb,
};
use super::resources::{Kind, Registry, SharedRegistry};
use super::text::TextLayoutIds;

/// `drawMesh`.
pub const MESH_API: u32 = 34;
/// AGSL `RuntimeShader`.
pub const RUNTIME_SHADER_API: u32 = 33;

/// The id pools of what recordings define.
#[derive(Debug)]
pub struct Pools {
    /// `RenderNode`s.
    pub nodes: IdPool,
    /// `Path`s.
    pub paths: IdPool,
    /// `Shader`s.
    pub shaders: IdPool,
}

impl Pools {
    pub const fn new() -> Self {
        Self {
            nodes: IdPool::new("node"),
            paths: IdPool::new("path"),
            shaders: IdPool::new("shader"),
        }
    }
}

/// What one recording owns: released when it is re-recorded or dropped.
#[derive(Debug, Default)]
pub struct Owned {
    nodes: Vec<u32>,
    paths: Vec<u32>,
    shaders: Vec<u32>,
    cached_paths: Vec<PathKey>,
    pictures: Vec<usize>,
}

type PathKey = (usize, bool);

#[derive(Debug)]
struct CachedPath {
    // Holds the allocation the key's address names.
    _elements: Arc<[PathEl]>,
    id: u32,
    refs: u32,
}

#[derive(Debug)]
struct SharedPicture {
    // Holds the allocation the key's address names.
    _picture: Picture,
    node: u32,
    refs: u32,
    owned: Owned,
}

/// Definitions shared by identity across recordings.
#[derive(Debug, Default)]
pub struct Caches {
    paths: HashMap<PathKey, CachedPath>,
    pictures: BTreeMap<usize, SharedPicture>,
}

/// Reused arrays for variable-length records.
#[derive(Debug, Default)]
pub struct Scratch {
    colors: Vec<u64>,
    floats: Vec<f32>,
    words: Vec<u32>,
    intervals: Vec<f32>,
    turn_colors: Vec<u64>,
    turn_positions: Vec<f32>,
}

/// The group nodes [`Lower::prepare`] recorded, by list address and
/// command index.
pub type GroupNodes = HashMap<(usize, usize), u32>;

/// A shape as the wire carries it.
#[derive(Clone, Copy, Debug)]
enum Wire {
    Rect([f32; 4]),
    RoundRect([f32; 4], f32),
    Oval([f32; 4]),
    Path(u32),
    Line([f32; 4]),
}

impl Wire {
    const fn fields(self, out: &mut Fields<'_>) {
        match self {
            Self::Rect(r) => {
                out.push(Field::U("shape", shape_kind::RECT));
                push_rect(out, r);
                out.push(Field::Pad(2));
            }
            Self::RoundRect(r, radius) => {
                out.push(Field::U("shape", shape_kind::ROUND_RECT));
                push_rect(out, r);
                out.push(Field::F("rx", radius))
                    .push(Field::F("ry", radius));
            }
            Self::Oval(r) => {
                out.push(Field::U("shape", shape_kind::OVAL));
                push_rect(out, r);
                out.push(Field::Pad(2));
            }
            Self::Path(id) => {
                out.push(Field::U("shape", shape_kind::PATH))
                    .push(Field::U("path", id))
                    .push(Field::Pad(5));
            }
            Self::Line([x0, y0, x1, y1]) => {
                out.push(Field::U("shape", shape_kind::LINE))
                    .push(Field::F("x0", x0))
                    .push(Field::F("y0", y0))
                    .push(Field::F("x1", x1))
                    .push(Field::F("y1", y1))
                    .push(Field::Pad(2));
            }
        }
    }
}

const fn push_rect(out: &mut Fields<'_>, [l, t, r, b]: [f32; 4]) {
    out.push(Field::F("left", l))
        .push(Field::F("top", t))
        .push(Field::F("right", r))
        .push(Field::F("bottom", b));
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the wire carries f32, as android.graphics stores geometry"
)]
const fn rect4(rect: Rect) -> [f32; 4] {
    [
        rect.x0 as f32,
        rect.y0 as f32,
        rect.x1 as f32,
        rect.y1 as f32,
    ]
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the wire carries f32, as android.graphics stores geometry"
)]
const fn f(value: f64) -> f32 {
    value as f32
}

/// A paint as the wire carries it.
#[derive(Clone, Copy, Debug)]
enum WirePaint {
    Color(u64),
    Shader(u32, f32),
}

impl WirePaint {
    const fn fields(self, out: &mut Fields<'_>) {
        match self {
            Self::Color(color) => {
                out.push(Field::U("paint", paint_kind::COLOR))
                    .push(Field::Color("color", color))
                    .push(Field::F("alpha", 1.0));
            }
            Self::Shader(id, alpha) => {
                out.push(Field::U("paint", paint_kind::SHADER))
                    .push(Field::U("shader", id))
                    .push(Field::Pad(1))
                    .push(Field::F("alpha", alpha));
            }
        }
    }
}

/// How a group lowers.
#[derive(Clone, Copy, Debug, PartialEq)]
enum GroupLowering {
    /// No isolation is observable: the scope draws inline.
    Inline,
    /// One paint-bearing draw: the opacity folds into its paint.
    Fold(f32),
    /// Its own node with alpha, a compositing layer and the blend mode.
    Node,
}

fn classify(group: &Group, list: &DisplayList, inner: Range<usize>) -> GroupLowering {
    if group.blend == BlendMode::Normal {
        if group.opacity >= 1.0 && !blends_within(list, inner.clone()) {
            return GroupLowering::Inline;
        }
        if inner.len() == 1 {
            let foldable = match &list.commands()[inner.start] {
                Command::Fill { paint, .. } | Command::Stroke { paint, .. } => {
                    !matches!(paint, Paint::Mesh(_))
                }
                Command::Glyphs { paint, .. } => !matches!(paint, Paint::Mesh(_)),
                Command::Shadow { .. } => true,
                _ => false,
            };
            if foldable {
                return GroupLowering::Fold(group.opacity);
            }
        }
    }
    GroupLowering::Node
}

fn list_key(list: &DisplayList) -> usize {
    std::ptr::from_ref(list).addr()
}

fn picture_key(picture: &Picture) -> usize {
    list_key(picture.display_list())
}

/// The android `BlendMode` code of `blend`.
#[must_use]
pub const fn blend_code(blend: BlendMode) -> u32 {
    use super::protocol::blend as b;
    match blend {
        BlendMode::Normal => b::SRC_OVER,
        BlendMode::Multiply => b::MULTIPLY,
        BlendMode::Screen => b::SCREEN,
        BlendMode::Overlay => b::OVERLAY,
        BlendMode::Darken => b::DARKEN,
        BlendMode::Lighten => b::LIGHTEN,
        BlendMode::ColorDodge => b::COLOR_DODGE,
        BlendMode::ColorBurn => b::COLOR_BURN,
        BlendMode::HardLight => b::HARD_LIGHT,
        BlendMode::SoftLight => b::SOFT_LIGHT,
        BlendMode::Difference => b::DIFFERENCE,
        BlendMode::Exclusion => b::EXCLUSION,
        BlendMode::Hue => b::HUE,
        BlendMode::Saturation => b::SATURATION,
        BlendMode::Color => b::COLOR,
        BlendMode::Luminosity => b::LUMINOSITY,
        BlendMode::Clear => b::CLEAR,
        BlendMode::Src => b::SRC,
        BlendMode::Dst => b::DST,
        BlendMode::DestOver => b::DST_OVER,
        BlendMode::SrcIn => b::SRC_IN,
        BlendMode::DestIn => b::DST_IN,
        BlendMode::SrcOut => b::SRC_OUT,
        BlendMode::DestOut => b::DST_OUT,
        BlendMode::SrcAtop => b::SRC_ATOP,
        BlendMode::DestAtop => b::DST_ATOP,
        BlendMode::Xor => b::XOR,
        BlendMode::PlusLighter => b::PLUS,
    }
}

/// One lowering pass's borrows of the encoder.
pub struct Lower<'a> {
    pub buffer: &'a mut CommandBuffer,
    pub pools: &'a mut Pools,
    pub caches: &'a mut Caches,
    pub registry: &'a SharedRegistry,
    pub text_layouts: &'a TextLayoutIds,
    pub scratch: &'a mut Scratch,
    pub api_level: u32,
    /// The layer being lowered, for errors.
    pub layer: u64,
}

impl Lower<'_> {
    fn unsupported(&self, what: impl Into<String>) -> HwuiError {
        HwuiError::Unsupported {
            layer: self.layer,
            what: what.into(),
        }
    }

    const fn require(&self, what: &'static str, needs: u32) -> Result<(), HwuiError> {
        if self.api_level >= needs {
            Ok(())
        } else {
            Err(HwuiError::RequiresApi {
                layer: self.layer,
                what,
                needs,
                device: self.api_level,
            })
        }
    }

    /// Creates a node.
    pub fn create_node(&mut self) -> Result<u32, HwuiError> {
        let node = self.pools.nodes.acquire()?;
        self.buffer.op(Op::CreateNode, &[Field::U("node", node)])?;
        Ok(node)
    }

    /// Queues a node's release.
    pub fn release_node(&mut self, node: u32) -> Result<(), HwuiError> {
        self.buffer.op(Op::ReleaseNode, &[Field::U("node", node)])?;
        self.pools.nodes.release(node);
        Ok(())
    }

    /// Opens `node`'s recording sized to `ink`, translated so `ink`'s
    /// integer origin is the node's; returns that origin and size.
    pub fn open(&mut self, node: u32, ink: Option<Rect>) -> Result<(), HwuiError> {
        let (origin, size) = extent(ink);
        self.open_at(node, origin, size)
    }

    /// Opens `node`'s recording of `size`, translated by `-origin`.
    pub fn open_at(
        &mut self,
        node: u32,
        origin: [i32; 2],
        size: [i32; 2],
    ) -> Result<(), HwuiError> {
        self.buffer.op(
            Op::Record,
            &[
                Field::U("node", node),
                Field::I("width", size[0]),
                Field::I("height", size[1]),
            ],
        )?;
        if origin != [0, 0] {
            self.concat(&affine_matrix(Affine::translate(Vec2::new(
                -f64::from(origin[0]),
                -f64::from(origin[1]),
            ))))?;
        }
        Ok(())
    }

    /// Closes the open recording.
    pub fn close(&mut self) -> Result<(), HwuiError> {
        self.buffer.op(Op::EndRecord, &[])
    }

    /// `Concat`.
    pub fn concat(&mut self, matrix: &Matrix) -> Result<(), HwuiError> {
        self.buffer.op(Op::Concat, &[Field::Fs("matrix", matrix)])
    }

    /// `DrawNode`, under `matrix` unless it is `None`.
    pub fn draw_node(&mut self, node: u32, matrix: Option<&Matrix>) -> Result<(), HwuiError> {
        if let Some(matrix) = matrix {
            self.buffer.op(Op::Save, &[])?;
            self.concat(matrix)?;
            self.buffer.op(Op::DrawNode, &[Field::U("node", node)])?;
            self.buffer.op(Op::Restore, &[])
        } else {
            self.buffer.op(Op::DrawNode, &[Field::U("node", node)])
        }
    }

    /// `Text`: draws platform text layout `layout`'s node under
    /// `transform`.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Unregistered`] naming the layer when the platform holds
    /// no such layout.
    pub fn text(&mut self, layout: TextLayoutId, transform: Affine) -> Result<(), HwuiError> {
        let layout = self.text_layouts.resolve(layout.raw(), self.layer)?;
        self.buffer.op(
            Op::Text,
            &[
                Field::U("layout", layout),
                Field::Fs("matrix", &affine_matrix(transform)),
            ],
        )
    }

    /// Releases what `owned` holds, dropping cached definitions nothing
    /// else uses.
    pub fn release(&mut self, owned: Owned) -> Result<(), HwuiError> {
        let Owned {
            nodes,
            paths,
            shaders,
            cached_paths,
            pictures,
        } = owned;
        for node in nodes {
            self.release_node(node)?;
        }
        for path in paths {
            self.release_path(path)?;
        }
        for shader in shaders {
            self.buffer
                .op(Op::ReleaseShader, &[Field::U("shader", shader)])?;
            self.pools.shaders.release(shader);
        }
        for key in cached_paths {
            let Some(entry) = self.caches.paths.get_mut(&key) else {
                continue;
            };
            entry.refs -= 1;
            if entry.refs == 0 {
                let id = entry.id;
                self.caches.paths.remove(&key);
                self.release_path(id)?;
            }
        }
        for key in pictures {
            let Some(entry) = self.caches.pictures.get_mut(&key) else {
                continue;
            };
            entry.refs -= 1;
            if entry.refs == 0 {
                let entry = self.caches.pictures.remove(&key).expect("present above");
                self.release_node(entry.node)?;
                self.release(entry.owned)?;
            }
        }
        Ok(())
    }

    fn release_path(&mut self, path: u32) -> Result<(), HwuiError> {
        self.buffer.op(Op::ReleasePath, &[Field::U("path", path)])?;
        self.pools.paths.release(path);
        Ok(())
    }

    /// Defines what `range` of `list` draws by reference: shared pictures
    /// and group nodes. No recording may be open.
    pub fn prepare(
        &mut self,
        list: &DisplayList,
        range: Range<usize>,
        owned: &mut Owned,
        groups: &mut GroupNodes,
    ) -> Result<(), HwuiError> {
        let commands = list.commands();
        let mut index = range.start;
        while index < range.end {
            match &commands[index] {
                Command::Picture { picture, .. } => {
                    self.share(picture, owned)?;
                }
                Command::BeginGroup { group, end } => {
                    let end = *end as usize;
                    if group.filter.is_some() {
                        return Err(self.unsupported(
                            "filters a group; filter lowering to RenderEffect is water-rs/waterui#1750",
                        ));
                    }
                    let inner = index + 1..end;
                    self.prepare(list, inner.clone(), owned, groups)?;
                    if classify(group, list, inner.clone()) == GroupLowering::Node {
                        let node = self.create_node()?;
                        owned.nodes.push(node);
                        let ink = content_ink(
                            list,
                            inner.clone(),
                            self.text_layouts,
                            &self.registry.lock(),
                        );
                        let (origin, size) = extent(ink);
                        set_position(self.buffer, node, bounds(origin, size), true)?;
                        self.buffer.op(
                            Op::SetAlpha,
                            &[Field::U("node", node), Field::F("alpha", group.opacity)],
                        )?;
                        self.buffer.op(
                            Op::SetComposite,
                            &[
                                Field::U("node", node),
                                Field::U("blend", blend_code(group.blend)),
                                Field::U("layer", 1),
                            ],
                        )?;
                        self.open(node, ink)?;
                        self.emit(list, inner, groups, owned)?;
                        self.close()?;
                        groups.insert((list_key(list), index), node);
                    }
                    index = end;
                }
                command => {
                    if let Some((mesh, local)) = command_paint(command).and_then(mesh_of) {
                        self.prepare_mesh(list, index, mesh, local, owned, groups)?;
                    }
                }
            }
            index += 1;
        }
        Ok(())
    }

    /// Records the node a mesh-painted draw at `index` draws: a fill clips
    /// the mesh to its shape; a stroke or glyph run draws its coverage and
    /// masks the mesh into it with `SRC_IN`. Both mesh nodes are isolated,
    /// so a later patch replaces an earlier one, as the contract's
    /// last-patch ownership says.
    fn prepare_mesh(
        &mut self,
        list: &DisplayList,
        index: usize,
        mesh: &MeshGradient,
        local: Affine,
        owned: &mut Owned,
        groups: &mut GroupNodes,
    ) -> Result<(), HwuiError> {
        self.require("a mesh gradient", MESH_API)?;
        let command = &list.commands()[index];
        let ink = content_ink(
            list,
            index..index + 1,
            self.text_layouts,
            &self.registry.lock(),
        );
        let (origin, size) = extent(ink);
        let fill = matches!(command, Command::Fill { .. });
        let painted = self.isolated_node(
            origin,
            size,
            if fill { blend::SRC_OVER } else { blend::SRC_IN },
            owned,
        )?;
        self.open_at(painted, origin, size)?;
        if let Command::Fill { shape, .. } = command {
            self.clip(shape, owned)?;
        }
        self.mesh(mesh, local)?;
        self.close()?;
        let node = if fill {
            painted
        } else {
            let node = self.isolated_node(origin, size, blend::SRC_OVER, owned)?;
            self.open_at(node, origin, size)?;
            let coverage = Paint::Solid(WorkingColor::new([1.0; 4]));
            match command {
                Command::Stroke { shape, stroke, .. } => {
                    self.stroke(shape, stroke, &coverage, 1.0, owned)?;
                }
                Command::Glyphs { run, .. } => self.glyphs(run, &coverage, 1.0, owned)?,
                _ => {
                    return Err(HwuiError::Encoding {
                        op: "Mesh",
                        reason: "only fills, strokes and glyph runs carry a paint".to_owned(),
                    });
                }
            }
            self.draw_node(painted, None)?;
            self.close()?;
            node
        };
        groups.insert((list_key(list), index), node);
        Ok(())
    }

    /// Draws the node [`Self::prepare_mesh`] recorded for the mesh-painted
    /// draw at `index`.
    fn draw_mesh_node(
        &mut self,
        list: &DisplayList,
        index: usize,
        alpha: f32,
        groups: &GroupNodes,
    ) -> Result<(), HwuiError> {
        if alpha < 1.0 {
            return Err(HwuiError::Encoding {
                op: "DrawNode",
                reason: "a mesh gradient's node cannot take a folded opacity".to_owned(),
            });
        }
        let node = *groups
            .get(&(list_key(list), index))
            .ok_or_else(|| HwuiError::Encoding {
                op: "DrawNode",
                reason: "a mesh gradient's node was not prepared".to_owned(),
            })?;
        self.draw_node(node, None)
    }

    /// A node of `size` at `origin` composited through its own layer with
    /// `blend`, owned by the layer.
    fn isolated_node(
        &mut self,
        origin: [i32; 2],
        size: [i32; 2],
        blend: u32,
        owned: &mut Owned,
    ) -> Result<u32, HwuiError> {
        let node = self.create_node()?;
        owned.nodes.push(node);
        set_position(self.buffer, node, bounds(origin, size), true)?;
        self.buffer.op(
            Op::SetComposite,
            &[
                Field::U("node", node),
                Field::U("blend", blend),
                Field::U("layer", 1),
            ],
        )?;
        Ok(node)
    }

    /// The node of the shared `picture`, recorded once and reused by
    /// identity; `owned` takes a reference.
    pub fn share(&mut self, picture: &Picture, owned: &mut Owned) -> Result<u32, HwuiError> {
        let key = picture_key(picture);
        if let Some(entry) = self.caches.pictures.get_mut(&key) {
            entry.refs += 1;
            owned.pictures.push(key);
            return Ok(entry.node);
        }
        let list = picture.display_list();
        let all = 0..list.commands().len();
        let mut inner = Owned::default();
        let mut groups = GroupNodes::new();
        self.prepare(list, all.clone(), &mut inner, &mut groups)?;
        let node = self.create_node()?;
        let ink = content_ink(list, all.clone(), self.text_layouts, &self.registry.lock());
        let (origin, size) = extent(ink);
        set_position(self.buffer, node, bounds(origin, size), true)?;
        self.open(node, ink)?;
        self.emit(list, all, &groups, &mut inner)?;
        self.close()?;
        self.caches.pictures.insert(
            key,
            SharedPicture {
                _picture: picture.clone(),
                node,
                refs: 1,
                owned: inner,
            },
        );
        owned.pictures.push(key);
        Ok(node)
    }

    fn image(&mut self, image: ImageId, dst: Rect, sampling: Sampling) -> Result<(), HwuiError> {
        let bitmap = self
            .registry
            .lock()
            .resolve(Kind::Bitmap, image.raw(), self.layer)?;
        let mut fields = Fields::new();
        fields.push(Field::U("bitmap", bitmap));
        push_rect(&mut fields, rect4(dst));
        fields.push(Field::U("sampling", sampling_code(sampling)));
        self.buffer.op(Op::Image, fields.as_slice())
    }

    /// Draws a shared picture's node.
    fn picture(&mut self, picture: &Picture, transform: Affine) -> Result<(), HwuiError> {
        let node = self
            .caches
            .pictures
            .get(&picture_key(picture))
            .ok_or_else(|| HwuiError::Encoding {
                op: "DrawNode",
                reason: "a shared picture was not prepared".to_owned(),
            })?
            .node;
        let matrix = (!is_identity(transform)).then(|| affine_matrix(transform));
        self.draw_node(node, matrix.as_ref())
    }

    /// Writes `range` of `list` into the open recording.
    pub fn emit(
        &mut self,
        list: &DisplayList,
        range: Range<usize>,
        groups: &GroupNodes,
        owned: &mut Owned,
    ) -> Result<(), HwuiError> {
        struct Scope {
            restore: bool,
            alpha: f32,
        }
        let commands = list.commands();
        let mut scopes: Vec<Scope> = Vec::new();
        let mut alpha = 1.0f32;
        let mut index = range.start;
        while index < range.end {
            match &commands[index] {
                command if command_paint(command).and_then(mesh_of).is_some() => {
                    self.draw_mesh_node(list, index, alpha, groups)?;
                }
                Command::Fill { shape, paint } => self.fill(shape, paint, alpha, owned)?,
                Command::Stroke {
                    shape,
                    stroke,
                    paint,
                } => self.stroke(shape, stroke, paint, alpha, owned)?,
                Command::Shadow { shape, shadow } => self.shadow(shape, shadow, alpha, owned)?,
                Command::Glyphs { run, paint } => self.glyphs(run, paint, alpha, owned)?,
                Command::Image {
                    image,
                    dst,
                    sampling,
                } => self.image(*image, *dst, *sampling)?,
                Command::Picture { picture, transform } => self.picture(picture, *transform)?,
                Command::Text { layout, transform } => {
                    if alpha < 1.0 {
                        return Err(HwuiError::Encoding {
                            op: "Text",
                            reason: "a text layout's node cannot take a folded opacity".to_owned(),
                        });
                    }
                    self.text(*layout, *transform)?;
                }
                Command::BeginClip { shape, .. } => {
                    self.buffer.op(Op::Save, &[])?;
                    self.clip(shape, owned)?;
                    scopes.push(Scope {
                        restore: true,
                        alpha,
                    });
                }
                Command::BeginTransform { transform, .. } => {
                    self.buffer.op(Op::Save, &[])?;
                    self.concat(&affine_matrix(*transform))?;
                    scopes.push(Scope {
                        restore: true,
                        alpha,
                    });
                }
                Command::BeginGroup { group, end } => {
                    let inner = index + 1..*end as usize;
                    match classify(group, list, inner) {
                        GroupLowering::Inline => scopes.push(Scope {
                            restore: false,
                            alpha,
                        }),
                        GroupLowering::Fold(opacity) => {
                            scopes.push(Scope {
                                restore: false,
                                alpha,
                            });
                            alpha *= opacity;
                        }
                        GroupLowering::Node => {
                            let node = *groups.get(&(list_key(list), index)).ok_or_else(|| {
                                HwuiError::Encoding {
                                    op: "DrawNode",
                                    reason: "a group node was not prepared".to_owned(),
                                }
                            })?;
                            self.draw_node(node, None)?;
                            index = *end as usize + 1;
                            continue;
                        }
                    }
                }
                Command::End => {
                    let scope = scopes.pop().ok_or_else(|| HwuiError::Encoding {
                        op: "Restore",
                        reason: "an End closes no scope".to_owned(),
                    })?;
                    if scope.restore {
                        self.buffer.op(Op::Restore, &[])?;
                    }
                    alpha = scope.alpha;
                }
            }
            index += 1;
        }
        Ok(())
    }

    /// Clips the open recording to `shape`.
    pub fn clip(&mut self, shape: &ShapeData, owned: &mut Owned) -> Result<(), HwuiError> {
        if let ShapeData::Rect(rect) = shape {
            let [l, t, r, b] = rect4(*rect);
            return self.buffer.op(
                Op::ClipRect,
                &[
                    Field::F("left", l),
                    Field::F("top", t),
                    Field::F("right", r),
                    Field::F("bottom", b),
                ],
            );
        }
        let path = match self.wire(shape, false, owned)? {
            Some(Wire::Path(path)) => path,
            Some(_) => {
                let elements: Vec<PathEl> = shape_path(shape).elements().to_vec();
                let path = self.define_path(&elements, FillRule::NonZero)?;
                owned.paths.push(path);
                path
            }
            // A line encloses nothing: everything is clipped away.
            None => {
                return self.buffer.op(
                    Op::ClipRect,
                    &[
                        Field::F("left", 0.0),
                        Field::F("top", 0.0),
                        Field::F("right", 0.0),
                        Field::F("bottom", 0.0),
                    ],
                );
            }
        };
        self.buffer.op(Op::ClipPath, &[Field::U("path", path)])
    }

    /// `shape` on the wire; `None` for a line that is filled (it encloses
    /// nothing).
    fn wire(
        &mut self,
        shape: &ShapeData,
        stroked: bool,
        owned: &mut Owned,
    ) -> Result<Option<Wire>, HwuiError> {
        Ok(Some(match shape {
            ShapeData::Rect(rect) => Wire::Rect(rect4(*rect)),
            ShapeData::RoundedRect(rounded) => {
                if let Some(radius) = rounded.radii().as_single_radius() {
                    Wire::RoundRect(rect4(rounded.rect()), f(radius))
                } else {
                    self.owned_path(shape, owned)?
                }
            }
            ShapeData::Circle(circle) => Wire::Oval(rect4(circle.bounding_box())),
            ShapeData::Ellipse(ellipse) => {
                if ellipse.rotation().sin().abs() <= 1e-9 {
                    Wire::Oval(rect4(ellipse.bounding_box()))
                } else {
                    self.owned_path(shape, owned)?
                }
            }
            ShapeData::Continuous(_) => self.owned_path(shape, owned)?,
            ShapeData::Line(line) => {
                if !stroked {
                    return Ok(None);
                }
                Wire::Line([f(line.p0.x), f(line.p0.y), f(line.p1.x), f(line.p1.y)])
            }
            ShapeData::Path { elements, rule } => {
                let key = (
                    Arc::as_ptr(elements).cast::<()>().addr(),
                    *rule == FillRule::EvenOdd,
                );
                if let Some(entry) = self.caches.paths.get_mut(&key) {
                    entry.refs += 1;
                } else {
                    let id = self.define_path(elements, *rule)?;
                    self.caches.paths.insert(
                        key,
                        CachedPath {
                            _elements: Arc::clone(elements),
                            id,
                            refs: 1,
                        },
                    );
                }
                owned.cached_paths.push(key);
                Wire::Path(self.caches.paths[&key].id)
            }
        }))
    }

    fn owned_path(&mut self, shape: &ShapeData, owned: &mut Owned) -> Result<Wire, HwuiError> {
        let path = shape_path(shape);
        let id = self.define_path(path.elements(), FillRule::NonZero)?;
        owned.paths.push(id);
        Ok(Wire::Path(id))
    }

    fn define_path(&mut self, elements: &[PathEl], rule: FillRule) -> Result<u32, HwuiError> {
        let id = self.pools.paths.acquire()?;
        let Scratch { words, floats, .. } = &mut *self.scratch;
        words.clear();
        floats.clear();
        for element in elements {
            let (code, points): (u32, &[Point]) = match element {
                PathEl::MoveTo(p) => (verb::MOVE, std::slice::from_ref(p)),
                PathEl::LineTo(p) => (verb::LINE, std::slice::from_ref(p)),
                PathEl::QuadTo(a, b) => {
                    words.push(verb::QUAD);
                    floats.extend([f(a.x), f(a.y), f(b.x), f(b.y)]);
                    continue;
                }
                PathEl::CurveTo(a, b, c) => {
                    words.push(verb::CUBIC);
                    floats.extend([f(a.x), f(a.y), f(b.x), f(b.y), f(c.x), f(c.y)]);
                    continue;
                }
                PathEl::ClosePath => (verb::CLOSE, &[]),
            };
            words.push(code);
            for point in points {
                floats.extend([f(point.x), f(point.y)]);
            }
        }
        let verbs = u32::try_from(words.len()).map_err(|_| too_many("DefinePath", "verbs"))?;
        let points =
            u32::try_from(floats.len() / 2).map_err(|_| too_many("DefinePath", "points"))?;
        let rule = match rule {
            FillRule::NonZero => fill_type::WINDING,
            FillRule::EvenOdd => fill_type::EVEN_ODD,
        };
        self.buffer.op(
            Op::DefinePath,
            &[
                Field::U("path", id),
                Field::U("fill", rule),
                Field::U("verbs", verbs),
                Field::U("points", points),
                Field::Us("verb", words),
                Field::Fs("xy", floats),
            ],
        )?;
        Ok(id)
    }

    fn paint(
        &mut self,
        paint: &Paint,
        alpha: f32,
        owned: &mut Owned,
    ) -> Result<WirePaint, HwuiError> {
        self.paint_at(paint, Affine::IDENTITY, alpha, owned)
    }

    /// `paint` under the shader-local transform `local`.
    fn paint_at(
        &mut self,
        paint: &Paint,
        local: Affine,
        alpha: f32,
        owned: &mut Owned,
    ) -> Result<WirePaint, HwuiError> {
        let shader = match paint {
            Paint::Transformed(transformed) => {
                return self.paint_at(
                    &transformed.paint,
                    local * transformed.transform,
                    alpha,
                    owned,
                );
            }
            Paint::Solid(color) => return Ok(WirePaint::Color(color_long(*color, alpha))),
            Paint::Linear(linear) => {
                gradient_stops(
                    &linear.stops,
                    linear.interpolation,
                    &mut self.scratch.colors,
                    &mut self.scratch.floats,
                );
                let (start, end) = (linear.start, linear.end);
                let mut head = Fields::new();
                head.push(Field::F("x0", f(start.x)))
                    .push(Field::F("y0", f(start.y)))
                    .push(Field::F("x1", f(end.x)))
                    .push(Field::F("y1", f(end.y)))
                    .push(Field::U("tile", tile_code(linear.extend)));
                self.gradient_shader(shader_kind::LINEAR, &affine_matrix(local), &head)?
            }
            Paint::Radial(radial) => {
                gradient_stops(
                    &radial.stops,
                    radial.interpolation,
                    &mut self.scratch.colors,
                    &mut self.scratch.floats,
                );
                let mut head = Fields::new();
                head.push(Field::F("x0", f(radial.start_center.x)))
                    .push(Field::F("y0", f(radial.start_center.y)))
                    .push(Field::F("r0", f(radial.start_radius)))
                    .push(Field::F("x1", f(radial.end_center.x)))
                    .push(Field::F("y1", f(radial.end_center.y)))
                    .push(Field::F("r1", f(radial.end_radius)))
                    .push(Field::U("tile", tile_code(radial.extend)));
                self.gradient_shader(shader_kind::RADIAL, &affine_matrix(local), &head)?
            }
            Paint::Sweep(sweep) => {
                let span = sweep_span(sweep).ok_or_else(|| HwuiError::Encoding {
                    op: "DefineShader",
                    reason: format!(
                        "a sweep from {} to {} radians about {:?} has no finite span",
                        sweep.start_angle, sweep.end_angle, sweep.center
                    ),
                })?;
                gradient_stops(
                    &sweep.stops,
                    sweep.interpolation,
                    &mut self.scratch.colors,
                    &mut self.scratch.floats,
                );
                let scratch = &mut *self.scratch;
                let turned = sweep_turn(
                    std::f64::consts::TAU / span,
                    sweep.extend,
                    &scratch.colors,
                    &scratch.floats,
                    &mut scratch.turn_colors,
                    &mut scratch.turn_positions,
                );
                if let Err(count) = turned {
                    return Err(self.unsupported(format!(
                        "sweeps a gradient over {span} radians with {:?}: one turn of SweepGradient would carry {count} stops, past {MAX_SWEEP_STOPS}",
                        sweep.extend
                    )));
                }
                std::mem::swap(&mut scratch.colors, &mut scratch.turn_colors);
                std::mem::swap(&mut scratch.floats, &mut scratch.turn_positions);
                let matrix =
                    affine_matrix(local * Affine::rotate_about(sweep.start_angle, sweep.center));
                let mut head = Fields::new();
                head.push(Field::F("cx", f(sweep.center.x)))
                    .push(Field::F("cy", f(sweep.center.y)));
                self.gradient_shader(shader_kind::SWEEP, &matrix, &head)?
            }
            Paint::Image(pattern) => self.bitmap_shader(pattern, local)?,
            Paint::Shader(shader) => self.runtime_shader(shader, local)?,
            Paint::Mesh(_) => {
                return Err(HwuiError::Encoding {
                    op: "DefineShader",
                    reason: "a mesh gradient draws through its prepared node, never as a paint"
                        .to_owned(),
                });
            }
        };
        owned.shaders.push(shader);
        Ok(WirePaint::Shader(shader, alpha))
    }

    /// Defines a gradient shader from `head` and the scratch stops.
    fn gradient_shader(
        &mut self,
        kind: u32,
        matrix: &Matrix,
        head: &Fields<'_>,
    ) -> Result<u32, HwuiError> {
        let id = self.pools.shaders.acquire()?;
        let count = u32::try_from(self.scratch.colors.len())
            .map_err(|_| too_many("DefineShader", "stops"))?;
        let mut fields = Fields::new();
        fields
            .push(Field::U("shader", id))
            .push(Field::U("kind", kind))
            .push(Field::Fs("matrix", matrix))
            .extend(head.as_slice())
            .push(Field::U("stops", count))
            .push(Field::Colors("colors", &self.scratch.colors))
            .push(Field::Fs("positions", &self.scratch.floats));
        self.buffer.op(Op::DefineShader, fields.as_slice())?;
        Ok(id)
    }

    fn bitmap_shader(&mut self, pattern: &ImagePattern, local: Affine) -> Result<u32, HwuiError> {
        let bitmap = self
            .registry
            .lock()
            .resolve(Kind::Bitmap, pattern.image.raw(), self.layer)?;
        let id = self.pools.shaders.acquire()?;
        self.buffer.op(
            Op::DefineShader,
            &[
                Field::U("shader", id),
                Field::U("kind", shader_kind::BITMAP),
                Field::Fs("matrix", &affine_matrix(local * pattern.transform)),
                Field::U("bitmap", bitmap),
                Field::U("tile_x", tile_code(pattern.extend_x)),
                Field::U("tile_y", tile_code(pattern.extend_y)),
                Field::U("sampling", sampling_code(pattern.sampling)),
            ],
        )?;
        Ok(id)
    }

    fn runtime_shader(&mut self, shader: &ShaderPaint, local: Affine) -> Result<u32, HwuiError> {
        self.require("a runtime shader", RUNTIME_SHADER_API)?;
        let runtime =
            self.registry
                .lock()
                .resolve(Kind::RuntimeShader, shader.shader.raw(), self.layer)?;
        let id = self.pools.shaders.acquire()?;
        let count = u32::try_from(shader.uniforms.len())
            .map_err(|_| too_many("DefineShader", "uniforms"))?;
        self.buffer.op(
            Op::DefineShader,
            &[
                Field::U("shader", id),
                Field::U("kind", shader_kind::RUNTIME),
                Field::Fs("matrix", &affine_matrix(local)),
                Field::U("runtime", runtime),
                Field::U("uniforms", count),
                Field::Fs("values", &shader.uniforms),
            ],
        )?;
        Ok(id)
    }

    fn fill(
        &mut self,
        shape: &ShapeData,
        paint: &Paint,
        alpha: f32,
        owned: &mut Owned,
    ) -> Result<(), HwuiError> {
        let Some(wire) = self.wire(shape, false, owned)? else {
            return Ok(());
        };
        let paint = self.paint(paint, alpha, owned)?;
        let mut fields = Fields::new();
        wire.fields(&mut fields);
        paint.fields(&mut fields);
        self.buffer.op(Op::Fill, fields.as_slice())
    }

    fn stroke(
        &mut self,
        shape: &ShapeData,
        stroke: &Stroke,
        paint: &Paint,
        alpha: f32,
        owned: &mut Owned,
    ) -> Result<(), HwuiError> {
        if stroke.width <= 0.0 {
            // A zero-width stroke covers nothing; Android would draw a
            // hairline instead.
            return Ok(());
        }
        let cap = self.cap(stroke)?;
        let Some(wire) = self.wire(shape, true, owned)? else {
            return Ok(());
        };
        let paint = self.paint(paint, alpha, owned)?;
        let intervals = &mut self.scratch.intervals;
        intervals.clear();
        dash_intervals(stroke, intervals);
        let dashes = u32::try_from(intervals.len()).map_err(|_| too_many("Stroke", "dashes"))?;
        let mut fields = Fields::new();
        wire.fields(&mut fields);
        paint.fields(&mut fields);
        fields
            .push(Field::F("width", f(stroke.width)))
            .push(Field::U("cap", cap))
            .push(Field::U("join", join_code(stroke.join)))
            .push(Field::F("miter", f(stroke.miter_limit)))
            .push(Field::F("phase", f(stroke.dash_offset)))
            .push(Field::U("dashes", dashes))
            .push(Field::Fs("intervals", &self.scratch.intervals));
        self.buffer.op(Op::Stroke, fields.as_slice())
    }

    fn cap(&self, stroke: &Stroke) -> Result<u32, HwuiError> {
        if stroke.start_cap != stroke.end_cap {
            return Err(self.unsupported(format!(
                "strokes with a {:?} start cap and a {:?} end cap; Paint has one cap",
                stroke.start_cap, stroke.end_cap
            )));
        }
        Ok(match stroke.start_cap {
            Cap::Butt => cap::BUTT,
            Cap::Round => cap::ROUND,
            Cap::Square => cap::SQUARE,
        })
    }

    fn shadow(
        &mut self,
        shape: &ShapeData,
        shadow: &Shadow,
        alpha: f32,
        owned: &mut Owned,
    ) -> Result<(), HwuiError> {
        let spread = shadow.spread;
        // A primitive grows exactly into another primitive; a path grows by
        // stroking its outline 2·spread wide with round joins, which is
        // exactly the set within `spread` of it.
        let (grown, outline) = match (grow(shape, spread), shape) {
            (Some(grown), _) => (grown, 0.0),
            (None, ShapeData::Line(_)) => {
                return Err(self.unsupported(format!(
                    "spreads the shadow of a line by {spread}; a line encloses no area to grow or shrink"
                )));
            }
            (None, _) if spread > 0.0 => (shape.clone(), spread),
            (None, _) => {
                return Err(self.unsupported(format!(
                    "shrinks the shadow of a path by a spread of {spread}; FILL_AND_STROKE only grows a path"
                )));
            }
        };
        let Some(wire) = self.wire(&grown, false, owned)? else {
            return Ok(());
        };
        let radius = blur_radius(shadow.sigma);
        let mut fields = Fields::new();
        wire.fields(&mut fields);
        fields
            .push(Field::Color("color", color_long(shadow.color, alpha)))
            .push(Field::F("radius", f(radius)))
            .push(Field::F("dx", f(shadow.offset.x)))
            .push(Field::F("dy", f(shadow.offset.y)))
            .push(Field::F("spread", f(outline)));
        self.buffer.op(Op::Shadow, fields.as_slice())
    }

    fn glyphs(
        &mut self,
        run: &GlyphRun,
        paint: &Paint,
        alpha: f32,
        owned: &mut Owned,
    ) -> Result<(), HwuiError> {
        let font = self.registry.lock().glyph_font(
            run.font.raw(),
            &run.coords,
            self.layer,
            self.buffer,
        )?;
        let intervals = &mut self.scratch.intervals;
        intervals.clear();
        let (style, stroke_fields) = match &run.style {
            GlyphStyle::Fill => (glyph_style::FILL, [0.0, 0.0, 0.0]),
            GlyphStyle::Stroke(stroke) => {
                // `DashPathEffect` applies to glyph outlines as to paths.
                dash_intervals(stroke, intervals);
                (
                    glyph_style::STROKE,
                    [
                        f(stroke.width),
                        f(stroke.miter_limit),
                        f(stroke.dash_offset),
                    ],
                )
            }
        };
        let (cap, join) = match &run.style {
            GlyphStyle::Fill => (cap::BUTT, join::MITER),
            GlyphStyle::Stroke(stroke) => (self.cap(stroke)?, join_code(stroke.join)),
        };
        let paint = self.paint(paint, alpha, owned)?;
        let mut start = 0;
        let glyphs = &run.glyphs;
        while start < glyphs.len() {
            if let Some(transform) = glyphs[start].transform {
                let glyph = glyphs[start];
                let origin = Vec2::new(f64::from(glyph.x), f64::from(glyph.y));
                let about = Affine::translate(origin) * transform * Affine::translate(-origin);
                self.buffer.op(Op::Save, &[])?;
                self.concat(&affine_matrix(about))?;
                self.glyph_batch(
                    font,
                    run.size,
                    paint,
                    style,
                    stroke_fields,
                    cap,
                    join,
                    start..start + 1,
                    run,
                )?;
                self.buffer.op(Op::Restore, &[])?;
                start += 1;
                continue;
            }
            let end = glyphs[start..]
                .iter()
                .position(|glyph| glyph.transform.is_some())
                .map_or(glyphs.len(), |offset| start + offset);
            self.glyph_batch(
                font,
                run.size,
                paint,
                style,
                stroke_fields,
                cap,
                join,
                start..end,
                run,
            )?;
            start = end;
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one Glyphs record's fields, split out of the run walk"
    )]
    fn glyph_batch(
        &mut self,
        font: u32,
        size: f32,
        paint: WirePaint,
        style: u32,
        [width, miter, phase]: [f32; 3],
        cap: u32,
        join: u32,
        range: Range<usize>,
        run: &GlyphRun,
    ) -> Result<(), HwuiError> {
        let Scratch {
            words,
            floats,
            intervals,
            ..
        } = &mut *self.scratch;
        words.clear();
        floats.clear();
        for glyph in &run.glyphs[range] {
            words.push(glyph.id);
            floats.extend([glyph.x, glyph.y]);
        }
        let count = u32::try_from(words.len()).map_err(|_| too_many("Glyphs", "glyphs"))?;
        let dashes = u32::try_from(intervals.len()).map_err(|_| too_many("Glyphs", "dashes"))?;
        let mut fields = Fields::new();
        fields
            .push(Field::U("font", font))
            .push(Field::F("size", size));
        paint.fields(&mut fields);
        fields
            .push(Field::U("style", style))
            .push(Field::F("width", width))
            .push(Field::U("cap", cap))
            .push(Field::U("join", join))
            .push(Field::F("miter", miter))
            .push(Field::F("phase", phase))
            .push(Field::U("dashes", dashes))
            .push(Field::Fs("intervals", intervals))
            .push(Field::U("count", count))
            .push(Field::Us("ids", words))
            .push(Field::Fs("xy", floats));
        self.buffer.op(Op::Glyphs, fields.as_slice())
    }

    /// Draws `mesh` under `local` into the open recording, one `Mesh` op
    /// per band of at most [`MESH_BAND_PATCHES`] patches in row-major
    /// order. Each patch carries its corners and premultiplied colours;
    /// the replayer's fragment shader inverts the bilinear patch.
    fn mesh(&mut self, mesh: &MeshGradient, local: Affine) -> Result<(), HwuiError> {
        let columns = mesh.columns() as usize;
        let stride = columns + 1;
        let total = columns * mesh.rows() as usize;
        let corners_of = |patch: usize| {
            let corner = (patch / columns) * stride + patch % columns;
            [corner, corner + 1, corner + stride, corner + stride + 1]
        };
        if let Some(patch) =
            (0..total).find(|&patch| folds(corners_of(patch).map(|i| mesh.points()[i])))
        {
            return Err(self.unsupported(format!(
                "fills a mesh gradient whose patch at row {}, column {} folds; HWUI covers a patch with its quad's two triangles",
                patch / columns,
                patch % columns
            )));
        }
        let interpolation = match mesh.interpolation_mode() {
            MeshColorInterpolation::Linear => mesh_interpolation::LINEAR,
            MeshColorInterpolation::Smoothstep => mesh_interpolation::SMOOTHSTEP,
        };
        let transformed = !is_identity(local);
        if transformed {
            self.buffer.op(Op::Save, &[])?;
            self.concat(&affine_matrix(local))?;
        }
        let band = MESH_BAND_PATCHES as usize;
        for start in (0..total).step_by(band) {
            let end = (start + band).min(total);
            let floats = &mut self.scratch.floats;
            floats.clear();
            floats.reserve((end - start) * MESH_PATCH_FLOATS as usize);
            for patch in start..end {
                let corners = corners_of(patch);
                for index in corners {
                    let point = mesh.points()[index];
                    floats.extend([f(point.x), f(point.y)]);
                }
                for index in corners {
                    floats.extend(premultiplied_linear_srgb(mesh.colors()[index]));
                }
            }
            let patches = u32::try_from(end - start).map_err(|_| too_many("Mesh", "patches"))?;
            self.buffer.op(
                Op::Mesh,
                &[
                    Field::U("interpolation", interpolation),
                    Field::U("patches", patches),
                    Field::Fs("patch", &self.scratch.floats),
                ],
            )?;
        }
        if transformed {
            self.buffer.op(Op::Restore, &[])?;
        }
        Ok(())
    }
}

/// The paint a draw command carries.
const fn command_paint(command: &Command) -> Option<&Paint> {
    match command {
        Command::Fill { paint, .. }
        | Command::Stroke { paint, .. }
        | Command::Glyphs { paint, .. } => Some(paint),
        _ => None,
    }
}

/// The mesh gradient `paint` draws, under its accumulated local transform.
fn mesh_of(paint: &Paint) -> Option<(&MeshGradient, Affine)> {
    let mut inner = paint;
    let mut local = Affine::IDENTITY;
    while let Paint::Transformed(transformed) = inner {
        local *= transformed.transform;
        inner = &transformed.paint;
    }
    match inner {
        Paint::Mesh(mesh) => Some((mesh, local)),
        _ => None,
    }
}

/// Whether the patch with corners 00, 10, 01, 11 is not a convex quad:
/// its outline turns both ways, so its bilinear image leaves the quad.
fn folds([origin, right, bottom, opposite]: [Point; 4]) -> bool {
    let ring = [origin, right, opposite, bottom];
    let mut turns = (false, false);
    for at in 0..4 {
        let [a, b, c] = [ring[at], ring[(at + 1) % 4], ring[(at + 2) % 4]];
        let turn = (b - a).cross(c - b);
        turns.0 |= turn > 0.0;
        turns.1 |= turn < 0.0;
    }
    turns.0 && turns.1
}

/// The span a sweep covers, as Cherenkov's oracle reads it: an end at or
/// before the start wraps forward by whole turns, and a whole-turn
/// difference is one turn. `None` when it is not finite at f32.
fn sweep_span(sweep: &SweepGradient) -> Option<f64> {
    let raw = sweep.end_angle - sweep.start_angle;
    if !raw.is_finite() || !sweep.start_angle.is_finite() || !sweep.center.is_finite() {
        return None;
    }
    let span = if raw > 0.0 {
        raw
    } else {
        let wrapped = raw.rem_euclid(std::f64::consts::TAU);
        if wrapped == 0.0 {
            std::f64::consts::TAU
        } else {
            wrapped
        }
    };
    (f(span).is_finite() && f(span) != 0.0).then_some(span)
}

/// A shape grown by `spread`, or `None` for a path, which has no exact
/// offset.
fn grow(shape: &ShapeData, spread: f64) -> Option<ShapeData> {
    use waterui_graphics::draw::ContinuousRect;
    use waterui_graphics::draw::kurbo::{Circle, Ellipse, RoundedRect, RoundedRectRadii};
    if spread == 0.0 {
        return Some(shape.clone());
    }
    let radii = |r: RoundedRectRadii| {
        RoundedRectRadii::new(
            (r.top_left + spread).max(0.0),
            (r.top_right + spread).max(0.0),
            (r.bottom_right + spread).max(0.0),
            (r.bottom_left + spread).max(0.0),
        )
    };
    Some(match shape {
        ShapeData::Rect(rect) => ShapeData::Rect(rect.inflate(spread, spread)),
        ShapeData::RoundedRect(rounded) => ShapeData::RoundedRect(RoundedRect::from_rect(
            rounded.rect().inflate(spread, spread),
            radii(rounded.radii()),
        )),
        ShapeData::Continuous(continuous) => ShapeData::Continuous(ContinuousRect {
            rect: continuous.rect.inflate(spread, spread),
            radii: radii(continuous.radii),
            smoothing: continuous.smoothing,
        }),
        ShapeData::Circle(circle) => ShapeData::Circle(Circle::new(
            circle.center,
            (circle.radius + spread).max(0.0),
        )),
        ShapeData::Ellipse(ellipse) => {
            let (r, rotation) = ellipse.radii_and_rotation();
            ShapeData::Ellipse(Ellipse::new(
                ellipse.center(),
                ((r.x + spread).max(0.0), (r.y + spread).max(0.0)),
                rotation,
            ))
        }
        ShapeData::Line(_) | ShapeData::Path { .. } => return None,
    })
}

fn shape_path(shape: &ShapeData) -> BezPath {
    match shape {
        ShapeData::Rect(rect) => rect.to_path(PATH_TOLERANCE),
        ShapeData::RoundedRect(rounded) => rounded.to_path(PATH_TOLERANCE),
        ShapeData::Continuous(continuous) => continuous.to_path(PATH_TOLERANCE),
        ShapeData::Circle(circle) => circle.to_path(PATH_TOLERANCE),
        ShapeData::Ellipse(ellipse) => ellipse.to_path(PATH_TOLERANCE),
        ShapeData::Line(line) => line.to_path(PATH_TOLERANCE),
        ShapeData::Path { elements, .. } => BezPath::from_vec(elements.to_vec()),
    }
}

/// The `BlurMaskFilter` radius of a Gaussian `sigma`: Skia's
/// `sigma = 0.57735 * radius + 0.5`, inverted. A sigma at or below 0.5
/// (sub-pixel) takes the smallest positive radius, since radius 0 means no
/// blur at all; zero sigma is a hard shadow.
fn blur_radius(sigma: f64) -> f64 {
    if sigma <= 0.0 {
        0.0
    } else {
        ((sigma - 0.5) / 0.577_35).max(1e-3)
    }
}

fn too_many(op: &'static str, what: &str) -> HwuiError {
    HwuiError::Encoding {
        op,
        reason: format!("its {what} exceed u32"),
    }
}

const fn tile_code(extend: Extend) -> u32 {
    match extend {
        Extend::Pad => tile::CLAMP,
        Extend::Repeat => tile::REPEAT,
        Extend::Reflect => tile::MIRROR,
        Extend::None => tile::DECAL,
    }
}

const fn sampling_code(sampling: Sampling) -> u32 {
    match sampling {
        Sampling::Nearest => sampling_kind::NEAREST,
        Sampling::Linear => sampling_kind::LINEAR,
    }
}

const fn join_code(join: Join) -> u32 {
    match join {
        Join::Miter => join::MITER,
        Join::Round => join::ROUND,
        Join::Bevel => join::BEVEL,
    }
}

/// The integer origin and size of a node covering `ink`.
#[must_use]
pub fn extent(ink: Option<Rect>) -> ([i32; 2], [i32; 2]) {
    let Some(ink) = ink else {
        return ([0, 0], [0, 0]);
    };
    let origin = [floor_i32(ink.x0), floor_i32(ink.y0)];
    let size = [
        (ceil_i32(ink.x1) - origin[0]).max(0),
        (ceil_i32(ink.y1) - origin[1]).max(0),
    ];
    (origin, size)
}

/// `SetPosition` bounds of a node at `origin` of `size`.
#[must_use]
pub const fn bounds(origin: [i32; 2], size: [i32; 2]) -> [i32; 4] {
    [
        origin[0],
        origin[1],
        origin[0] + size[0],
        origin[1] + size[1],
    ]
}

/// `stroke`'s dash pattern as `DashPathEffect` intervals: an odd pattern
/// repeats to an even one, as `DashPathEffect` requires.
fn dash_intervals(stroke: &Stroke, intervals: &mut Vec<f32>) {
    intervals.extend(stroke.dash_pattern.iter().map(|dash| f(*dash)));
    if intervals.len() % 2 == 1 {
        intervals.extend_from_within(..);
    }
}

/// How far a stroke's ink reaches past its path: half the width, times the
/// miter limit at a miter join.
fn stroke_reach(stroke: &Stroke) -> f64 {
    let reach = if stroke.join == Join::Miter {
        stroke.miter_limit.max(std::f64::consts::SQRT_2)
    } else {
        std::f64::consts::SQRT_2
    };
    stroke.width * 0.5 * reach
}

/// A conservative ink box of `range` of `list`, in the list's space. A text
/// layout or font the platform does not hold adds nothing; lowering it
/// fails.
#[must_use]
pub fn content_ink(
    list: &DisplayList,
    range: Range<usize>,
    text_layouts: &TextLayoutIds,
    registry: &Registry,
) -> Option<Rect> {
    let commands = list.commands();
    let mut transform = Affine::IDENTITY;
    let mut clip: Option<Rect> = None;
    let mut scopes: Vec<(Affine, Option<Rect>)> = Vec::new();
    let mut ink: Option<Rect> = None;
    let mut add = |rect: Rect, transform: Affine, clip: Option<Rect>| {
        let rect = transform.transform_rect_bbox(rect);
        let rect = clip.map_or(Some(rect), |clip| intersect(Some(rect), clip));
        ink = union(ink, rect);
    };
    for command in &commands[range] {
        match command {
            Command::Fill { shape, .. } => add(shape.bounds(), transform, clip),
            Command::Stroke { shape, stroke, .. } => {
                let reach = stroke_reach(stroke);
                add(shape.bounds().inflate(reach, reach), transform, clip);
            }
            Command::Shadow { shape, shadow } => {
                let reach = 3.0f64.mul_add(shadow.sigma, shadow.spread.max(0.0));
                add(
                    (shape.bounds() + shadow.offset).inflate(reach, reach),
                    transform,
                    clip,
                );
            }
            Command::Glyphs { run, .. } => {
                // The font's own bounding box (`head`), which bounds every
                // glyph of its default instance, y up in ems.
                let Some([x_min, y_min, x_max, y_max]) = registry.font_bounds(run.font.raw())
                else {
                    continue;
                };
                let size = f64::from(run.size);
                let stroke = match &run.style {
                    GlyphStyle::Stroke(stroke) => stroke_reach(stroke),
                    GlyphStyle::Fill => 0.0,
                };
                for glyph in run.glyphs.iter() {
                    let origin = Point::new(f64::from(glyph.x), f64::from(glyph.y));
                    let rect = Rect::new(
                        f64::from(x_min).mul_add(size, origin.x),
                        f64::from(y_max).mul_add(-size, origin.y),
                        f64::from(x_max).mul_add(size, origin.x),
                        f64::from(y_min).mul_add(-size, origin.y),
                    )
                    .inflate(stroke, stroke);
                    let local = glyph.transform.map_or(Affine::IDENTITY, |t| {
                        Affine::translate(origin.to_vec2())
                            * t
                            * Affine::translate(-origin.to_vec2())
                    });
                    add(rect, transform * local, clip);
                }
            }
            Command::Image { dst, .. } => add(*dst, transform, clip),
            Command::Text {
                layout,
                transform: placed,
            } => {
                if let Some(bounds) = text_layouts.bounds(layout.raw()) {
                    add(bounds, transform * *placed, clip);
                }
            }
            Command::Picture {
                picture,
                transform: placed,
            } => {
                let list = picture.display_list();
                if let Some(inner) =
                    content_ink(list, 0..list.commands().len(), text_layouts, registry)
                {
                    add(inner, transform * *placed, clip);
                }
            }
            Command::BeginClip { shape, .. } => {
                scopes.push((transform, clip));
                let bounds = transform.transform_rect_bbox(shape.bounds());
                clip = Some(clip.map_or(bounds, |clip| clip.intersect(bounds)));
            }
            Command::BeginTransform {
                transform: inner, ..
            } => {
                scopes.push((transform, clip));
                transform *= *inner;
            }
            Command::BeginGroup { .. } => scopes.push((transform, clip)),
            Command::End => {
                if let Some((t, c)) = scopes.pop() {
                    transform = t;
                    clip = c;
                }
            }
        }
    }
    ink
}

/// A colour's `ColorLong` for the clear: opaque components as given.
#[must_use]
pub fn clear_color(color: WorkingColor) -> u64 {
    color_long(color, 1.0)
}

/// `SetPosition`.
pub fn set_position(
    buffer: &mut CommandBuffer,
    node: u32,
    [left, top, right, bottom]: [i32; 4],
    clip: bool,
) -> Result<(), HwuiError> {
    buffer.op(
        Op::SetPosition,
        &[
            Field::U("node", node),
            Field::I("left", left),
            Field::I("top", top),
            Field::I("right", right),
            Field::I("bottom", bottom),
            Field::U("clip", u32::from(clip)),
        ],
    )
}
