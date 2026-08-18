//! Reactive state the Inspector renders.
//!
//! Every field is a signal or a reactive collection, so an incoming event
//! updates exactly the views that depend on it. Nothing here is rebuilt
//! wholesale: a log line arriving does not disturb the frame chart, and a frame
//! arriving does not disturb the tree.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use waterui::Identifiable;
use waterui::chart::DataPoint;
use waterui::prelude::*;
use waterui_inspector_protocol::{
    Channel, ChannelSet, FrameSample, LogLevel, LogRecord, NodeId, SignalEvent, SignalId,
    StallSample, TargetInfo, TaskWindow, TreeNode, TreeUpdate,
};

/// How many frames the timeline keeps.
///
/// At 120 Hz this is about two and a half seconds — long enough to see a hitch
/// in context, short enough that the chart stays readable.
const FRAME_HISTORY: usize = 300;

/// How many log lines and signal events are retained.
const EVENT_HISTORY: usize = 500;

/// Where the inspector is in its connection lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connection {
    /// Trying to reach the target.
    Connecting,
    /// Attached and streaming.
    Live,
    /// Detached; the reason is worth showing rather than hiding.
    Failed(Str),
}

/// A section of the inspector, and the sidebar selection type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    /// Connection and target summary.
    Overview,
    /// Frame timeline.
    Frames,
    /// Accessibility tree.
    Tree,
    /// Main-thread task occupancy.
    Tasks,
    /// Application logs.
    Logs,
    /// Reactive-graph activity.
    Signals,
}

impl Section {
    /// Every section, in sidebar order.
    pub const ALL: [Self; 6] = [
        Self::Overview,
        Self::Frames,
        Self::Tree,
        Self::Tasks,
        Self::Logs,
        Self::Signals,
    ];

    /// Sidebar title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Frames => "Frames",
            Self::Tree => "View tree",
            Self::Tasks => "Tasks",
            Self::Logs => "Logs",
            Self::Signals => "Signals",
        }
    }

    /// The channel this section needs, if it needs one.
    #[must_use]
    pub const fn channel(self) -> Option<Channel> {
        match self {
            Self::Overview => None,
            Self::Frames => Some(Channel::Frames),
            Self::Tree => Some(Channel::Tree),
            Self::Tasks => Some(Channel::Tasks),
            Self::Logs => Some(Channel::Logs),
            Self::Signals => Some(Channel::Signals),
        }
    }
}

/// One row of the flattened accessibility tree.
#[derive(Debug, Clone, PartialEq, Eq, Identifiable)]
pub struct TreeRow {
    /// Node identity, which is also the reconciliation key.
    #[id]
    pub id: u64,
    /// Nesting depth, used for indentation.
    pub depth: usize,
    /// Node role.
    pub role: Str,
    /// Accessibility label, when the node has one.
    pub label: Option<Str>,
    /// Node value, when the node has one.
    pub value: Option<Str>,
    /// Formatted bounds, when the node is laid out.
    pub bounds: Option<Str>,
    /// Whether the node is hidden from assistive technology.
    pub hidden: bool,
    /// Whether the node accepts interaction.
    pub enabled: bool,
}

/// One row of the task table.
#[derive(Debug, Clone, PartialEq, Eq, Identifiable)]
pub struct TaskRow {
    /// Task type name, which is also its identity.
    #[id]
    pub id: Str,
    /// Polls in the last window.
    pub polls: u32,
    /// Mean wall time per poll.
    pub mean_us: u64,
    /// Longest single poll.
    pub max_us: u32,
    /// Polls that exceeded the frame budget.
    pub over_budget: u32,
}

/// One log line.
#[derive(Debug, Clone, PartialEq, Eq, Identifiable)]
pub struct LogRow {
    /// Monotonic identity; log records have no natural key.
    #[id]
    pub id: u64,
    /// Severity.
    pub level: LogLevel,
    /// Emitting target.
    pub target: Str,
    /// Rendered message.
    pub message: Str,
    /// Source location, when the record carried one.
    pub origin: Option<Str>,
}

/// One reactive-graph node, as the inspector understands it.
#[derive(Debug, Clone, PartialEq, Eq, Identifiable)]
pub struct SignalRow {
    /// Node identity.
    #[id]
    pub id: u64,
    /// Type of the value it holds.
    pub type_name: Str,
    /// Where in the application it was created.
    pub origin: Str,
    /// Live watchers.
    pub subscribers: u32,
    /// Notifications observed since it appeared.
    pub notifications: u64,
    /// Whether the node has since been dropped.
    pub dropped: bool,
}

