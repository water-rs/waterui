use std::net::SocketAddr;
use std::time::Duration;

use futures_lite::io::BufReader;
use smol::net::TcpStream;
use waterui::app::App;
use waterui::component::list::{List, Section, row, detail_row};
use waterui::icon::SystemIcon;
use waterui::prelude::*;
use waterui::prelude::theme_color::{Accent, Background, MutedForeground};
use waterui_inspector_protocol::transport::{read_frame, write_frame};
use waterui_inspector_protocol::{InspectorClientMessage, InspectorEvent, InspectorServerMessage};

#[derive(Clone)]
struct InspectorState {
    target: Binding<Str>,
    status: Binding<Str>,
    connected: Binding<bool>,
    polls: Binding<i32>,
    stalls: Binding<i32>,
    app_pid: Binding<Str>,
    build_commit: Binding<Str>,
    last_poll: Binding<Str>,
    last_stall: Binding<Str>,
    last_error: Binding<Str>,
    attempts: Binding<i32>,
}

impl InspectorState {
    fn runtime() -> Self {
        Self {
            status: Binding::container(Str::from("Waiting for target app")),
            target: Binding::container(Str::from("—")),
            connected: Binding::bool(false),
            polls: Binding::i32(0),
            stalls: Binding::i32(0),
            app_pid: Binding::container(Str::from("—")),
            build_commit: Binding::container(Str::from("—")),
            last_poll: Binding::container(Str::from("Awaiting first poll")),
            last_stall: Binding::container(Str::from("No stalls reported")),
            last_error: Binding::container(Str::from("—")),
            attempts: Binding::i32(0),
        }
    }
}

fn main() -> impl View {
    let state = InspectorState::runtime();
    inspector_view(reactive_snapshot(state.clone())).task({
        let state_for_task = state.clone();
        async move {
            connect_forever(state_for_task).await;
        }
    })
}

#[preview]
fn inspector_style_preview() -> impl View {
    inspector_view(snapshot_for(StaticSnapshot::live()))
}

#[preview]
fn inspector_offline_preview() -> impl View {
    inspector_view(snapshot_for(StaticSnapshot::offline()))
}

// ---- View ---------------------------------------------------------------

fn inspector_view(snap: Snapshot) -> NavigationView {
    NavigationView::new(
        "Inspector",
        List::content((
            Section::new("Connection").content((
                row(label("Status").system_icon(SystemIcon::from_static("wifi")), snap.connection_label)
                    .value_color(Accent),
                row(label("Endpoint").system_icon(SystemIcon::from_static("network")), snap.endpoint),
                row(label("Detail").system_icon(SystemIcon::from_static("info.circle")), snap.status),
            )),
            Section::new("Activity").content((
                row(label("Polls").system_icon(SystemIcon::from_static("arrow.triangle.2.circlepath")), snap.polls)
                    .value_color(MutedForeground),
                row(label("Stalls").system_icon(SystemIcon::from_static("exclamationmark.triangle")), snap.stalls)
                    .value_color(MutedForeground),
                row(label("Connect attempts").system_icon(SystemIcon::from_static("arrow.clockwise")), snap.attempts)
                    .value_color(MutedForeground),
            )),
            Section::new("Runtime").content((
                row(label("App PID").system_icon(SystemIcon::from_static("number")), snap.app_pid)
                    .value_color(MutedForeground),
                row(label("Build").system_icon(SystemIcon::from_static("hammer")), snap.build_commit)
                    .value_color(MutedForeground),
            )),
            Section::new("Recent events").content((
                detail_row(label("Last poll").system_icon(SystemIcon::from_static("clock")), snap.last_poll),
                detail_row(label("Last stall").system_icon(SystemIcon::from_static("exclamationmark.octagon")), snap.last_stall),
                detail_row(label("Last error").system_icon(SystemIcon::from_static("xmark.octagon")), snap.last_error),
            )),
        ))
        .background(Background),
    )
}

