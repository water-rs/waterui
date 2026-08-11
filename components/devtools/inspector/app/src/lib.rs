//! The `WaterUI` Inspector.
//!
//! A development tool for `WaterUI` applications, written in `WaterUI`. It attaches
//! to a running app over a loopback socket and shows its frames, view tree,
//! main-thread occupancy, logs, and reactive graph.
//!
//! This crate is a real library rather than a scaffolding template, so it is
//! compiled, linted, and tested along with the rest of the workspace. A devtool
//! that the compiler cannot see is a devtool that rots.

use waterui::app::App;
use waterui::prelude::*;

pub mod connection;
pub mod model;
pub mod ui;

pub use model::Model;

/// The Inspector's root view, attached to the target named by the environment.
///
/// A missing or malformed target is shown in the UI rather than panicking: this
/// is a diagnostic tool, and dying silently behind an already-open window is the
/// least useful thing it could do.
pub fn main() -> impl View {
    let model = Model::new();
    let (sender, receiver) = connection::subscription_channel();
    let view = ui::inspector(model.clone(), sender);

    match connection::Target::from_env() {
        Ok(target) => view
            .task({
                let model = model;
                async move {
                    connection::run(model, target, receiver).await;
                }
            })
            .anyview(),
        Err(error) => {
            model
                .connection
                .set(model::Connection::Failed(Str::from(error)));
            view.anyview()
        }
    }
}

/// Application entry point.
#[must_use]
pub fn app(env: Environment) -> App {
    App::new(main, env)
}

/// A populated inspector, for previews and visual review.
///
/// Previews drive the same [`Model`] the live app does, so what is reviewed is
/// the real view code rather than a parallel mock of it.
#[must_use]
pub fn preview_model() -> Model {
    let model = Model::new();
    preview_connection(&model);
    preview_frames(&model);
    preview_tree(&model);
    preview_tasks(&model);
    preview_logs(&model);
    preview_signals(&model);
    model
}

fn preview_connection(model: &Model) {
    use waterui_inspector_protocol::{ChannelSet, TargetInfo};

    model.endpoint.set(Str::from("127.0.0.1:23106"));
    model.connection.set(model::Connection::Live);
    model.available.set(ChannelSet::all());
    model.subscribed.set(ChannelSet::all());
    model.target.set(Some(TargetInfo {
        pid: 81_406,
        name: String::from("Gallery"),
        backend: String::from("hydrolysis"),
    }));
}

/// A steady frame rate with one deliberate hitch, so the timeline shows both a
/// normal session and the thing you would actually be hunting.
fn preview_frames(model: &Model) {
    use waterui_inspector_protocol::{FrameKind, FrameSample};

    for index in 0..120_u32 {
        let steady = 2_400 + (index % 12) * 90;
        let total = if index == 96 { 19_800 } else { steady };
        model.apply_frame(FrameSample {
            kind: if index % 3 == 0 {
                FrameKind::Refresh
            } else {
                FrameKind::Reencode
            },
            layout_us: total / 3,
            encode_us: total / 3,
            present_us: total / 3,
            total_us: total,
            budget_us: 8_333,
            refresh_hz: 120.0,
            layers: 14,
            clip_depth: 3,
        });
    }
}

/// Includes one unlabelled control, which the tree pane should call out.
fn preview_tree(model: &Model) {
    use waterui_inspector_protocol::{NodeId, TreeUpdate};

    model.apply_tree(TreeUpdate {
        revision: 0,
        root: NodeId(1),
        focus: Some(NodeId(4)),
        full: true,
        changed: vec![
            preview_node(1, "window", Some("Gallery"), &[2]),
            preview_node(2, "group", None, &[3, 4, 5]),
            preview_node(3, "label", Some("Components"), &[]),
            preview_node(4, "button", Some("Buttons"), &[]),
            preview_node(5, "button", None, &[]),
        ],
        removed: Vec::new(),
    });
}

fn preview_tasks(model: &Model) {
    use waterui_inspector_protocol::{StallSample, TaskAggregate, TaskWindow};

    model.apply_tasks(TaskWindow {
        window_ms: 100,
        budget_us: 8_333,
        refresh_hz: 120.0,
        tasks: vec![
            TaskAggregate {
                task_type: String::from("hydrolysis::runner::FramePump"),
                polls: 12,
                ready: 0,
                wall_us_total: 41_200,
                wall_us_max: 19_800,
                cpu_us_total: 38_900,
                over_budget: 1,
            },
            TaskAggregate {
                task_type: String::from("waterui::media::VideoTick"),
                polls: 6,
                ready: 6,
                wall_us_total: 1_800,
                wall_us_max: 410,
                cpu_us_total: 1_600,
                over_budget: 0,
            },
        ],
    });

    model.apply_stall(StallSample {
        task_type: String::from("hydrolysis::runner::FramePump"),
        wall_us: 19_800,
        cpu_us: 19_100,
        budget_us: 8_333,
        usage_pct: 237.6,
        backtrace: vec![
            String::from("hydrolysis::renderer::refresh_window_frame"),
            String::from("waterui_layout::measure::text"),
        ],
    });
}

fn preview_logs(model: &Model) {
    use waterui_inspector_protocol::{LogLevel, LogRecord};

    for (level, message) in [
        (LogLevel::Info, "Gallery window opened"),
        (LogLevel::Debug, "Installed Material 3 theme"),
        (
            LogLevel::Warn,
            "Display refresh rate unavailable; using fallback",
        ),
        (LogLevel::Error, "Failed to decode asset `hero.png`"),
    ] {
        model.apply_log(LogRecord {
            level,
            target: String::from("waterui::gallery"),
            message: String::from(message),
            file: Some(String::from("gallery/src/lib.rs")),
            line: Some(142),
            fields: Vec::new(),
        });
    }
}

/// One node fires far more often than the others — the shape of the problem
/// this pane exists to reveal.
fn preview_signals(model: &Model) {
    use waterui_inspector_protocol::{SignalEvent, SignalId, SignalNodeInfo};

    for (index, (type_name, file, line, fired)) in [
        ("Container<Str>", "gallery/src/drawer.rs", 142_u32, 40_u32),
        ("Container<bool>", "gallery/src/lib.rs", 88, 12),
        ("List<Row>", "gallery/src/list.rs", 27, 3),
    ]
    .into_iter()
    .enumerate()
    {
        let node = SignalId(index as u64 + 1);
        model.apply_signal(SignalEvent::Created(SignalNodeInfo {
            node,
            type_name: String::from(type_name),
            file: String::from(file),
            line,
            column: 9,
        }));
        model.apply_signal(SignalEvent::Subscribed {
            node,
            subscribers: 3,
        });
        for _ in 0..fired {
            model.apply_signal(SignalEvent::Notified {
                node,
                subscribers: 3,
            });
        }
    }
}

fn preview_node(
    id: u64,
    role: &str,
    label: Option<&str>,
    children: &[u64],
) -> waterui_inspector_protocol::TreeNode {
    use waterui_inspector_protocol::{Bounds, NodeId, NodeState, TreeNode};

    TreeNode {
        id: NodeId(id),
        role: String::from(role),
        label: label.map(String::from),
        value: None,
        bounds: Some(Bounds {
            x: 16.0,
            #[expect(
                clippy::cast_precision_loss,
                reason = "fixture ids are single digits"
            )]
            y: 24.0 * id as f32,
            width: 320.0,
            height: 40.0,
        }),
        state: NodeState {
            enabled: true,
            selected: false,
            checked: None,
            expanded: None,
            hidden: false,
        },
        children: children.iter().copied().map(NodeId).collect(),
    }
}
