//! The lowering's decisions, as plain data: which Core Animation layer a
//! recorded command becomes, and which of its operands a layer property
//! carries.
//!
//! Nothing here touches the platform, so the recording-to-layer-tree
//! translation is testable by value: [`outline`] describes the tree a
//! display list lowers to, and the realization in [`super::layers`]
//! builds exactly that.
//!
//! # Group compositing
//!
//! A group composites onto what lies beneath it in its level — the
//! enclosing group, or the canvas. A Core Animation compositing filter
//! blends a layer with everything behind it in the window instead, so it
//! serves only the modes whose result does not depend on the backdrop's
//! coverage being the canvas's: the separable blend modes and plus-lighter.
//! Every other mode — the Porter-Duff operators, and hue, saturation,
//! colour and luminosity, which Core Animation has no filter for — is drawn
//! by Core Graphics together with the content beneath it in its level, as
//! one layer ([`drawn_run`], drawn by [`super::replay`]). Either way a
//! group blends in an encoded space whatever its recorded `blend_space`:
//! Core Animation in the window's, Core Graphics in the extended sRGB the
//! run is drawn in. That is the deviation the canvas design states for
//! Apple, the same as Android's HWUI (water-rs/waterui#1560, "layer blend
//! modes").

use core::ops::Range;

use kurbo::{Affine, Stroke};
use waterui_graphics::backend::draw::{
    BlendMode, Command, GlyphRun, GlyphStyle, Group, ImageId, MeshGradient, Paint, Picture,
    Sampling, Shadow, ShapeData, TextLayoutId, WorkingColor,
};

/// How a paint reaches the screen.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintClass<'a> {
    /// A colour: the colour property of a `CAShapeLayer`.
    Solid(WorkingColor),
    /// A mesh gradient: the Metal pass, under a mask of the shape.
    Mesh {
        /// The mesh.
        mesh: &'a MeshGradient,
        /// Maps the mesh's coordinates into the shape's space.
        transform: Affine,
    },
    /// A gradient or an image pattern: drawn by Core Graphics into a layer
    /// that the shape masks.
    Drawn,
}

/// Classifies `paint`. A shader paint, which only an engine surface draws,
/// classifies as drawn: Core Graphics reports it and paints nothing.
#[must_use]
pub fn classify(paint: &Paint) -> PaintClass<'_> {
    match paint {
        Paint::Solid(color) => PaintClass::Solid(*color),
        Paint::Mesh(mesh) => PaintClass::Mesh {
            mesh,
            transform: Affine::IDENTITY,
        },
        Paint::Transformed(transformed) => match classify(&transformed.paint) {
            // Geometry and coverage are the shape's: a colour has no
            // coordinates for the transform to move.
            solid @ PaintClass::Solid(_) => solid,
            PaintClass::Mesh { mesh, transform } => PaintClass::Mesh {
                mesh,
                transform: transformed.transform * transform,
            },
            PaintClass::Drawn => PaintClass::Drawn,
        },
        Paint::Linear(_)
        | Paint::Radial(_)
        | Paint::Sweep(_)
        | Paint::Image(_)
        | Paint::Shader(_) => PaintClass::Drawn,
    }
}

/// `mesh`'s vertex grid as the mesh pass and the CPU rasterization take it:
/// points in the mesh's space and straight linear Display P3 colours.
#[must_use]
pub fn grid(mesh: &MeshGradient) -> (Vec<[f64; 2]>, Vec<[f32; 4]>) {
    (
        mesh.points().iter().map(|p| [p.x, p.y]).collect(),
        mesh.colors().iter().map(|c| c.components).collect(),
    )
}

/// What a fill or stroke covers.
#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    /// A recorded shape.
    Shape(ShapeData),
    /// The outlines of a glyph run.
    Glyphs(GlyphRun),
}