/// Pre-resolved reactive (or static) text streams the inspector list renders.
/// The same `Snapshot` shape feeds both the runtime app and previews — the
/// difference is just where the `Computed<Str>` values come from.
struct Snapshot {
    connection_label: Computed<Str>,
    endpoint: Computed<Str>,
    status: Computed<Str>,
    polls: Computed<Str>,
    stalls: Computed<Str>,
    attempts: Computed<Str>,
    app_pid: Computed<Str>,
    build_commit: Computed<Str>,
    last_poll: Computed<Str>,
    last_stall: Computed<Str>,
    last_error: Computed<Str>,
}

// ---- Reactive (runtime) -------------------------------------------------

fn reactive_snapshot(state: InspectorState) -> Snapshot {
    let connection_label: Computed<Str> = state
        .connected
        .clone()
        .select(Str::from("Live"), Str::from("Retrying"))
        .computed();

    Snapshot {
        connection_label,
        endpoint: state.target.clone().computed(),
        status: state.status.clone().computed(),
        polls: int_to_text(state.polls.clone()),
        stalls: int_to_text(state.stalls.clone()),
        attempts: int_to_text(state.attempts.clone()),
        app_pid: state.app_pid.clone().computed(),
        build_commit: state.build_commit.clone().computed(),
        last_poll: state.last_poll.clone().computed(),
        last_stall: state.last_stall.clone().computed(),
        last_error: state.last_error.clone().computed(),
    }
}

fn int_to_text(binding: Binding<i32>) -> Computed<Str> {
    binding.map(|n| Str::from(n.to_string())).computed()
}

// ---- Static preview snapshot --------------------------------------------

struct StaticSnapshot {
    endpoint: &'static str,
    connected: bool,
    polls: i32,
    stalls: i32,
    attempts: i32,
    last_poll: &'static str,
    last_stall: &'static str,
    status: &'static str,
    app_pid: &'static str,
    build_commit: &'static str,
    last_error: &'static str,
}

impl StaticSnapshot {
    const fn live() -> Self {
        Self {
            endpoint: "127.0.0.1:24123",
            connected: true,
            polls: 1824,
            stalls: 3,
            attempts: 4,
            last_poll: "waterui::layout::HStack · 274µs wall · 261µs cpu · 8333µs budget @120Hz",
            last_stall: "waterui::media::Video · 132.1% of budget · overrun 1983µs",
            status: "Connected",
            app_pid: "81406",
            build_commit: "bdee11e5a716",
            last_error: "—",
        }
    }

    const fn offline() -> Self {
        Self {
            endpoint: "—",
            connected: false,
            polls: 0,
            stalls: 0,
            attempts: 1,
            last_poll: "Awaiting first poll",
            last_stall: "No stalls reported",
            status: "Waiting for target app",
            app_pid: "—",
            build_commit: "—",
            last_error: "—",
        }
    }
}

fn snapshot_for(snap: StaticSnapshot) -> Snapshot {
    Snapshot {
        connection_label: constant(if snap.connected { "Live" } else { "Retrying" }),
        endpoint: constant(snap.endpoint),
        status: constant(snap.status),
        polls: constant_int(snap.polls),
        stalls: constant_int(snap.stalls),
        attempts: constant_int(snap.attempts),
        app_pid: constant(snap.app_pid),
        build_commit: constant(snap.build_commit),
        last_poll: constant(snap.last_poll),
        last_stall: constant(snap.last_stall),
        last_error: constant(snap.last_error),
    }
}

fn constant(value: &'static str) -> Computed<Str> {
    Computed::constant(Str::from(value))
}

fn constant_int(value: i32) -> Computed<Str> {
    Computed::constant(Str::from(value.to_string()))
}

// ---- Networking ----------------------------------------------------------

