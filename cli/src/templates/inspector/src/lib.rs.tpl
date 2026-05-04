use std::net::SocketAddr;
use std::time::Duration;

use futures_lite::io::BufReader;
use smol::net::TcpStream;
use waterui::app::App;
use waterui::prelude::*;
use waterui::prelude::theme_color::{Background, Foreground, MutedForeground, Surface, SurfaceVariant};
use waterui::shape::{Capsule, RoundedRectangle};
use waterui::widget::condition::when;
use waterui_inspector_protocol::transport::{read_frame, write_frame};
use waterui_inspector_protocol::{InspectorClientMessage, InspectorEvent, InspectorServerMessage};

const LIVE: Srgb = Srgb::from_hex("#22c55e");
const RETRY: Srgb = Srgb::from_hex("#f59e0b");

const PAGE_PADDING: f32 = 18.0;
const SECTION_GAP: f32 = 14.0;
const CARD_PADDING: f32 = 18.0;
const CARD_GAP: f32 = 14.0;
const KPI_GAP: f32 = 10.0;

const CARD_RADIUS: f32 = 0.16;
const TILE_RADIUS: f32 = 0.14;

const KPI_TILE_HEIGHT: f32 = 96.0;

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
    reactive_dashboard(state.clone()).task({
        let state_for_task = state.clone();
        async move {
            connect_forever(state_for_task).await;
        }
    })
}

#[preview]
fn inspector_style_preview() -> impl View {
    static_dashboard(StaticSnapshot::live())
}

#[preview]
fn inspector_offline_preview() -> impl View {
    static_dashboard(StaticSnapshot::offline())
}

// ---- Layout shell --------------------------------------------------------

fn dashboard_shell<H, K, A, D>(hero: H, kpi: K, activity: A, diagnostics: D) -> impl View
where
    H: View,
    K: View,
    A: View,
    D: View,
{
    scroll(
        vstack((hero, kpi, activity, diagnostics))
            .spacing(SECTION_GAP)
            .padding_with(EdgeInsets::all(PAGE_PADDING)),
    )
    .background(Background)
    .ignore_safe_area(EdgeSet::ALL)
}

fn card<V: View>(content: V) -> impl View {
    content
        .padding_with(EdgeInsets::all(CARD_PADDING))
        .background(SurfaceVariant)
        .clip(RoundedRectangle::new(CARD_RADIUS))
}

fn eyebrow(label: &'static str) -> impl View {
    text(label).footnote().bold().foreground(MutedForeground)
}

fn pill(label: &'static str, color: Srgb) -> impl View {
    text(label)
        .footnote()
        .bold()
        .foreground(Srgb::WHITE)
        .padding_with(EdgeInsets::symmetric(6.0, 14.0))
        .background(color)
        .clip(Capsule)
}

fn kpi_tile_view<V: View>(label: &'static str, value: V) -> impl View {
    vstack((eyebrow(label), spacer(), value))
        .alignment(HorizontalAlignment::Leading)
        .spacing(6.0)
        .padding_with(EdgeInsets::all(14.0))
        .min_height(KPI_TILE_HEIGHT)
        .background(SurfaceVariant)
        .clip(RoundedRectangle::new(TILE_RADIUS))
}

fn event_block_view<V: View>(label: &'static str, body: V) -> impl View {
    vstack((eyebrow(label), body))
        .alignment(HorizontalAlignment::Leading)
        .spacing(6.0)
        .padding_with(EdgeInsets::all(12.0))
        .background(Surface)
        .clip(RoundedRectangle::new(TILE_RADIUS))
}

fn stat_row<V: View>(label: &'static str, value: V) -> impl View {
    hstack((
        text(label).caption().foreground(MutedForeground),
        spacer(),
        value,
    ))
    .spacing(8.0)
}

fn hero_layout<E, P>(endpoint: E, status: P) -> impl View
where
    E: View,
    P: View,
{
    card(
        hstack((
            vstack((
                eyebrow("WATERUI"),
                text("Inspector")
                    .sub_headline()
                    .bold()
                    .foreground(Foreground),
                endpoint,
            ))
            .alignment(HorizontalAlignment::Leading)
            .spacing(4.0),
            spacer(),
            status,
        ))
        .alignment(VerticalAlignment::Center)
        .spacing(12.0),
    )
}

fn kpi_strip_view<P, S, A>(polls: P, stalls: S, attempts: A) -> impl View
where
    P: View,
    S: View,
    A: View,
{
    hstack((
        kpi_tile_view("Polls", polls),
        kpi_tile_view("Stalls", stalls),
        kpi_tile_view("Retries", attempts),
    ))
    .spacing(KPI_GAP)
}