/// One stall, with whatever stack was affordable to capture.
#[derive(Debug, Clone, PartialEq, Identifiable)]
pub struct StallRow {
    /// Monotonic identity; stalls have no natural key.
    #[id]
    pub id: u64,
    /// Task that overran.
    pub task_type: Str,
    /// Percentage of the frame budget consumed.
    pub usage_pct: f32,
    /// Wall time spent in the poll.
    pub wall_us: u64,
    /// Captured stack, empty when capture was rate-limited.
    pub backtrace: Vec<Str>,
}

/// Everything the Inspector displays.
#[derive(Clone)]
#[expect(
    missing_debug_implementations,
    reason = "a reactive model's fields are signals; their debug output is the type name, not the state"
)]
pub struct Model {
    /// Which pane is shown; also driven by the target asking to reveal a node.
    pub section: Binding<Option<Section>>,
    /// The node the target last asked to show, highlighted in the tree.
    pub revealed: Binding<Option<u64>>,
    /// Connection lifecycle.
    pub connection: Binding<Connection>,
    /// Endpoint being inspected.
    pub endpoint: Binding<Str>,
    /// What the target said it is.
    pub target: Binding<Option<TargetInfo>>,
    /// Channels the target can produce.
    pub available: Binding<ChannelSet>,
    /// Channels currently subscribed.
    pub subscribed: Binding<ChannelSet>,
    /// Events lost, per channel, since the session began.
    pub dropped: Binding<u64>,

    /// Most recent frame.
    pub last_frame: Binding<Option<FrameSample>>,
    /// Frame durations, oldest first, for the timeline.
    pub frame_series: Binding<Vec<DataPoint>>,
    /// Frame budget in microseconds, for the timeline's reference line.
    pub frame_budget_us: Binding<u32>,
    /// Frames observed.
    pub frames_seen: Binding<u64>,

    /// Flattened accessibility tree.
    pub tree: Binding<Vec<TreeRow>>,
    /// Task occupancy for the last window.
    pub tasks: Binding<Vec<TaskRow>>,
    /// Recent log lines, newest last.
    pub logs: Binding<Vec<LogRow>>,
    /// Reactive-graph nodes, busiest first.
    pub signals: Binding<Vec<SignalRow>>,
    /// Recent stalls, newest first.
    pub stalls: Binding<Vec<StallRow>>,

    /// Bookkeeping that backs the reactive collections.
    tracking: Rc<RefCell<Tracking>>,
}

/// State that exists only to turn deltas into the collections above.
#[derive(Default)]
struct Tracking {
    tree_nodes: BTreeMap<NodeId, TreeNode>,
    tree_root: Option<NodeId>,
    signals: BTreeMap<SignalId, SignalRow>,
    next_row_id: u64,
}

impl Tracking {
    const fn next_id(&mut self) -> u64 {
        let id = self.next_row_id;
        self.next_row_id = self.next_row_id.wrapping_add(1);
        id
    }
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}

impl Model {
    /// Creates an empty model, before anything has connected.
    #[must_use]
    pub fn new() -> Self {
        Self {
            connection: Binding::container(Connection::Connecting),
            endpoint: Binding::container(Str::from("")),
            target: Binding::container(None),
            available: Binding::container(ChannelSet::empty()),
            subscribed: Binding::container(ChannelSet::default_subscription()),
            dropped: Binding::u64(0),
            last_frame: Binding::container(None),
            frame_series: Binding::container(Vec::new()),
            frame_budget_us: Binding::u32(0),
            frames_seen: Binding::u64(0),
            tree: Binding::container(Vec::new()),
            tasks: Binding::container(Vec::new()),
            logs: Binding::container(Vec::new()),
            signals: Binding::container(Vec::new()),
            stalls: Binding::container(Vec::new()),
            section: Binding::container(Some(Section::Overview)),
            revealed: Binding::container(None),
            tracking: Rc::new(RefCell::new(Tracking::default())),
        }
    }

    /// Shows `node`, because someone asked to inspect it in the application.
    ///
    /// Opens the tree, subscribes to it if it was not already — a request to
    /// see a node is a request for the channel that carries it — and marks the
    /// node so the pane can reveal it.
    pub fn apply_select(&self, node: u64) {
        self.subscribed
            .set(self.subscribed.get() | ChannelSet::TREE);
        self.section.set(Some(Section::Tree));
        self.revealed.set(Some(node));
    }