async fn connect_forever(state: InspectorState) {
    const RETRY_DELAY: Duration = Duration::from_millis(1000);

    let runtime_target = std::env::var("WATERUI_INSPECTOR_TARGET_ADDR")
        .expect("missing WATERUI_INSPECTOR_TARGET_ADDR");
    let token = std::env::var("WATERUI_INSPECTOR_TOKEN")
        .expect("missing WATERUI_INSPECTOR_TOKEN");
    let addr: SocketAddr = runtime_target
        .parse()
        .unwrap_or_else(|e| panic!("invalid target address `{runtime_target}`: {e}"));

    let target_label = Str::from(addr.to_string());
    state.target.set(target_label.clone());

    loop {
        let next_attempt = state.attempts.get().saturating_add(1);
        state.attempts.set(next_attempt);
        state.connected.set(false);
        match connect_and_stream(token.clone(), addr, state.clone()).await {
            Ok(()) => {
                state.last_error.set(Str::from("stream closed"));
                state.status.set(Str::from("Retrying in 1s"));
            }
            Err(error) => {
                state.last_error.set(Str::from(error.clone()));
                state.status.set(Str::from("Retrying in 1s"));
            }
        }
        smol::Timer::after(RETRY_DELAY).await;
    }
}

async fn connect_and_stream(
    token: String,
    addr: SocketAddr,
    state: InspectorState,
) -> Result<(), String> {
    let stream = TcpStream::connect(addr)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    let _ = stream.set_nodelay(true);

    let mut writer = stream.clone();
    write_frame(
        &mut writer,
        &InspectorClientMessage::Hello {
            token: token.clone(),
            from_seq: None,
        },
    )
    .await
    .map_err(|e| format!("failed to send hello: {e}"))?;

    let mut reader = BufReader::new(stream);
    match read_frame::<_, InspectorServerMessage>(&mut reader)
        .await
        .map_err(|e| format!("failed to read handshake: {e}"))?
    {
        InspectorServerMessage::HelloAck { protocol, app_pid: pid } => {
            state.connected.set(true);
            state.last_error.set(Str::from("—"));
            state.app_pid.set(Str::from(pid.to_string()));
            state.build_commit.set(Str::from(protocol.build_commit));
            state.status.set(Str::from("Connected"));
        }
        InspectorServerMessage::Error { message } => {
            return Err(message);
        }
        other => {
            return Err(format!("unexpected handshake response: {other:?}"));
        }
    }

    loop {
        match read_frame::<_, InspectorServerMessage>(&mut reader).await {
            Ok(InspectorServerMessage::Event { envelope }) => match envelope.event {
                InspectorEvent::RuntimePoll(poll) => {
                    state.polls.set(state.polls.get().saturating_add(1));
                    state.last_poll.set(Str::from(format!(
                        "{} · {}µs wall · {}µs cpu · {}µs budget @{}Hz",
                        poll.task_type, poll.wall_us, poll.cpu_us, poll.budget_us, poll.refresh_hz
                    )));
                }
                InspectorEvent::MainThreadStall(stall) => {
                    state.stalls.set(state.stalls.get().saturating_add(1));
                    state.last_stall.set(Str::from(format!(
                        "{} · {:.1}% of budget · overrun {}µs · {}",
                        stall.task_type,
                        stall.usage_pct,
                        stall.overrun_us,
                        summarize_backtrace(stall.backtrace.as_deref())
                    )));
                }
            },
            Ok(InspectorServerMessage::Pong) => {}
            Ok(InspectorServerMessage::Error { message }) => {
                state.last_error.set(Str::from(format!("server error: {message}")));
                return Err(message);
            }
            Ok(InspectorServerMessage::HelloAck { .. }) => {}
            Err(error) => {
                state.connected.set(false);
                return Err(format!("stream closed: {error}"));
            }
        }
    }
}

fn summarize_backtrace(backtrace: Option<&str>) -> String {
    let Some(backtrace) = backtrace else {
        return "n/a".to_string();
    };
    backtrace
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("n/a")
        .chars()
        .take(80)
        .collect()
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
