//! The geometry this crate draws, in `WaterUI` units.
//!
//! Nothing outside [`crate::engine`] names a `merman` type. Everything the
//! renderer and the label layer read is defined here, so the crate that
//! produces the geometry can change — and it will, once the typed layout
//! projection is released upstream — without the drawing code noticing.

use alloc::vec::Vec;

use waterui_core::layout::{Point, Rect, Size};
use waterui_str::Str;

/// One laid-out diagram.
#[derive(Debug, Clone, Default)]
pub struct DiagramLayout {
    /// The diagram's natural size, before any scaling to fit its container.
    pub size: Size,
    /// Subgraph and participant-box frames, drawn beneath everything else.
    pub clusters: Vec<Cluster>,
    /// Sequence-diagram `loop` / `alt` / `opt` frames, drawn beneath messages.
    pub fragments: Vec<Fragment>,
    /// Node boxes.
    pub nodes: Vec<Node>,
    /// Connections between nodes.
    pub edges: Vec<Edge>,
    /// Vertical participant lines, empty for every diagram but `sequence`.
    pub lifelines: Vec<Lifeline>,
    /// The font size the diagram's labels were laid out at, from Mermaid's
    /// `fontSize` configuration. Labels must be drawn at this size: it is the
    /// size their boxes were measured with.
    pub font_size: f32,
    /// The diagram's accessible name, from `accTitle`.
    pub acc_title: Option<Str>,
    /// The diagram's accessible description, from `accDescr`.
    pub acc_description: Option<Str>,
}

impl DiagramLayout {
    /// Every label the diagram draws, in the order they should be placed.
    ///
    /// Labels are not painted into the scene: they are real text views, and
    /// this is what the label layer iterates.
    pub fn labels(&self) -> impl Iterator<Item = &Label> {
        self.clusters
            .iter()
            .filter_map(|cluster| cluster.label.as_ref())
            .chain(
                self.fragments
                    .iter()
                    .flat_map(|fragment| fragment.labels.iter()),
            )
            .chain(self.nodes.iter().filter_map(|node| node.label.as_ref()))
            .chain(self.edges.iter().filter_map(|edge| edge.label.as_ref()))
    }
}

/// A piece of text the diagram draws as a real view rather than as glyphs
/// painted into the scene.
#[derive(Debug, Clone)]
pub struct Label {
    /// Stable identity, so a membership change diffs instead of rebuilding.
    pub id: Str,
    /// The box layout reserved for this text, in diagram coordinates.
    pub frame: Rect,
    /// The text itself.
    pub text: Str,
    /// How prominent the text is, which decides its font weight.
    pub emphasis: Emphasis,
}

/// How prominent a label is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Emphasis {
    /// Ordinary label text.
    #[default]
    Normal,
    /// A title: a subgraph name, a fragment keyword, a participant name.
    Title,
    /// Secondary text, such as a fragment's guard condition.
    Muted,
}

/// A node box.
#[derive(Debug, Clone)]
pub struct Node {
    /// The node's id in the source.
    pub id: Str,
    /// The box the node occupies.
    pub frame: Rect,
    /// The outline to stroke and fill.
    pub shape: NodeShape,
    /// The node's label, if it has one.
    pub label: Option<Label>,
}

/// The outline of a node.
///
/// This is the geometric shape, which is what Mermaid calls a node's
/// `layout_shape` — the thing that decided how much room layout reserved. The
/// authored spelling (`[]`, `()`, `{{}}`, `shape: cyl`) is a source detail that
/// has already been resolved by the time geometry exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeShape {
    /// `A[text]` — a plain rectangle.
    Rectangle,
    /// `A(text)` — a rectangle with rounded corners.
    RoundedRectangle,
    /// `A([text])` — fully rounded ends.
    Stadium,
    /// `A[[text]]` — a rectangle with an inner vertical rule at each end.
    Subroutine,
    /// `A[(text)]` — a database cylinder.
    Cylinder,
    /// `A((text))` — a circle.
    Circle,
    /// `A(((text)))` — a circle inside a circle.
    DoubleCircle,
    /// `A>text]` — a flag with one notched edge.
    Asymmetric,
    /// `A{text}` — a decision diamond.
    Diamond,
    /// `A{{text}}` — a hexagon.
    Hexagon,
    /// `A[/text/]` — a parallelogram leaning right.
    ParallelogramRight,
    /// `A[\text\]` — a parallelogram leaning left.
    ParallelogramLeft,
    /// `A[/text\]` — a trapezoid, wide at the bottom.
    Trapezoid,
    /// `A[\text/]` — a trapezoid, wide at the top.
    TrapezoidInverted,
    /// A label with no outline at all.
    Text,
    /// A sequence-diagram participant header.
    Participant,
    /// A sequence-diagram actor stick figure.
    Actor,
    /// A sequence-diagram note.
    Note,
}

