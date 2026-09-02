//! Mermaid source in, [`DiagramLayout`] out.
//!
//! This is the only module that names a `merman` type. Everything downstream —
//! the scene, the labels, the accessibility tree — reads [`crate::layout`].

use alloc::string::{String, ToString as _};
use alloc::vec::Vec;

use merman_core::{Engine, ParseOptions};
use merman_render::LayoutOptions;
use merman_render::environment::RenderEnvironment;
use merman_render::family::{self, LayoutProjection};
use merman_render::model::{LayoutEdge, LayoutLabel, LayoutNode};
use waterui_core::layout::{Point, Rect, Size};
use waterui_str::Str;

use crate::layout::{
    Cluster, DiagramLayout, Edge, EdgeMarker, EdgeStroke, Emphasis, Fragment, Label, Lifeline,
    Node, NodeShape, UnsupportedShape,
};
use crate::measure;

/// Why a diagram could not be drawn.
#[derive(Debug, thiserror::Error)]
pub enum MermaidError {
    /// The source is not a diagram Mermaid recognises.
    #[error("the source does not begin with a Mermaid diagram header")]
    NotADiagram,
    /// The source is a diagram, but its syntax is wrong.
    #[error("{0}")]
    Parse(merman_core::Error),
    /// The layout runtime refused to start a session.
    #[error("{0}")]
    Session(merman_core::runtime::RuntimePolicyError),
    /// The diagram parsed, but laying it out failed.
    #[error("{0}")]
    Layout(merman_render::Error),
    /// The diagram is a family this crate does not draw yet.
    #[error(
        "`{0}` diagrams are not drawn by waterui-mermaid yet; \
         flowchart and sequence are the families it supports"
    )]
    UnsupportedFamily(Str),
    /// The diagram uses a node shape this crate has no outline for.
    #[error(transparent)]
    UnsupportedShape(#[from] UnsupportedShape),
    /// The layout named a piece of geometry this crate does not know how to
    /// draw, which means the diagram family grew something since this code
    /// last read it.
    #[error(
        "the layout produced `{0}`, which waterui-mermaid does not know how to draw; \
         the diagram is not drawn rather than drawn with a piece missing"
    )]
    UnknownGeometry(Str),
}

/// Mermaid's default label font size, used when the source does not set one.
const DEFAULT_FONT_SIZE: f32 = 16.0;

/// The container width Mermaid families that consult one are laid out against.
///
/// Only a handful of families read it, and a diagram is drawn at its natural
/// size regardless; this is the value Mermaid itself defaults to.
const LAYOUT_CONTAINER: Size = Size::new(800.0, 600.0);

/// Parses and lays out one diagram.
///
/// # Errors
///
/// See [`MermaidError`].
pub fn render(source: &str) -> Result<DiagramLayout, MermaidError> {
    let container = LAYOUT_CONTAINER;
    let engine = Engine::new();
    let parsed = engine
        .parse_diagram_for_render_model_sync(source, ParseOptions::default())
        .map_err(MermaidError::Parse)?
        .ok_or(MermaidError::NotADiagram)?;

    let family = Str::from(parsed.metadata().diagram_type.clone());
    let font_size = configured_font_size(parsed.metadata());
    let semantic = parsed.model().clone();

    let session = RenderEnvironment::deterministic()
        .with_text_measurement_policy(measure::policy())
        .begin_session()
        .map_err(MermaidError::Session)?;

    let mut options = LayoutOptions::default();
    options.container_width = f64::from(container.width);
    options.container_height = f64::from(container.height);

    let artifact = family::prepare(parsed, &options, session).map_err(MermaidError::Layout)?;

    match artifact.layout() {
        LayoutProjection::Flowchart(geometry) => {
            let merman_core::RenderSemanticModel::Flowchart(model) = &semantic else {
                unreachable!("a flowchart layout is only produced from a flowchart model")
            };
            flowchart(model, geometry, font_size)
        }
        LayoutProjection::SequenceDiagram(geometry) => {
            let merman_core::RenderSemanticModel::Sequence(model) = &semantic else {
                unreachable!("a sequence layout is only produced from a sequence model")
            };
            sequence(model, geometry, font_size)
        }
        _ => Err(MermaidError::UnsupportedFamily(family)),
    }
}