    /// Records a frame and advances the timeline.
    pub fn apply_frame(&self, sample: FrameSample) {
        self.frames_seen.set(self.frames_seen.get() + 1);
        self.frame_budget_us.set(sample.budget_us);

        let mut series = self.frame_series.get();
        if series.len() >= FRAME_HISTORY {
            series.remove(0);
        }
        // Re-index so the x axis is always "frames ago", independent of how many
        // frames the session has seen in total.
        #[expect(
            clippy::cast_precision_loss,
            reason = "a frame duration in microseconds is far inside f32's exact-integer range"
        )]
        series.push(DataPoint::new(0.0, sample.total_us as f32 / 1000.0));
        for (index, point) in series.iter_mut().enumerate() {
            #[expect(
                clippy::cast_precision_loss,
                reason = "the series is bounded by FRAME_HISTORY, far inside f32's exact-integer range"
            )]
            {
                point.x = index as f32;
            }
        }
        self.frame_series.set(series);
        self.last_frame.set(Some(sample));
    }

    /// Applies a tree update and re-flattens the display rows.
    pub fn apply_tree(&self, update: TreeUpdate) {
        {
            let mut tracking = self.tracking.borrow_mut();
            if update.full {
                tracking.tree_nodes.clear();
            }
            tracking.tree_root = Some(update.root);
            for node in update.changed {
                tracking.tree_nodes.insert(node.id, node);
            }
            for id in update.removed {
                tracking.tree_nodes.remove(&id);
            }
        }
        // `replace` notifies once; the collection then diffs by id, so an
        // unchanged row keeps its view rather than being rebuilt.
        self.tree.set(self.flatten_tree());
    }

    /// Walks the tree depth-first into indented display rows.
    fn flatten_tree(&self) -> Vec<TreeRow> {
        let tracking = self.tracking.borrow();
        let Some(root) = tracking.tree_root else {
            return Vec::new();
        };

        let mut rows = Vec::with_capacity(tracking.tree_nodes.len());
        let mut stack = vec![(root, 0_usize)];
        // A malformed tree must not hang the inspector, so a node is emitted at
        // most once however the backend wired its children.
        let mut visited = std::collections::BTreeSet::new();

        while let Some((id, depth)) = stack.pop() {
            let Some(node) = tracking.tree_nodes.get(&id) else {
                continue;
            };
            if !visited.insert(id) {
                continue;
            }
            rows.push(TreeRow {
                id: id.0,
                depth,
                role: Str::from(node.role.clone()),
                label: node.label.clone().map(Str::from),
                value: node.value.clone().map(Str::from),
                bounds: node.bounds.map(|bounds| {
                    Str::from(format!(
                        "{:.0}, {:.0}  {:.0}×{:.0}",
                        bounds.x, bounds.y, bounds.width, bounds.height
                    ))
                }),
                hidden: node.state.hidden,
                enabled: node.state.enabled,
            });
            for child in node.children.iter().rev() {
                stack.push((*child, depth + 1));
            }
        }
        rows
    }

    /// Replaces the task table with the newest window.
    pub fn apply_tasks(&self, window: TaskWindow) {
        let rows = window
            .tasks
            .into_iter()
            .map(|task| TaskRow {
                mean_us: task
                    .wall_us_total
                    .checked_div(u64::from(task.polls))
                    .unwrap_or(0),
                id: Str::from(short_type_name(&task.task_type)),
                polls: task.polls,
                max_us: task.wall_us_max,
                over_budget: task.over_budget,
            })
            .collect();
        self.tasks.set(rows);
    }

    /// Records a stall, newest first.
    pub fn apply_stall(&self, stall: StallSample) {
        let id = self.tracking.borrow_mut().next_id();
        let mut rows = self.stalls.get();
        rows.insert(
            0,
            StallRow {
                id,
                task_type: Str::from(short_type_name(&stall.task_type)),
                usage_pct: stall.usage_pct,
                wall_us: stall.wall_us,
                backtrace: stall.backtrace.into_iter().map(Str::from).collect(),
            },
        );
        rows.truncate(EVENT_HISTORY);
        self.stalls.set(rows);
    }

    /// Appends a log line, dropping the oldest once the buffer is full.
    pub fn apply_log(&self, record: LogRecord) {
        let id = self.tracking.borrow_mut().next_id();
        let mut rows = self.logs.get();
        if rows.len() >= EVENT_HISTORY {
            rows.remove(0);
        }
        rows.push(LogRow {
            id,
            level: record.level,
            target: Str::from(record.target),
            message: Str::from(record.message),
            origin: record
                .file
                .map(|file| Str::from(format!("{file}:{}", record.line.unwrap_or(0)))),
        });
        self.logs.set(rows);
    }

    /// Folds a reactive-graph event into the node table.
    pub fn apply_signal(&self, event: SignalEvent) {
        {
            let mut tracking = self.tracking.borrow_mut();
            match event {
                SignalEvent::Created(info) => {
                    tracking.signals.insert(
                        info.node,
                        SignalRow {
                            id: info.node.0,
                            type_name: Str::from(short_type_name(&info.type_name)),
                            origin: Str::from(format!(
                                "{}:{}:{}",
                                info.file, info.line, info.column
                            )),
                            subscribers: 0,
                            notifications: 0,
                            dropped: false,
                        },
                    );
                }
                SignalEvent::Subscribed { node, subscribers }
                | SignalEvent::Unsubscribed { node, subscribers } => {
                    if let Some(row) = tracking.signals.get_mut(&node) {
                        row.subscribers = subscribers;
                    }
                }
                SignalEvent::Notified { node, subscribers } => {
                    if let Some(row) = tracking.signals.get_mut(&node) {
                        row.subscribers = subscribers;
                        row.notifications = row.notifications.saturating_add(1);
                    }
                }
                SignalEvent::Dropped { node } => {
                    if let Some(row) = tracking.signals.get_mut(&node) {
                        row.dropped = true;
                    }
                }
            }
        }

        let mut rows: Vec<SignalRow> = self.tracking.borrow().signals.values().cloned().collect();
        // Busiest first: the reason to open this channel is to find what is
        // firing more than it should.
        rows.sort_by_key(|row| core::cmp::Reverse(row.notifications));
        rows.truncate(EVENT_HISTORY);
        self.signals.set(rows);
    }

    /// Records events the target could not deliver.
    pub fn apply_drop(&self, events: u64) {
        self.dropped.set(self.dropped.get().saturating_add(events));
    }

    /// Clears everything that belongs to a connection, keeping UI preferences.
    pub fn reset_stream(&self) {
        *self.tracking.borrow_mut() = Tracking::default();
        self.tree.set(Vec::new());
        self.tasks.set(Vec::new());
        self.signals.set(Vec::new());
        self.stalls.set(Vec::new());
        self.last_frame.set(None);
        self.frame_series.set(Vec::new());
    }
}

