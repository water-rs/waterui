//! Event payloads carried by each channel.
//!
//! Payloads are deliberately backend-agnostic: a rendering backend projects its
//! own frame and accessibility data into these types, so an Apple, Android, or
//! Hydrolysis target all look the same to the inspector.

use serde::{Deserialize, Serialize};

use crate::channel::Channel;

/// One event from the target application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InspectorEvent {
    /// A rendered (or deliberately skipped) frame.
    Frame(FrameSample),
    /// An accessibility-tree update.
    Tree(TreeUpdate),
    /// One aggregation window of main-thread task occupancy.
    Tasks(TaskWindow),
    /// A single main-thread stall worth a backtrace.
    Stall(StallSample),
    /// A `tracing` record.
    Log(LogRecord),
    /// A reactive-graph event.
    Signal(SignalEvent),
}

impl InspectorEvent {
    /// The channel this event belongs to.
    #[must_use]
    pub const fn channel(&self) -> Channel {
        match self {
            Self::Frame(_) => Channel::Frames,
            Self::Tree(_) => Channel::Tree,
            Self::Tasks(_) | Self::Stall(_) => Channel::Tasks,
            Self::Log(_) => Channel::Logs,
            Self::Signal(_) => Channel::Signals,
        }
    }
}

// ---- frames -------------------------------------------------------------

/// What a frame actually cost the renderer.
///
/// A renderer that skips work still reports the frame, because "we did nothing"
/// is the answer to most performance questions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FrameSample {
    /// How much of the pipeline this frame ran.
    pub kind: FrameKind,
    /// Time spent laying out the view tree.
    pub layout_us: u32,
    /// Time spent encoding the scene.
    pub encode_us: u32,
    /// Time spent presenting to the surface.
    pub present_us: u32,
    /// Wall time for the whole frame.
    pub total_us: u32,
    /// Frame budget derived from the display refresh rate.
    pub budget_us: u32,
    /// Display refresh rate the budget came from.
    pub refresh_hz: f32,
    /// Composited layers submitted for this frame.
    pub layers: u32,
    /// Deepest nesting of clip layers reached while encoding.
    ///
    /// Clip nesting is one of the few structural costs a renderer pays that a
    /// developer can act on directly, by flattening the view tree.
    pub clip_depth: u32,
}

impl FrameSample {
    /// Fraction of the frame budget consumed, where `1.0` is exactly on budget.
    ///
    /// Returns `None` when the budget is unknown, rather than inventing one.
    #[must_use]
    pub fn budget_usage(&self) -> Option<f32> {
        if self.budget_us == 0 {
            return None;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "microsecond counts are far below f32's exact-integer range"
        )]
        Some(self.total_us as f32 / self.budget_us as f32)
    }
}

/// How much of the render pipeline a frame ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameKind {
    /// Nothing to show; no work was done.
    Idle,
    /// Visual state changed but layout did not; the tree was re-encoded only.
    Reencode,
    /// The retained tree was laid out again and re-encoded.
    Refresh,
}

impl FrameKind {
    /// Stable lowercase name for display.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Reencode => "reencode",
            Self::Refresh => "refresh",
        }
    }
}

// ---- tree ---------------------------------------------------------------

/// Stable identifier for an accessibility node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u64);

/// An incremental accessibility-tree update.
///
/// Updates carry only what changed. A client that connects mid-session receives
/// a full update first, marked by `full`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeUpdate {
    /// Monotonic revision of the tree this update produces.
    pub revision: u64,
    /// Root node of the tree.
    pub root: NodeId,
    /// Node holding input focus, when any.
    pub focus: Option<NodeId>,
    /// Whether this update describes the entire tree rather than a delta.
    pub full: bool,
    /// Nodes created or modified by this update.
    pub changed: Vec<TreeNode>,
    /// Nodes removed by this update.
    pub removed: Vec<NodeId>,
}

/// One node of the accessibility tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreeNode {
    /// Stable identifier.
    pub id: NodeId,
    /// Backend-reported role, lowercase (`button`, `label`, `list`).
    pub role: String,
    /// Accessibility label.
    pub label: Option<String>,
    /// Current value, for controls that have one.
    pub value: Option<String>,
    /// Layout rectangle in window coordinates.
    pub bounds: Option<Bounds>,
    /// Interaction and selection state.
    pub state: NodeState,
    /// Children, in order.
    pub children: Vec<NodeId>,
}

/// Interaction state of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NodeState {
    /// Whether the node accepts interaction.
    pub enabled: bool,
    /// Whether the node is selected.
    pub selected: bool,
    /// Toggle state, for nodes that have one.
    pub checked: Option<bool>,
    /// Disclosure state, for nodes that have one.
    pub expanded: Option<bool>,
    /// Whether the node is hidden from assistive technology.
    pub hidden: bool,
}

/// A rectangle in window coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    /// Distance from the window's left edge.
    pub x: f32,
    /// Distance from the window's top edge.
    pub y: f32,
    /// Width in points.
    pub width: f32,
    /// Height in points.
    pub height: f32,
}

// ---- tasks --------------------------------------------------------------