fn activity_card_view<I, P, S>(indicator: I, last_poll: P, last_stall: S) -> impl View
where
    I: View,
    P: View,
    S: View,
{
    card(
        vstack((
            section_header("Live activity", indicator),
            event_block_view("LAST POLL", last_poll),
            event_block_view("LAST STALL", last_stall),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(CARD_GAP),
    )
}

fn section_header<I: View>(title: &'static str, indicator: I) -> impl View {
    hstack((
        text(title).sub_headline().bold().foreground(Foreground),
        spacer(),
        indicator,
    ))
    .spacing(8.0)
}

fn diagnostics_card_view<S, P, B, E>(status: S, pid: P, build: B, error: E) -> impl View
where
    S: View,
    P: View,
    B: View,
    E: View,
{
    card(
        vstack((
            text("Diagnostics").sub_headline().bold().foreground(Foreground),
            stat_row("Status", status),
            stat_row("App PID", pid),
            stat_row("Runtime build", build),
            stat_row("Last error", error),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(10.0),
    )
}

// ---- Reactive (runtime) dashboard ---------------------------------------

fn reactive_dashboard(state: InspectorState) -> impl View {
    let endpoint = text!("{endpoint}", endpoint = state.target.clone())
        .caption()
        .foreground(MutedForeground);

    let polls = text!("{n}", n = state.polls.clone())
        .headline()
        .bold()
        .foreground(Foreground);
    let stalls = text!("{n}", n = state.stalls.clone())
        .headline()
        .bold()
        .foreground(Foreground);
    let attempts = text!("{n}", n = state.attempts.clone())
        .headline()
        .bold()
        .foreground(Foreground);

    let last_poll = text!("{v}", v = state.last_poll.clone())
        .body()
        .foreground(Foreground);
    let last_stall = text!("{v}", v = state.last_stall.clone())
        .body()
        .foreground(Foreground);

    let status_text = text!("{v}", v = state.status.clone())
        .body()
        .foreground(Foreground);
    let pid_text = text!("{v}", v = state.app_pid.clone())
        .body()
        .foreground(Foreground);
    let build_text = text!("{v}", v = state.build_commit.clone())
        .body()
        .foreground(MutedForeground);
    let error_text = text!("{v}", v = state.last_error.clone())
        .body()
        .foreground(MutedForeground);

    dashboard_shell(
        hero_layout(endpoint, reactive_status_pill(state.connected.clone())),
        kpi_strip_view(polls, stalls, attempts),
        activity_card_view(
            reactive_stream_indicator(state.connected),
            last_poll,
            last_stall,
        ),
        diagnostics_card_view(status_text, pid_text, build_text, error_text),
    )
}

fn reactive_status_pill(connected: Binding<bool>) -> impl View {
    when(connected, || pill("LIVE", LIVE)).otherwise(|| pill("RETRYING", RETRY))
}

fn reactive_stream_indicator(connected: Binding<bool>) -> impl View {
    when(connected, || stream_indicator("STREAMING", LIVE))
        .otherwise(|| stream_indicator("PAUSED", RETRY))
}

fn stream_indicator(label: &'static str, color: Srgb) -> impl View {
    text(label).footnote().bold().foreground(color)
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
            status: "Streaming live events",
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

fn static_dashboard(snap: StaticSnapshot) -> impl View {
    let (pill_label, pill_color, indicator_label, indicator_color) = if snap.connected {
        ("LIVE", LIVE, "STREAMING", LIVE)
    } else {
        ("RETRYING", RETRY, "PAUSED", RETRY)
    };

    dashboard_shell(
        hero_layout(
            text(snap.endpoint).caption().foreground(MutedForeground),
            pill(pill_label, pill_color),
        ),
        kpi_strip_view(
            static_kpi_value(snap.polls),
            static_kpi_value(snap.stalls),
            static_kpi_value(snap.attempts),
        ),
        activity_card_view(
            stream_indicator(indicator_label, indicator_color),
            text(snap.last_poll).body().foreground(Foreground),
            text(snap.last_stall).body().foreground(Foreground),
        ),
        diagnostics_card_view(
            text(snap.status).body().foreground(Foreground),
            text(snap.app_pid).body().foreground(Foreground),
            text(snap.build_commit).body().foreground(MutedForeground),
            text(snap.last_error).body().foreground(MutedForeground),
        ),
    )
}

fn static_kpi_value(value: i32) -> impl View {
    text(value.to_string())
        .headline()
        .bold()
        .foreground(Foreground)
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