/// Lowers a flowchart's geometry, joined with the semantic model that knows
/// each node's shape and each edge's decoration.
fn flowchart(
    model: &merman_core::diagrams::flowchart::FlowchartModel,
    geometry: &merman_render::model::FlowchartLayout,
    font_size: f32,
) -> Result<DiagramLayout, MermaidError> {
    use merman_core::diagrams::flowchart::{FlowEdgeStroke, FlowEdgeVisibility};

    let origin = Origin::of(geometry.bounds.as_ref());

    let mut nodes = Vec::with_capacity(geometry.nodes.len());
    for laid_out in &geometry.nodes {
        if laid_out.is_cluster {
            continue;
        }
        let authored = model.nodes.iter().find(|node| node.id == laid_out.id);
        let shape = NodeShape::resolve(
            authored
                .and_then(|node| node.layout_shape.as_deref())
                .unwrap_or("squareRect"),
        )?;
        let frame = origin.node(laid_out);
        nodes.push(Node {
            id: Str::from(laid_out.id.clone()),
            frame,
            shape,
            label: authored
                .and_then(|node| node.label.clone())
                .filter(|text| !text.is_empty())
                .map(|text| Label {
                    id: Str::from(alloc::format!("node:{}", laid_out.id)),
                    // A node's label is centred in its box. The shapes that
                    // waste corner space — diamond, the parallelograms — had
                    // that accounted for when layout sized the box.
                    frame,
                    text: Str::from(text),
                    emphasis: Emphasis::Normal,
                }),
        });
    }

    let mut edges = Vec::with_capacity(geometry.edges.len());
    for laid_out in &geometry.edges {
        let authored = model.edges.iter().find(|edge| edge.id == laid_out.id);
        if authored.is_some_and(|edge| edge.visibility == FlowEdgeVisibility::Invisible) {
            continue;
        }
        edges.push(Edge {
            id: Str::from(laid_out.id.clone()),
            points: laid_out
                .points
                .iter()
                .map(|point| origin.point(point.x, point.y))
                .collect(),
            label: edge_label(
                laid_out,
                authored.and_then(|edge| edge.label.as_deref()),
                origin,
            ),
            stroke: match authored.map(|edge| edge.stroke_kind) {
                Some(FlowEdgeStroke::Dotted) => EdgeStroke::Dotted,
                Some(FlowEdgeStroke::Thick) => EdgeStroke::Thick,
                Some(FlowEdgeStroke::Normal) | None => EdgeStroke::Normal,
            },
            start_marker: marker(authored.map(|edge| edge.start_marker)),
            end_marker: marker(authored.map(|edge| edge.end_marker)),
        });
    }

    let clusters = geometry
        .clusters
        .iter()
        .map(|cluster| Cluster {
            id: Str::from(cluster.id.clone()),
            frame: origin.centred(cluster.x, cluster.y, cluster.width, cluster.height),
            label: (!cluster.title.is_empty()).then(|| Label {
                id: Str::from(alloc::format!("cluster:{}", cluster.id)),
                frame: origin.label(&cluster.title_label),
                text: Str::from(cluster.title.clone()),
                emphasis: Emphasis::Title,
            }),
        })
        .collect();

    Ok(DiagramLayout {
        size: bounds_size(geometry.bounds.as_ref()),
        clusters,
        fragments: Vec::new(),
        nodes,
        edges,
        lifelines: Vec::new(),
        font_size,
        acc_title: model.acc_title.clone().map(Str::from),
        acc_description: model.acc_descr.clone().map(Str::from),
    })
}

/// The label font size the diagram was laid out with.
///
/// `fontSize` is Mermaid's own documented configuration knob, and it is what
/// `merman` derives the `TextStyle` it measures with from. Reading it here is
/// what lets a label be drawn at the size its box was measured at.
fn configured_font_size(metadata: &merman_core::ParseMetadata) -> f32 {
    metadata
        .effective_config
        .as_value()
        .get("fontSize")
        .and_then(serde_json::Value::as_f64)
        .map(|size| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a font size is a small positive magnitude that f32 represents exactly enough to draw with"
            )]
            let size = size as f32;
            size
        })
        .filter(|size| size.is_finite() && *size > 0.0)
        .unwrap_or(DEFAULT_FONT_SIZE)
}

