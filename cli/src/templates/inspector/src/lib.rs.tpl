use std::net::SocketAddr;
use std::time::Duration;

use futures_lite::io::BufReader;
use smol::net::TcpStream;
use waterui::app::App;
use waterui::id::{Id, TaggedView};
use waterui::navigation::tab::{Tab, TabPosition, Tabs};
use waterui::navigation::NavigationView;
use waterui::prelude::*;
use waterui::prelude::theme_color::{Background, Foreground, MutedForeground, Surface, SurfaceVariant};
use waterui::shape::RoundedRectangle;
use waterui::widget::condition::when;
use waterui_inspector_protocol::transport::{read_frame, write_frame};
use waterui_inspector_protocol::{InspectorClientMessage, InspectorEvent, InspectorServerMessage};

const TAB_OVERVIEW: i32 = 1;
const TAB_RUNTIME: i32 = 2;
const TAB_STALLS: i32 = 3;

const CHIP_LIVE: Srgb = Srgb::from_hex("#15803d");
const CHIP_RETRY: Srgb = Srgb::from_hex("#b45309");

const PANEL_RADIUS: f32 = 0.008;
const TAB_TRACK_RADIUS: f32 = 0.24;
const METRIC_RADIUS: f32 = 0.008;

fn tab_id(value: i32) -> Id {
    Id::try_from(value).unwrap_or_else(|_| panic!("tab id must be non-zero, got {value}"))
}

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
    active_tab: Binding<Id>,
}

impl InspectorState {
    fn runtime() -> Self {
        Self {
            status: Binding::container(Str::from("Waiting for target app...")),
            target: Binding::container(Str::from("-")),
            connected: Binding::bool(false),
            polls: Binding::i32(0),
            stalls: Binding::i32(0),
            app_pid: Binding::container(Str::from("-")),
            build_commit: Binding::container(Str::from("-")),
            last_poll: Binding::container(Str::from("-")),
            last_stall: Binding::container(Str::from("-")),
            last_error: Binding::container(Str::from("-")),
            attempts: Binding::i32(0),
            active_tab: Binding::container(tab_id(TAB_OVERVIEW)),
        }
    }

    fn preview() -> Self {
        Self {
            status: Binding::container(Str::from("Connected to 127.0.0.1:24123")),
            target: Binding::container(Str::from("127.0.0.1:24123")),
            connected: Binding::bool(true),
            polls: Binding::i32(1824),
            stalls: Binding::i32(3),
            app_pid: Binding::container(Str::from("81406")),
            build_commit: Binding::container(Str::from("bdee11e5a716...")),
            last_poll: Binding::container(Str::from(
                "waterui::layout::HStack | wall=274us cpu=261us budget=8333us @120Hz",
            )),
            last_stall: Binding::container(Str::from(
                "waterui::media::Video | 132.1% budget | overrun=1983us | bt=task::poll > renderer::tick",
            )),
            last_error: Binding::container(Str::from("-")),
            attempts: Binding::i32(4),
            active_tab: Binding::container(tab_id(TAB_OVERVIEW)),
        }
    }
}

fn main() -> impl View {
    let state = InspectorState::runtime();
    inspector_dashboard(state.clone()).task({
        let state_for_task = state.clone();
        async move {
            connect_forever(state_for_task).await;
        }
    })
}

#[preview]
fn inspector_style_preview() -> impl View {
    inspector_dashboard(InspectorState::preview())
}

fn inspector_dashboard(state: InspectorState) -> impl View {
    vstack((
        hero_card(state.clone()),
        metrics_row(state.clone()),
        inspector_tabs(state).height(560.0),
    ))
    .spacing(14.0)
    .padding_with(EdgeInsets::all(18.0))
    .background(Background)
    .ignore_safe_area(EdgeSet::ALL)
}

fn hero_card(state: InspectorState) -> impl View {
    hstack((
        vstack((
            text("WaterUI Inspector").title().bold().foreground(Foreground),
            text!("Target endpoint: {target}", target = state.target.clone())
                .caption()
                .foreground(MutedForeground),
            text!(
                "{connected}",
                connected = state.connected.select("Runtime connected", "Runtime retrying")
            )
                .caption()
                .foreground(MutedForeground),
        ))
        .spacing(4.0),
        spacer(),
        status_chip(state.connected),
    ))
    .spacing(14.0)
    .padding_with(EdgeInsets::all(16.0))
    .background(Surface)
    .clip(RoundedRectangle::new(PANEL_RADIUS))
}