/// What one recorded command (or scope opener) realizes as.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// A fill or stroke of a shape or glyph run.
    Painted {
        /// What is covered.
        geometry: Geometry,
        /// `Some` strokes the geometry, `None` fills it.
        stroke: Option<Stroke>,
        /// What covers it.
        paint: Paint,
    },
    /// An image drawn into a rectangle.
    Image {
        /// The image.
        image: ImageId,
        /// Where it lands.
        dst: kurbo::Rect,
        /// How it is sampled.
        sampling: Sampling,
    },
    /// A shadow of a shape.
    Shadow {
        /// The casting shape.
        shape: ShapeData,
        /// The shadow.
        shadow: Shadow,
    },
    /// A clip around the commands up to the matching end.
    Clip(ShapeData),
    /// A transform around them.
    Transform(Affine),
    /// An isolated group around them.
    Group(Group),
    /// A shared picture drawn under a transform.
    Picture {
        /// The picture's commands.
        picture: Picture,
        /// Where it is placed.
        transform: Affine,
    },
    /// A registered text layout drawn under a transform: a picture the
    /// registry holds, which the layout's registration keeps.
    Text {
        /// The layout.
        layout: TextLayoutId,
        /// Where it is placed.
        transform: Affine,
        /// What the colour the text draws in by default becomes.
        paint: Option<Paint>,
    },
}

/// The node `command` realizes as; `None` for the end of a scope.
#[must_use]
pub fn describe(command: &Command) -> Option<Node> {
    Some(match command {
        Command::Fill { shape, paint } => Node::Painted {
            geometry: Geometry::Shape(shape.clone()),
            stroke: None,
            paint: paint.clone(),
        },
        Command::Stroke {
            shape,
            stroke,
            paint,
        } => Node::Painted {
            geometry: Geometry::Shape(shape.clone()),
            stroke: Some(stroke.clone()),
            paint: paint.clone(),
        },
        Command::Glyphs { run, paint } => Node::Painted {
            geometry: Geometry::Glyphs(run.clone()),
            stroke: match &run.style {
                GlyphStyle::Fill => None,
                GlyphStyle::Stroke(stroke) => Some(stroke.clone()),
            },
            paint: paint.clone(),
        },
        Command::Shadow { shape, shadow } => Node::Shadow {
            shape: shape.clone(),
            shadow: *shadow,
        },
        Command::Image {
            image,
            dst,
            sampling,
        } => Node::Image {
            image: *image,
            dst: *dst,
            sampling: *sampling,
        },
        Command::Picture { picture, transform } => Node::Picture {
            picture: picture.clone(),
            transform: *transform,
        },
        Command::Text {
            layout,
            transform,
            paint,
        } => Node::Text {
            layout: *layout,
            transform: *transform,
            paint: paint.clone(),
        },
        Command::BeginClip { shape, .. } => Node::Clip(shape.clone()),
        Command::BeginTransform { transform, .. } => Node::Transform(*transform),
        Command::BeginGroup { group, .. } => Node::Group(*group),
        Command::End => return None,
    })
}

/// Reports a filter on `group`: a canvas group records none, so the
/// filter is not applied.
pub fn reject_group_filter(group: &Group) {
    if group.filter.is_some() {
        waterui::contract_violation!(
            "a canvas group records no filter; got {:?}; the filter is not applied",
            group.filter
        );
    }
}

/// How a group with some blend mode reaches the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Composite {
    /// Source over: a plain layer.
    Normal,
    /// A layer whose Core Animation compositing filter is the mode.
    Filter(&'static str),
    /// Drawn by Core Graphics together with what lies beneath it in its
    /// level: the mode composites against the canvas's own content.
    Backdrop,
    /// Nothing of the group shows: `Dst` keeps the destination.
    Hidden,
}