/// Where a diagram's own coordinate space starts.
///
/// Mermaid lays a diagram out wherever its algorithm happens to begin, not at
/// the origin — a sequence diagram's bounds start at `(-50, -10)` — so every
/// coordinate is shifted to put the diagram's top-left corner at `(0, 0)` and
/// the geometry the renderer sees is always positive.
#[derive(Debug, Clone, Copy, Default)]
struct Origin {
    x: f64,
    y: f64,
}

impl Origin {
    /// Reads the diagram's origin off its bounds.
    fn of(bounds: Option<&merman_render::model::Bounds>) -> Self {
        bounds.map_or(Self { x: 0.0, y: 0.0 }, |bounds| Self {
            x: bounds.min_x,
            y: bounds.min_y,
        })
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "diagram coordinates are screen-scale magnitudes that f32 represents exactly enough to draw"
    )]
    fn point(self, x: f64, y: f64) -> Point {
        Point::new((x - self.x) as f32, (y - self.y) as f32)
    }

    /// A box Mermaid positions by its centre.
    fn centred(self, x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect::new(
            self.point(x - width / 2.0, y - height / 2.0),
            size_of(width, height),
        )
    }

    fn node(self, node: &LayoutNode) -> Rect {
        self.centred(node.x, node.y, node.width, node.height)
    }

    fn label(self, label: &LayoutLabel) -> Rect {
        self.centred(label.x, label.y, label.width, label.height)
    }
}

/// Translates a Mermaid edge marker into the decoration we draw.
const fn marker(marker: Option<merman_core::diagrams::flowchart::FlowEdgeMarker>) -> EdgeMarker {
    use merman_core::diagrams::flowchart::FlowEdgeMarker as Authored;
    match marker {
        Some(Authored::Point) => EdgeMarker::Arrow,
        Some(Authored::Circle) => EdgeMarker::Circle,
        Some(Authored::Cross) => EdgeMarker::Cross,
        Some(Authored::None) | None => EdgeMarker::None,
    }
}

/// Pairs an edge's laid-out label box with the text that belongs in it.
fn edge_label(edge: &LayoutEdge, text: Option<&str>, origin: Origin) -> Option<Label> {
    let frame = edge.label.as_ref()?;
    let text = text.map(str::trim).filter(|text| !text.is_empty())?;
    Some(Label {
        id: Str::from(alloc::format!("edge:{}", edge.id)),
        frame: origin.label(frame),
        text: Str::from(text.to_string()),
        emphasis: Emphasis::Normal,
    })
}

/// The diagram's natural size.
fn bounds_size(bounds: Option<&merman_render::model::Bounds>) -> Size {
    bounds.map_or_else(Size::default, |bounds| {
        size_of(bounds.max_x - bounds.min_x, bounds.max_y - bounds.min_y)
    })
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "diagram coordinates are screen-scale magnitudes that f32 represents exactly enough to draw"
)]
const fn size_of(width: f64, height: f64) -> Size {
    Size::new(width as f32, height as f32)
}

/// Mermaid's `SequenceDB.LINETYPE` values, which is where a message's arrow and
/// stroke come from — the laid-out edge carries geometry only.
mod linetype {
    pub const DOTTED: i32 = 1;
    pub const SOLID_CROSS: i32 = 3;
    pub const DOTTED_CROSS: i32 = 4;
    pub const SOLID_OPEN: i32 = 5;
    pub const DOTTED_OPEN: i32 = 6;
    pub const SOLID_POINT: i32 = 24;
    pub const DOTTED_POINT: i32 = 25;
    pub const BIDIRECTIONAL_DOTTED: i32 = 34;