fn metrics_row(state: InspectorState) -> impl View {
    hstack((
        metric_tile(
            "Runtime polls",
            text!("{polls}", polls = state.polls.clone()).title().bold(),
        ),
        metric_tile(
            "Stall alerts",
            text!("{stalls}", stalls = state.stalls.clone()).title().bold(),
        ),
        metric_tile(
            "Connect attempts",
            text!("{attempts}", attempts = state.attempts.clone()).title().bold(),
        ),
    ))
    .spacing(10.0)
}

fn inspector_tabs(state: InspectorState) -> impl View {
    let selection = state.active_tab.clone();
    let overview_state = state.clone();
    let runtime_state = state.clone();
    let stalls_state = state.clone();

    Tabs::new(
        selection,
        vec![
            Tab::new(
                TaggedView::new(
                    tab_id(TAB_OVERVIEW),
                    text("Overview").sub_headline().bold().anyview(),
                ),
                move || {
                    tab_navigation(
                        "Overview",
                        scroll(
                            overview_panel(overview_state.clone())
                                .padding_with(EdgeInsets::all(14.0)),
                        ),
                    )
                },
            ),
            Tab::new(
                TaggedView::new(
                    tab_id(TAB_RUNTIME),
                    text("Runtime").sub_headline().bold().anyview(),
                ),
                move || {
                    tab_navigation(
                        "Runtime",
                        scroll(runtime_panel(runtime_state.clone()).padding_with(EdgeInsets::all(14.0))),
                    )
                },
            ),
            Tab::new(
                TaggedView::new(
                    tab_id(TAB_STALLS),
                    text("Stalls").sub_headline().bold().anyview(),
                ),
                move || {
                    tab_navigation(
                        "Stalls",
                        scroll(stalls_panel(stalls_state.clone()).padding_with(EdgeInsets::all(14.0))),
                    )
                },
            ),
        ],
    )
    .position(TabPosition::Top)
    .background(Surface)
    .clip(RoundedRectangle::new(PANEL_RADIUS))
}

fn tab_navigation(title: &'static str, content: impl View) -> NavigationView {
    let mut navigation = NavigationView::new(title, content);
    navigation.bar.hidden = Computed::constant(true);
    navigation
}

fn overview_panel(state: InspectorState) -> impl View {
    vstack((
        panel(
            "Connection",
            vstack((
                detail_block("Connection state", text!("{status}", status = state.status.clone()).caption()),
                detail_block("Target endpoint", text!("{target}", target = state.target.clone()).caption()),
                detail_block("Last transport error", text!("{last_error}", last_error = state.last_error.clone()).caption()),
                hstack((
                    key_value(
                        "Connected",
                        text!("{connected}", connected = state.connected.clone()).body(),
                    ),
                    key_value("Attempts", text!("{attempts}", attempts = state.attempts.clone()).body()),
                ))
                .spacing(16.0),
            ))
            .spacing(10.0),
        ),
        panel(
            "Runtime Identity",
            vstack((
                hstack((
                    key_value("App PID", text!("{pid}", pid = state.app_pid.clone()).body()),
                    key_value("Runtime Build", text!("{commit}", commit = state.build_commit.clone()).caption()),
                ))
                .spacing(16.0),
                key_value(
                    "Session attempts",
                    text!("{attempts}", attempts = state.attempts.clone()).body(),
                ),
            ))
            .spacing(10.0),
        ),
    ))
    .spacing(12.0)
}