/// How a group blended with `mode` is realized; see the module docs.
///
/// The filter names are the ones Core Animation evaluates to the same
/// result as the Core Graphics blend mode of the same name; Core Animation
/// recognizes no filter for hue, saturation, colour and luminosity.
#[must_use]
pub const fn composite(mode: BlendMode) -> Composite {
    Composite::Filter(match mode {
        BlendMode::Normal => return Composite::Normal,
        BlendMode::Dst => return Composite::Hidden,
        BlendMode::Multiply => "multiplyBlendMode",
        BlendMode::Screen => "screenBlendMode",
        BlendMode::Overlay => "overlayBlendMode",
        BlendMode::Darken => "darkenBlendMode",
        BlendMode::Lighten => "lightenBlendMode",
        BlendMode::ColorDodge => "colorDodgeBlendMode",
        BlendMode::ColorBurn => "colorBurnBlendMode",
        BlendMode::HardLight => "hardLightBlendMode",
        BlendMode::SoftLight => "softLightBlendMode",
        BlendMode::Difference => "differenceBlendMode",
        BlendMode::Exclusion => "exclusionBlendMode",
        BlendMode::PlusLighter => "plusL",
        BlendMode::Hue
        | BlendMode::Saturation
        | BlendMode::Color
        | BlendMode::Luminosity
        | BlendMode::Clear
        | BlendMode::Src
        | BlendMode::SrcIn
        | BlendMode::SrcOut
        | BlendMode::SrcAtop
        | BlendMode::DestOver
        | BlendMode::DestIn
        | BlendMode::DestOut
        | BlendMode::DestAtop
        | BlendMode::Xor => return Composite::Backdrop,
    })
}

/// Whether the scope opened at `scope` — the canvas for `None` — is a
/// level: what a group directly inside it composites onto.
#[must_use]
pub const fn is_level(commands: &[Command], scope: Option<usize>) -> bool {
    match scope {
        None => true,
        Some(open) => matches!(commands[open], Command::BeginGroup { .. }),
    }
}

/// Whether the command at `index` composites a group against the content
/// beneath it in the level it sits in: such a group itself, or a clip,
/// transform or picture holding one outside any nested group.
#[must_use]
pub fn holds_backdrop(commands: &[Command], index: usize) -> bool {
    match &commands[index] {
        Command::BeginGroup { group, .. } => composite(group.blend) == Composite::Backdrop,
        Command::BeginClip { .. } | Command::BeginTransform { .. } => {
            children(commands, Some(index))
                .into_iter()
                .any(|child| holds_backdrop(commands, child))
        }
        Command::Picture { picture, .. } => {
            let inner = picture.display_list().commands();
            children(inner, None)
                .into_iter()
                .any(|child| holds_backdrop(inner, child))
        }
        _ => false,
    }
}

/// The commands of the level `scope` that Core Graphics draws as one
/// layer: its direct children from the first through the last that holds
/// a backdrop-compositing group (whose scope the range includes); `None`
/// when every child has a layer of its own. Later children stay layers
/// over it, since they composite source-over onto the result.
#[must_use]
pub fn drawn_run(commands: &[Command], scope: Option<usize>) -> Option<Range<usize>> {
    debug_assert!(is_level(commands, scope), "only a level holds a run");
    let direct = children(commands, scope);
    let last = *direct
        .iter()
        .rev()
        .find(|&&child| holds_backdrop(commands, child))?;
    let end = scope_end(&commands[last]).map_or(last + 1, |end| end + 1);
    Some(direct[0]..end)
}

/// Every run [`drawn_run`] finds in `commands`, outside other runs and
/// shared pictures (whose own trees plan theirs), in command order.
#[must_use]
pub fn runs(commands: &[Command]) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    collect_runs(commands, None, &mut found);
    found.sort_by_key(|run| run.start);
    found
}

fn collect_runs(commands: &[Command], scope: Option<usize>, found: &mut Vec<Range<usize>>) {
    let run = if is_level(commands, scope) {
        drawn_run(commands, scope)
    } else {
        None
    };
    for child in children(commands, scope) {
        if run.as_ref().is_some_and(|run| run.contains(&child)) {
            continue;
        }
        if scope_end(&commands[child]).is_some() {
            collect_runs(commands, Some(child), found);
        }
    }
    found.extend(run);
}