    /// The keyword a block-opening message spells, if it opens one.
    pub const fn block_keyword(line_type: i32) -> Option<&'static str> {
        Some(match line_type {
            10 => "loop",
            12 => "alt",
            15 => "opt",
            19 | 32 => "par",
            22 => "rect",
            27 => "critical",
            30 => "break",
            _ => return None,
        })
    }

    /// Whether the message is drawn with a dashed line.
    pub const fn is_dotted(line_type: i32) -> bool {
        matches!(
            line_type,
            DOTTED | DOTTED_CROSS | DOTTED_OPEN | DOTTED_POINT | BIDIRECTIONAL_DOTTED
        )
    }

    /// The decoration at the arrow's head.
    pub const fn head(line_type: i32) -> crate::layout::EdgeMarker {
        use crate::layout::EdgeMarker;
        match line_type {
            SOLID_CROSS | DOTTED_CROSS => EdgeMarker::Cross,
            SOLID_POINT | DOTTED_POINT => EdgeMarker::Circle,
            SOLID_OPEN | DOTTED_OPEN => EdgeMarker::None,
            _ => EdgeMarker::Arrow,
        }
    }
}

/// Lowers a sequence diagram.
///
/// The layout names what each piece is: `actor-top-<id>` and
/// `actor-bottom-<id>` are a participant's two headers, `note-<n>` is a note,
/// `msg-<n>` is a message and `lifeline-<id>` is the vertical line between one
/// participant's headers. The style of a message — dashed or solid, which
/// arrowhead — lives in the semantic model rather than the geometry, so the two
/// are joined here by that index.
fn sequence(
    model: &merman_core::diagrams::sequence::SequenceDiagramRenderModel,
    geometry: &merman_render::model::SequenceDiagramLayout,
    font_size: f32,
) -> Result<DiagramLayout, MermaidError> {
    let origin = Origin::of(geometry.bounds.as_ref());

    let mut nodes = Vec::with_capacity(geometry.nodes.len());
    for laid_out in &geometry.nodes {
        nodes.push(sequence_node(model, laid_out, origin)?);
    }

    let mut edges = Vec::new();
    let mut lifelines = Vec::new();
    for laid_out in &geometry.edges {
        match sequence_connector(model, laid_out, origin) {
            Connector::Message(edge) => edges.push(edge),
            Connector::Lifeline(lifeline) => lifelines.push(lifeline),
            Connector::Degenerate => {}
        }
    }

    Ok(DiagramLayout {
        size: bounds_size(geometry.bounds.as_ref()),
        clusters: participant_boxes(model, geometry, origin),
        fragments: fragments(model, geometry, origin, &lifelines),
        nodes,
        edges,
        lifelines,
        font_size,
        acc_title: model.acc_title.clone().map(Str::from),
        acc_description: model.acc_descr.clone().map(Str::from),
    })
}

/// One node of a sequence diagram's geometry.
///
/// The layout's own id says what the node is; the semantic model says what it
/// holds. A node whose id matches neither shape is a family feature this crate
/// has not learned yet, and drawing it as a bare rectangle would hide that.
fn sequence_node(
    model: &merman_core::diagrams::sequence::SequenceDiagramRenderModel,
    laid_out: &LayoutNode,
    origin: Origin,
) -> Result<Node, MermaidError> {
    let frame = origin.node(laid_out);
    let (shape, text) = if let Some(actor_id) = laid_out
        .id
        .strip_prefix("actor-top-")
        .or_else(|| laid_out.id.strip_prefix("actor-bottom-"))
    {
        let actor = model.actors.get(actor_id);
        let shape = if actor.is_some_and(|actor| actor.actor_type == "actor") {
            NodeShape::Actor
        } else {
            NodeShape::Participant
        };
        (shape, actor.map(|actor| actor.description.clone()))
    } else if let Some(index) = laid_out.id.strip_prefix("note-") {
        let text = index
            .parse::<usize>()
            .ok()
            .and_then(|index| model.messages.get(index))
            .map(|message| message.message.as_text().to_owned());
        (NodeShape::Note, text)
    } else {
        return Err(MermaidError::UnknownGeometry(Str::from(
            laid_out.id.clone(),
        )));
    };

    Ok(Node {
        id: Str::from(laid_out.id.clone()),
        frame,
        shape,
        label: text
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
            .map(|text| Label {
                id: Str::from(alloc::format!("node:{}", laid_out.id)),
                frame: crate::shape::label_area(shape, frame),
                text: Str::from(text),
                emphasis: if laid_out.id.starts_with("note-") {
                    Emphasis::Normal
                } else {
                    Emphasis::Title
                },
            }),
    })
}