impl NodeShape {
    /// Resolves Mermaid's geometric shape name.
    ///
    /// The names are Mermaid's own `layout_shape` vocabulary, including the
    /// aliases its shape catalogue accepts. An unrecognised name is an error
    /// rather than a silently substituted rectangle: a diagram drawn with the
    /// wrong outline is a wrong diagram, and saying so is the only way the gap
    /// ever gets closed.
    ///
    /// # Errors
    ///
    /// Returns the unrecognised name when this crate has no outline for it.
    pub fn resolve(name: &str) -> Result<Self, UnsupportedShape> {
        Ok(match name {
            "squareRect" | "rect" | "proc" | "process" | "rectangle" => Self::Rectangle,
            "roundedRect" | "rounded" | "event" => Self::RoundedRectangle,
            "stadium" | "terminal" | "pill" => Self::Stadium,
            "subroutine" | "subprocess" | "framed-rectangle" | "subproc" => Self::Subroutine,
            "cylinder" | "db" | "database" | "cyl" | "disk" => Self::Cylinder,
            "circle" | "circ" => Self::Circle,
            "doublecircle" | "double-circle" | "dbl-circ" => Self::DoubleCircle,
            "odd" | "flag" | "rect_left_inv_arrow" => Self::Asymmetric,
            "diamond" | "question" | "diam" | "decision" => Self::Diamond,
            "hexagon" | "hex" | "prepare" => Self::Hexagon,
            "lean_right" | "lean-r" | "in-out" => Self::ParallelogramRight,
            "lean_left" | "lean-l" | "out-in" => Self::ParallelogramLeft,
            "trapezoid" | "trap-b" | "priority" => Self::Trapezoid,
            "inv_trapezoid" | "trap-t" | "manual" => Self::TrapezoidInverted,
            "text" => Self::Text,
            other => return Err(UnsupportedShape(Str::from(other.to_owned()))),
        })
    }
}

/// A node shape this crate cannot draw.
#[derive(Debug, Clone, thiserror::Error)]
#[error(
    "Mermaid node shape `{0}` has no outline in waterui-mermaid yet; \
     the diagram is not drawn rather than drawn with the wrong shape"
)]
pub struct UnsupportedShape(pub Str);

/// A connection between two nodes.
#[derive(Debug, Clone)]
pub struct Edge {
    /// The edge's id in the layout.
    pub id: Str,
    /// The polyline the edge follows, already routed around the nodes.
    pub points: Vec<Point>,
    /// The edge's label, if it has one.
    pub label: Option<Label>,
    /// The stroke pattern.
    pub stroke: EdgeStroke,
    /// The decoration at the first point.
    pub start_marker: EdgeMarker,
    /// The decoration at the last point.
    pub end_marker: EdgeMarker,
}

/// How an edge's line is stroked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeStroke {
    /// A solid line at the normal weight.
    #[default]
    Normal,
    /// A dashed line.
    Dotted,
    /// A solid line at twice the normal weight.
    Thick,
}

/// The decoration at one end of an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeMarker {
    /// A bare line end.
    #[default]
    None,
    /// A filled arrowhead.
    Arrow,
    /// An open circle.
    Circle,
    /// A cross.
    Cross,
}

/// A subgraph frame, or a sequence diagram's participant box.
#[derive(Debug, Clone)]
pub struct Cluster {
    /// The cluster's id in the source.
    pub id: Str,
    /// The frame the cluster occupies.
    pub frame: Rect,
    /// The cluster's title.
    pub label: Option<Label>,
}

/// A sequence diagram's `loop` / `alt` / `opt` / `par` frame.
#[derive(Debug, Clone)]
pub struct Fragment {
    /// The fragment's id in the layout.
    pub id: Str,
    /// The frame the fragment encloses.
    pub frame: Rect,
    /// The `y` of each divider inside the frame, such as an `else` boundary.
    pub dividers: Vec<f32>,
    /// The keyword tab and any guard conditions.
    pub labels: Vec<Label>,
}

/// A sequence diagram participant's vertical line.
#[derive(Debug, Clone)]
pub struct Lifeline {
    /// The participant this line belongs to.
    pub id: Str,
    /// The line's `x`, in diagram coordinates.
    pub x: f32,
    /// Where the line starts.
    pub top: f32,
    /// Where the line ends.
    pub bottom: f32,
}