/// The layer kind a node lowers to — the layer-tree description tests
/// compare.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub enum LayerKind {
    /// A `CAShapeLayer`: a colour fill or stroke of a path.
    Shape,
    /// A Core Graphics drawn layer under a `CAShapeLayer` mask.
    Paint,
    /// The Metal mesh layer under a `CAShapeLayer` mask.
    Mesh,
    /// A `CALayer` whose contents are the image.
    Image,
    /// A `CALayer` casting a shadow from a path and drawing nothing else.
    Shadow,
    /// A layer clipping to its bounds: a rectangle clip.
    ClipBounds,
    /// A layer under a `CAShapeLayer` mask: any other clip.
    ClipMask,
    /// A layer carrying an affine transform.
    Transform,
    /// A layer carrying group opacity and how it composites.
    Group {
        /// The group's opacity.
        opacity: f32,
        /// How it composites; never [`Composite::Backdrop`], which a run
        /// draws.
        composite: Composite,
    },
    /// A layer carrying a transform over the picture's own layers.
    Picture,
    /// A layer whose contents Core Graphics draws from a run of commands
    /// ([`drawn_run`]).
    Drawn,
}

/// One layer of the lowered tree, with the layers of the commands inside
/// its scope.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    /// What the layer is.
    pub kind: LayerKind,
    /// The layers inside it, bottom first.
    pub children: Vec<Self>,
}

/// The layer kind `node` lowers to.
#[cfg(test)]
#[must_use]
pub fn kind_of(node: &Node) -> LayerKind {
    match node {
        Node::Painted { paint, .. } => match classify(paint) {
            PaintClass::Solid(_) => LayerKind::Shape,
            PaintClass::Mesh { .. } => LayerKind::Mesh,
            PaintClass::Drawn => LayerKind::Paint,
        },
        Node::Image { .. } => LayerKind::Image,
        Node::Shadow { .. } => LayerKind::Shadow,
        Node::Clip(ShapeData::Rect(_)) => LayerKind::ClipBounds,
        Node::Clip(_) => LayerKind::ClipMask,
        Node::Transform(_) => LayerKind::Transform,
        Node::Group(group) => LayerKind::Group {
            opacity: group.opacity,
            composite: composite(group.blend),
        },
        Node::Picture { .. } | Node::Text { .. } => LayerKind::Picture,
    }
}

/// The index of the `End` that closes the scope `command` opens.
#[must_use]
pub const fn scope_end(command: &Command) -> Option<usize> {
    match command {
        Command::BeginClip { end, .. }
        | Command::BeginTransform { end, .. }
        | Command::BeginGroup { end, .. } => Some(*end as usize),
        _ => None,
    }
}

/// The indices of the commands directly inside the scope opened at
/// `scope` — or at the top level for `None` — each of which has a layer of
/// its own there. The scope's `End` and everything deeper are skipped.
#[must_use]
pub fn children(commands: &[Command], scope: Option<usize>) -> Vec<usize> {
    let (mut index, stop) = scope.map_or((0, commands.len()), |open| {
        (
            open + 1,
            scope_end(&commands[open]).expect("a scope opener closes its scope"),
        )
    });
    let mut direct = Vec::new();
    while index < stop {
        direct.push(index);
        index = scope_end(&commands[index]).map_or(index + 1, |end| end + 1);
    }
    direct
}

/// The layer tree `list` lowers to.
#[cfg(test)]
#[must_use]
pub fn outline(list: &waterui_graphics::backend::draw::DisplayList) -> Vec<Outline> {
    outline_scope(list.commands(), None)
}

#[cfg(test)]
fn outline_scope(commands: &[Command], scope: Option<usize>) -> Vec<Outline> {
    let run = if is_level(commands, scope) {
        drawn_run(commands, scope)
    } else {
        None
    };
    let drawn = run.as_ref().map(|_| Outline {
        kind: LayerKind::Drawn,
        children: Vec::new(),
    });
    drawn
        .into_iter()
        .chain(
            children(commands, scope)
                .into_iter()
                .filter(|index| !run.as_ref().is_some_and(|run| run.contains(index)))
                .map(|index| outline_node(commands, index)),
        )
        .collect()
}

#[cfg(test)]
fn outline_node(commands: &[Command], index: usize) -> Outline {
    let node = describe(&commands[index]).expect("only a scope's End has no node");
    let children = match &node {
        Node::Picture { picture, .. } => outline(picture.display_list()),
        _ if scope_end(&commands[index]).is_some() => outline_scope(commands, Some(index)),
        _ => Vec::new(),
    };
    Outline {
        kind: kind_of(&node),
        children,
    }
}