/// What one of a sequence layout's edges turns out to be.
enum Connector {
    /// A message between participants.
    Message(Edge),
    /// A participant's vertical line.
    Lifeline(Lifeline),
    /// A lifeline with no endpoints, which is nothing to draw.
    Degenerate,
}

/// One edge of a sequence diagram's geometry.
///
/// A message's style — dashed or solid, and which arrowhead — is not in the
/// geometry: it comes from Mermaid's `LINETYPE` on the semantic message the
/// edge's id indexes.
fn sequence_connector(
    model: &merman_core::diagrams::sequence::SequenceDiagramRenderModel,
    laid_out: &LayoutEdge,
    origin: Origin,
) -> Connector {
    if let Some(actor_id) = laid_out.id.strip_prefix("lifeline-") {
        let (Some(top), Some(bottom)) = (laid_out.points.first(), laid_out.points.last()) else {
            return Connector::Degenerate;
        };
        let top = origin.point(top.x, top.y);
        let bottom = origin.point(bottom.x, bottom.y);
        return Connector::Lifeline(Lifeline {
            id: Str::from(actor_id.to_owned()),
            x: top.x,
            top: top.y,
            bottom: bottom.y,
        });
    }

    let message = laid_out
        .id
        .strip_prefix("msg-")
        .and_then(|index| index.parse::<usize>().ok())
        .and_then(|index| model.messages.get(index));
    let line_type = message.map_or(0, |message| message.message_type);

    Connector::Message(Edge {
        id: Str::from(laid_out.id.clone()),
        points: laid_out
            .points
            .iter()
            .map(|point| origin.point(point.x, point.y))
            .collect(),
        label: edge_label(
            laid_out,
            message.map(|message| message.message.as_text()),
            origin,
        ),
        stroke: if linetype::is_dotted(line_type) {
            EdgeStroke::Dotted
        } else {
            EdgeStroke::Normal
        },
        start_marker: EdgeMarker::None,
        end_marker: linetype::head(line_type),
    })
}

/// The `box ... end` groupings a sequence diagram can put around participants.
fn participant_boxes(
    model: &merman_core::diagrams::sequence::SequenceDiagramRenderModel,
    geometry: &merman_render::model::SequenceDiagramLayout,
    origin: Origin,
) -> Vec<Cluster> {
    let _ = model;
    geometry
        .clusters
        .iter()
        .map(|cluster| Cluster {
            id: Str::from(cluster.id.clone()),
            frame: origin.centred(cluster.x, cluster.y, cluster.width, cluster.height),
            label: (!cluster.title.is_empty()).then(|| Label {
                id: Str::from(alloc::format!("cluster:{}", cluster.id)),
                frame: origin.label(&cluster.title_label),
                text: Str::from(cluster.title.clone()),
                emphasis: Emphasis::Title,
            }),
        })
        .collect()
}