fn runtime_panel(state: InspectorState) -> impl View {
    vstack((
        panel(
            "Poll Latency Snapshot",
            vstack((
                detail_block("Last poll", text!("{poll}", poll = state.last_poll.clone()).caption()),
                key_value(
                    "Current status",
                    text!("{status}", status = state.status.clone()).caption(),
                ),
            ))
            .spacing(10.0),
        ),
        panel(
            "Runtime Counters",
            vstack((
                hstack((
                    metric_tile(
                        "Runtime polls",
                        text!("{polls}", polls = state.polls.clone()).title().bold(),
                    ),
                    metric_tile(
                        "Stall alerts",
                        text!("{stalls}", stalls = state.stalls.clone()).title().bold(),
                    ),
                ))
                .spacing(10.0),
                key_value("Attempts", text!("{attempts}", attempts = state.attempts.clone()).body()),
            ))
            .spacing(10.0),
        ),
    ))
    .spacing(12.0)
}

fn stalls_panel(state: InspectorState) -> impl View {
    vstack((
        panel(
            "Latest Stall",
            vstack((
                detail_block(
                    "Most recent",
                    text!("{last_stall}", last_stall = state.last_stall.clone()).caption(),
                ),
                key_value(
                    "Stall count",
                    text!("{stalls}", stalls = state.stalls.clone()).body(),
                ),
            ))
            .spacing(10.0),
        ),
        panel(
            "Transport Diagnostics",
            vstack((
                detail_block(
                    "Last error",
                    text!("{last_error}", last_error = state.last_error.clone()).caption(),
                ),
                key_value("Target", text!("{target}", target = state.target.clone()).caption()),
            ))
            .spacing(10.0),
        ),
    ))
    .spacing(12.0)
}

fn status_chip(connected: Binding<bool>) -> impl View {
    when(connected, || {
        text("LIVE")
            .caption()
            .bold()
            .foreground(Srgb::WHITE)
            .padding_with(EdgeInsets::symmetric(6.0, 12.0))
            .background(CHIP_LIVE)
            .clip(RoundedRectangle::new(TAB_TRACK_RADIUS))
    })
    .otherwise(|| {
        text("RETRYING")
            .caption()
            .bold()
            .foreground(Srgb::WHITE)
            .padding_with(EdgeInsets::symmetric(6.0, 12.0))
            .background(CHIP_RETRY)
            .clip(RoundedRectangle::new(TAB_TRACK_RADIUS))
    })
}

fn panel<T>(title: &'static str, body: T) -> impl View
where
    T: View,
{
    vstack((
        text(title).sub_headline().bold().foreground(Foreground),
        body,
    ))
    .spacing(8.0)
    .padding_with(EdgeInsets::all(14.0))
    .background(Surface)
    .clip(RoundedRectangle::new(PANEL_RADIUS))
}

fn key_value<T>(label: &'static str, value: T) -> impl View
where
    T: View,
{
    vstack((text(label).caption().foreground(MutedForeground), value.foreground(Foreground))).spacing(3.0)
}

fn detail_block<T>(title: &'static str, body: T) -> impl View
where
    T: View,
{
    vstack((
        text(title).caption().foreground(MutedForeground),
        body.foreground(Foreground),
    ))
    .spacing(6.0)
    .padding_with(EdgeInsets::all(10.0))
    .background(SurfaceVariant)
    .min_height(52.0)
    .clip(RoundedRectangle::new(METRIC_RADIUS))
}

fn metric_tile<T>(title: &'static str, value: T) -> impl View
where
    T: View,
{
    vstack((
        text(title).caption().foreground(MutedForeground),
        value.foreground(Foreground),
    ))
    .spacing(4.0)
    .padding_with(EdgeInsets::all(10.0))
    .min_width(130.0)
    .background(SurfaceVariant)
    .clip(RoundedRectangle::new(METRIC_RADIUS))
}

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
        match connect_and_stream(
            token.clone(),
            addr,
            state.clone(),
        )
        .await
        {
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
            state.last_error.set(Str::from("-"));
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
                        "{} | wall={}us cpu={}us budget={}us @{}Hz",
                        poll.task_type, poll.wall_us, poll.cpu_us, poll.budget_us, poll.refresh_hz
                    )));
                }
                InspectorEvent::MainThreadStall(stall) => {
                    state.stalls.set(state.stalls.get().saturating_add(1));
                    state.last_stall.set(Str::from(format!(
                        "{} | {:.1}% budget | overrun={}us | bt={}",
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
        .take(100)
        .collect()
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
