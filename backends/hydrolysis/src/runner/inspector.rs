//! Projection of Hydrolysis frame profiles into inspector events.
//!
//! Hydrolysis already measures every phase of a frame for its own render
//! diagnostics. The inspector is a second consumer of that same measurement, so
//! nothing new is timed here — the profile is translated and published.

use std::time::Duration;

use waterui::Environment;
use waterui::inspector::FrameRecorder;
use waterui_inspector_protocol::{FrameKind, FrameSample};

#[cfg(feature = "accessibility")]
use waterui::inspector::TreeRecorder;
#[cfg(feature = "accessibility")]
use waterui_inspector_protocol::{Bounds, NodeId, NodeState, TreeNode};

use super::window::{FrameMode, FrameProfile};

/// Publishes one frame, if an inspector is attached and wants frames.
pub(super) fn publish_frame(
    environment: &Environment,
    mode: FrameMode,
    profile: &FrameProfile,
    total: Duration,
) {
    let Some(recorder) = environment.get::<FrameRecorder>() else {
        return;
    };
    if !recorder.is_active() {
        return;
    }

    let phases = &profile.phases;
    let counters = &profile.counters;
    let refresh_hz = waterui::task::max_refresh_rate_hz();

    recorder.record(FrameSample {
        kind: frame_kind(mode),
        // Everything up to a laid-out tree: building the root view value,
        // dispatching it into scene state, and finishing layout.
        layout_us: micros(phases.build_content + phases.scene_dispatch + phases.scene_finish),
        // Acquiring the target and submitting encoded work.
        encode_us: micros(phases.acquire + phases.render),
        present_us: micros(phases.present),
        total_us: micros(total),
        budget_us: budget_us(refresh_hz),
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a display refresh rate is far inside f32 range"
        )]
        refresh_hz: refresh_hz as f32,
        layers: counters.scene_layers,
        clip_depth: counters.max_clip_depth,
    });
}

/// How much of the pipeline the frame ran, from the mode it was scheduled with.
const fn frame_kind(mode: FrameMode) -> FrameKind {
    match mode {
        FrameMode::Idle => FrameKind::Idle,
        FrameMode::Reencode => FrameKind::Reencode,
        FrameMode::Refresh => FrameKind::Refresh,
    }
}

/// Frame budget for a refresh rate, or `0` when the rate is unusable.
///
/// Zero means "unknown" to the protocol, which reports no budget usage rather
/// than pretending the frame was perfectly on budget.
fn budget_us(refresh_hz: f64) -> u32 {
    if refresh_hz <= 0.0 {
        return 0;
    }
    micros(Duration::from_secs_f64(1.0 / refresh_hz))
}

fn micros(duration: Duration) -> u32 {
    u32::try_from(duration.as_micros()).unwrap_or(u32::MAX)
}

/// Publishes the accessibility tree, if an inspector is attached and wants it.
///
/// Hydrolysis regenerates its whole tree whenever it changes, so the entire node
/// set is handed over and [`TreeRecorder`] turns it into a delta.
#[cfg(feature = "accessibility")]
pub(super) fn publish_tree(
    environment: &Environment,
    update: &accesskit::TreeUpdate,
) {
    let Some(recorder) = environment.get::<TreeRecorder>() else {
        return;
    };
    if !recorder.is_active() {
        return;
    }

    let root = update
        .tree
        .as_ref()
        .map_or(NodeId(0), |tree| NodeId(tree.root.0));
    let nodes = update
        .nodes
        .iter()
        .map(|(id, node)| project_node(*id, node))
        .collect();

    recorder.record_snapshot(root, Some(NodeId(update.focus.0)), nodes);
}

/// Projects one accesskit node into the backend-agnostic protocol shape.
#[cfg(feature = "accessibility")]
fn project_node(id: accesskit::NodeId, node: &accesskit::Node) -> TreeNode {
    let checked = match node.toggled() {
        Some(accesskit::Toggled::True) => Some(true),
        Some(accesskit::Toggled::False) => Some(false),
        // `Mixed` is neither checked nor unchecked, and saying "false" would be
        // a lie the inspector then displays.
        Some(accesskit::Toggled::Mixed) | None => None,
    };

    TreeNode {
        id: NodeId(id.0),
        role: format!("{:?}", node.role()).to_lowercase(),
        label: node.label().map(ToOwned::to_owned),
        value: node
            .value()
            .map(ToOwned::to_owned)
            .or_else(|| node.numeric_value().map(|value| value.to_string())),
        bounds: node.bounds().map(|rect| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "window-space point coordinates are far inside f32 range"
            )]
            Bounds {
                x: rect.x0 as f32,
                y: rect.y0 as f32,
                width: (rect.x1 - rect.x0) as f32,
                height: (rect.y1 - rect.y0) as f32,
            }
        }),
        state: NodeState {
            enabled: !node.is_disabled(),
            selected: node.is_selected().unwrap_or(false),
            checked,
            expanded: node.is_expanded(),
            hidden: node.is_hidden(),
        },
        children: node.children().iter().map(|id| NodeId(id.0)).collect(),
    }
}