/// The `loop` / `alt` / `opt` frames.
///
/// The layout supplies each fragment's vertical extent, keyed by the index of
/// the message that opened it. Its horizontal extent is not given and is not
/// guessed: it is the span of the lifelines belonging to the participants the
/// fragment's own messages touch, which is what Mermaid draws the frame around.
/// A fragment enclosing no messages spans every lifeline, as Mermaid draws it.
fn fragments(
    model: &merman_core::diagrams::sequence::SequenceDiagramRenderModel,
    geometry: &merman_render::model::SequenceDiagramLayout,
    origin: Origin,
    lifelines: &[Lifeline],
) -> Vec<Fragment> {
    /// How far a fragment's frame sits outside the lifelines it encloses.
    const MARGIN: f32 = 24.0;
    /// Height of the keyword tab in the fragment's top-left corner.
    const TAB_HEIGHT: f32 = 20.0;
    /// Width of the keyword tab.
    const TAB_WIDTH: f32 = 52.0;

    let mut fragments = Vec::with_capacity(geometry.block_layouts_by_id.len());
    for (id, block) in &geometry.block_layouts_by_id {
        let opener = id
            .parse::<usize>()
            .ok()
            .and_then(|index| model.messages.get(index));
        let Some(keyword) = opener
            .map(|message| message.message_type)
            .and_then(linetype::block_keyword)
        else {
            continue;
        };

        let enclosed = enclosed_actors(model, geometry, block);
        let (left, right) = lifeline_span(lifelines, &enclosed);
        let top = origin.point(0.0, block.start_y).y;
        let bottom = origin.point(0.0, block.stop_y).y;
        let (left, right) = (left - MARGIN, right + MARGIN);

        let mut labels = Vec::with_capacity(2);
        labels.push(Label {
            id: Str::from(alloc::format!("fragment:{id}:keyword")),
            frame: Rect::new(
                Point::new(left, top),
                Size::new(TAB_WIDTH.min(right - left), TAB_HEIGHT),
            ),
            text: Str::from(keyword),
            emphasis: Emphasis::Title,
        });
        if let Some(guard) = opener
            .map(|message| message.message.as_text().trim().to_owned())
            .filter(|guard| !guard.is_empty())
        {
            labels.push(Label {
                id: Str::from(alloc::format!("fragment:{id}:guard")),
                frame: Rect::new(
                    Point::new(left + TAB_WIDTH, top),
                    Size::new((right - left - TAB_WIDTH).max(0.0), TAB_HEIGHT),
                ),
                text: Str::from(guard),
                emphasis: Emphasis::Muted,
            });
        }

        fragments.push(Fragment {
            id: Str::from(id.clone()),
            frame: Rect::new(Point::new(left, top), Size::new(right - left, bottom - top)),
            dividers: block
                .section_ys_by_id
                .values()
                .map(|y| origin.point(0.0, *y).y)
                .collect(),
            labels,
        });
    }
    // `block_layouts_by_id` is a hash map, and a frame drawn under another has
    // to be drawn first, so order them outermost-first by height.
    fragments.sort_by(|left, right| {
        right
            .frame
            .height()
            .total_cmp(&left.frame.height())
            .then_with(|| left.frame.y().total_cmp(&right.frame.y()))
    });
    fragments
}

/// The participants whose messages fall inside one fragment.
///
/// Membership is decided by the laid-out message's own `y`, which is the only
/// thing that relates a message to a fragment: the semantic model nests nothing.
fn enclosed_actors(
    model: &merman_core::diagrams::sequence::SequenceDiagramRenderModel,
    geometry: &merman_render::model::SequenceDiagramLayout,
    block: &merman_render::model::SequenceBlockLayout,
) -> Vec<String> {
    let mut actors = Vec::new();
    for edge in &geometry.edges {
        let Some(index) = edge
            .id
            .strip_prefix("msg-")
            .and_then(|index| index.parse::<usize>().ok())
        else {
            continue;
        };
        let Some(y) = edge.points.first().map(|point| point.y) else {
            continue;
        };
        if y < block.start_y || y > block.stop_y {
            continue;
        }
        let Some(message) = model.messages.get(index) else {
            continue;
        };
        for actor in [message.from.as_ref(), message.to.as_ref()]
            .into_iter()
            .flatten()
        {
            if !actors.iter().any(|known| known == actor) {
                actors.push(actor.clone());
            }
        }
    }
    actors
}

/// The horizontal extent of the named lifelines, or of every lifeline when the
/// list is empty.
fn lifeline_span(lifelines: &[Lifeline], actors: &[String]) -> (f32, f32) {
    let mut left = f32::INFINITY;
    let mut right = f32::NEG_INFINITY;
    for lifeline in lifelines {
        let id: &str = &lifeline.id;
        let selected = actors.is_empty() || actors.iter().any(|actor| actor == id);
        if selected {
            left = left.min(lifeline.x);
            right = right.max(lifeline.x);
        }
    }
    if left.is_finite() && right.is_finite() {
        (left, right)
    } else {
        (0.0, 0.0)
    }
}