/// Trims a Rust type name down to something a table can show.
///
/// Fully-qualified generic future types run to hundreds of characters; the tail
/// is the part that identifies them.
fn short_type_name(name: &str) -> String {
    let trimmed = name.rsplit("::").next().unwrap_or(name);
    if trimmed.len() > 48 {
        format!("{}…", &trimmed[..47])
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{Model, short_type_name};
    use waterui_inspector_protocol::{Bounds, NodeId, NodeState, TreeNode, TreeUpdate};

    fn node(id: u64, children: &[u64]) -> TreeNode {
        TreeNode {
            id: NodeId(id),
            role: String::from("group"),
            label: None,
            value: None,
            bounds: Some(Bounds {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            }),
            state: NodeState::default(),
            children: children.iter().copied().map(NodeId).collect(),
        }
    }

    #[test]
    fn a_tree_flattens_depth_first_with_depth() {
        let model = Model::new();
        model.apply_tree(TreeUpdate {
            revision: 0,
            root: NodeId(1),
            focus: None,
            full: true,
            changed: vec![node(1, &[2, 3]), node(2, &[4]), node(3, &[]), node(4, &[])],
            removed: Vec::new(),
        });

        let rows = model.tree.get();
        assert_eq!(
            rows.iter()
                .map(|row| (row.id, row.depth))
                .collect::<Vec<_>>(),
            vec![(1, 0), (2, 1), (4, 2), (3, 1)]
        );
    }

    /// A backend that reports a cycle must not hang the inspector.
    #[test]
    fn a_cyclic_tree_terminates() {
        let model = Model::new();
        model.apply_tree(TreeUpdate {
            revision: 0,
            root: NodeId(1),
            focus: None,
            full: true,
            changed: vec![node(1, &[2]), node(2, &[1])],
            removed: Vec::new(),
        });

        assert_eq!(model.tree.get().len(), 2);
    }

    #[test]
    fn a_delta_updates_only_what_it_names() {
        let model = Model::new();
        model.apply_tree(TreeUpdate {
            revision: 0,
            root: NodeId(1),
            focus: None,
            full: true,
            changed: vec![node(1, &[2, 3]), node(2, &[]), node(3, &[])],
            removed: Vec::new(),
        });

        model.apply_tree(TreeUpdate {
            revision: 1,
            root: NodeId(1),
            focus: None,
            full: false,
            changed: vec![node(1, &[2])],
            removed: vec![NodeId(3)],
        });

        assert_eq!(
            model
                .tree
                .get()
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn long_type_names_are_trimmed_to_their_tail() {
        assert_eq!(short_type_name("core::future::Ready<i32>"), "Ready<i32>");
        assert_eq!(short_type_name("Short"), "Short");
        assert_eq!(short_type_name(&"x".repeat(100)).chars().count(), 48);
    }
}