/// Main-thread occupancy aggregated over one window.
///
/// Aggregation happens in the target process. Reporting every poll individually
/// costs more than the work being measured.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskWindow {
    /// Length of the aggregation window.
    pub window_ms: u32,
    /// Frame budget the tasks were measured against.
    pub budget_us: u32,
    /// Display refresh rate the budget came from.
    pub refresh_hz: f32,
    /// Per-task-type totals, most expensive first.
    pub tasks: Vec<TaskAggregate>,
}

/// Totals for one task type within an aggregation window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAggregate {
    /// Type name of the spawned future.
    pub task_type: String,
    /// Number of polls in the window.
    pub polls: u32,
    /// How many of those polls completed the future.
    pub ready: u32,
    /// Total wall time across all polls.
    pub wall_us_total: u64,
    /// Longest single poll.
    pub wall_us_max: u32,
    /// Total CPU time across all polls.
    pub cpu_us_total: u64,
    /// How many polls exceeded the frame budget.
    pub over_budget: u32,
}

/// One poll that overran the frame budget badly enough to be worth a backtrace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StallSample {
    /// Type name of the spawned future.
    pub task_type: String,
    /// Wall time spent in the poll.
    pub wall_us: u64,
    /// CPU time spent in the poll.
    pub cpu_us: u64,
    /// Frame budget that was exceeded.
    pub budget_us: u64,
    /// Percentage of the frame budget consumed.
    pub usage_pct: f32,
    /// Captured stack, innermost first.
    ///
    /// Empty when backtrace capture is rate-limited: capturing a stack costs
    /// milliseconds on the very thread being measured, so it is deliberately
    /// not done for every stall.
    pub backtrace: Vec<String>,
}

// ---- logs ---------------------------------------------------------------

/// One `tracing` record from the target application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRecord {
    /// Severity.
    pub level: LogLevel,
    /// Emitting target, usually a module path.
    pub target: String,
    /// Rendered message.
    pub message: String,
    /// Source file, when the record carried one.
    pub file: Option<String>,
    /// Source line, when the record carried one.
    pub line: Option<u32>,
    /// Structured fields other than the message.
    pub fields: Vec<(String, String)>,
}

/// Severity of a log record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LogLevel {
    /// Verbose tracing.
    Trace,
    /// Debugging detail.
    Debug,
    /// Informational.
    Info,
    /// Something suspicious.
    Warn,
    /// Something broken.
    Error,
}

impl LogLevel {
    /// Stable uppercase name for display.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

// ---- signals ------------------------------------------------------------

/// Stable identifier for a reactive-graph node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SignalId(pub u64);

/// A reactive-graph lifecycle or notification event.
///
/// Only [`SignalEvent::Created`] carries strings. Notifications are the hot path
/// and reference the node by id, so a busy graph does not re-send its own
/// source locations thousands of times per second.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SignalEvent {
    /// A state-owning node entered the graph.
    Created(SignalNodeInfo),
    /// A watcher was registered.
    Subscribed {
        /// The node subscribed to.
        node: SignalId,
        /// Live watcher count after the change.
        subscribers: u32,
    },
    /// A watcher was dropped.
    Unsubscribed {
        /// The node unsubscribed from.
        node: SignalId,
        /// Live watcher count after the change.
        subscribers: u32,
    },
    /// The node notified its watchers.
    Notified {
        /// The notifying node.
        node: SignalId,
        /// How many watchers were notified.
        subscribers: u32,
    },
    /// The node left the graph.
    Dropped {
        /// The node that was dropped.
        node: SignalId,
    },
}

/// Provenance of a reactive-graph node, sent once when it is created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalNodeInfo {
    /// Stable identifier used by every later event for this node.
    pub node: SignalId,
    /// Type name of the value the node holds.
    pub type_name: String,
    /// Source file that created the node.
    pub file: String,
    /// Source line that created the node.
    pub line: u32,
    /// Source column that created the node.
    pub column: u32,
}

#[cfg(test)]
mod tests {
    use super::{Channel, FrameKind, FrameSample, InspectorEvent};

    #[test]
    fn stalls_and_aggregates_share_the_tasks_channel() {
        let window = InspectorEvent::Tasks(super::TaskWindow {
            window_ms: 100,
            budget_us: 8333,
            refresh_hz: 120.0,
            tasks: Vec::new(),
        });
        let stall = InspectorEvent::Stall(super::StallSample {
            task_type: String::from("Frame"),
            wall_us: 20_000,
            cpu_us: 19_000,
            budget_us: 8_333,
            usage_pct: 240.0,
            backtrace: Vec::new(),
        });
        assert_eq!(window.channel(), Channel::Tasks);
        assert_eq!(stall.channel(), Channel::Tasks);
    }

    /// An unknown budget must not silently read as "perfectly on budget".
    #[test]
    fn budget_usage_is_absent_without_a_budget() {
        let mut sample = FrameSample {
            kind: FrameKind::Refresh,
            layout_us: 100,
            encode_us: 200,
            present_us: 300,
            total_us: 600,
            budget_us: 0,
            refresh_hz: 0.0,
            layers: 10,
            clip_depth: 2,
        };
        assert_eq!(sample.budget_usage(), None);

        sample.budget_us = 1200;
        assert_eq!(sample.budget_usage(), Some(0.5));
    }
}
