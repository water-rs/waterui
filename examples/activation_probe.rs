//! Activation-policy probe for water-rs/waterui#1302.
//!
//! The window starts [`WindowState::Closed`]; `StayResident` keeps the
//! process alive with zero windows so another app (TextEdit in the test
//! harness) can be frontmost when the probe finally shows. A timer flips
//! the presentation state to `Normal` — the same show path a drop-down
//! terminal's toggle uses — and every re-show mounts a fresh window, so
//! selector resolution and the activation policy run each time. A
//! `TextField` echoes keystrokes so an observer can tell where key events
//! land.
//!
//! Environment:
//!   PROBE_ACTIVATION   onshow | onclick | never   (default onshow)
//!   PROBE_SELECTOR     primary | pointer | focused (default primary)
//!   PROBE_SHOW_AFTER   seconds until the window opens (default 2.5)
//!   PROBE_START_OPEN   open the window at launch (initial state Normal
//!                      instead of Closed) — exercises launch activation
//!                      with a window mounted at startup
//!   PROBE_CYCLE        reopen the window every N seconds after the first
//!                      show instead of showing once (default unset)
//!   PROBE_EXIT_AFTER   self-exit after N seconds (default 3600)

use std::thread;
use std::time::Duration;

use hydrolysis::run;
use waterui::app::{App, LastWindowPolicy};
use waterui::layout::{Point, Rect, Size};
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::shape::{RoundedRectangle, ShapeExt};
use waterui::window::{
    Activation, MonitorSelector, Window, WindowPresentation, WindowState, conditional_window,
};

fn env_secs(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(default)
}

fn main_view(policy: Activation, echoed: Binding<Str>) -> impl View {
    let label = match policy {
        Activation::OnShow => "activation: on-show (activates on show)",
        Activation::OnClick => "activation: on-click (activates on click)",
        Activation::Never => "activation: never (never takes focus)",
    };
    vstack((
        text("activation-probe").size(24.0),
        text(label).size(14.0),
        text("keys typed here echo below:").size(12.0),
        TextField::new("type something", &echoed),
        RoundedRectangle::new(0.2)
            .fill(Color::srgb_hex("#7C3AED"))
            .size(320.0, 120.0),
    ))
    .spacing(16.0)
    .padding()
    .background(Color::srgb_hex("#F3E8FF"))
    .foreground(Color::srgb_hex("#1E1B4B"))
}

fn main() {
    let policy = match std::env::var("PROBE_ACTIVATION")
        .unwrap_or_else(|_| "onshow".into())
        .as_str()
    {
        "onshow" => Activation::OnShow,
        "onclick" => Activation::OnClick,
        "never" => Activation::Never,
        other => panic!("PROBE_ACTIVATION must be onshow|onclick|never, got {other}"),
    };
    let selector = match std::env::var("PROBE_SELECTOR")
        .unwrap_or_else(|_| "primary".into())
        .as_str()
    {
        "primary" => MonitorSelector::Primary,
        "pointer" => MonitorSelector::Pointer,
        "focused" => MonitorSelector::Focused,
        other => panic!("PROBE_SELECTOR must be primary|pointer|focused, got {other}"),
    };

    let state = binding(if std::env::var("PROBE_START_OPEN").is_ok() {
        WindowState::Normal
    } else {
        WindowState::Closed
    });
    let presentation = WindowPresentation::new(&state);
    let show_after = Duration::from_secs_f64(env_secs("PROBE_SHOW_AFTER", 2.5));
    let cycle = std::env::var("PROBE_CYCLE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .map(Duration::from_secs_f64);
    thread::spawn(move || {
        thread::sleep(Duration::from_secs_f64(env_secs(
            "PROBE_EXIT_AFTER",
            3600.0,
        )));
        std::process::exit(0);
    });

    let echoed = binding(Str::from(""));
    let creator_echoed = echoed.clone();
    // The host mounts the view tree that owns `conditional_window`. It
    // carries `Activation::Never` so its own mount — via `orderFront:` on
    // macOS — can never activate the app: every launch/show/keystroke
    // result below measures the POLICY window alone, with no default
    // OnShow window activating at startup.
    let host = Window::new("probe-host", binding(WindowState::Normal), move || {
        // `Binding` is not Send; its mailbox runs the mutation on the
        // binding's executor thread, the sanctioned cross-thread write
        // path. The mailbox is created here — inside the view builder —
        // because the local executor exists only on the runner thread.
        let mailbox = presentation.state().mailbox();
        thread::spawn(move || {
            thread::sleep(show_after);
            mailbox.handle(|b| b.set(WindowState::Normal));
            if let Some(period) = cycle {
                loop {
                    thread::sleep(period);
                    mailbox.handle(|b| b.set(WindowState::Closed));
                    thread::sleep(period);
                    mailbox.handle(|b| b.set(WindowState::Normal));
                }
            }
        });
        let creator_echoed = creator_echoed.clone();
        vstack((
            text("probe-host (never activates)").size(11.0),
            conditional_window(&presentation, move |state| {
                let echoed = creator_echoed.clone();
                Window::new("activation-probe", state, move || {
                    main_view(policy, echoed.clone())
                })
                .placement(MonitorSelector::clone(&selector), |monitor| {
                    // Dock at the top of the resolved monitor's visible
                    // frame: full width, 45% of visible height — the
                    // quick-terminal geometry hydroterm computes.
                    let vf = &monitor.visible_frame;
                    Rect::new(
                        Point::new(vf.origin().x, vf.origin().y),
                        Size::new(vf.size().width, vf.size().height * 0.45),
                    )
                })
                .activation(policy)
            }),
        ))
    })
    .activation(Activation::Never);
    run(
        App::new_with_windows([host], Environment::new())
            .on_last_window_closed(LastWindowPolicy::StayResident),
        hydrolysis_m3::Material3::defaults(),
    );
}
