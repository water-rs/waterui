//! Competitive benchmark app (water-rs/waterui#1262, harness #1864) — the
//! single WaterUI contestant every platform leg builds:
//!
//! - W1 Hello:  one centred label and one button that increments a counter.
//! - W2 Feed: lazy list of 10,000 rows (avatar circle, two text lines,
//!   trailing timestamp).
//! - W3 Motion: 200 independently animated rounded rectangles (position,
//!   rotation, opacity) running continuously.
//! - W4 Text: scrolling screen of 50 paragraphs of mixed Latin/CJK/emoji.
//! - W5 Motion capacity: the W3 scene stepped geometrically 200→25600 rects.
//! - W6 Feed capacity: the W2 fling program with per-row nested
//!   text+shape child count stepped 1→64.
//!
//! Workload selection accepts either channel a leg can deliver:
//! `-bench-workload W1|W2|W3|W4|W5|W6` in argv (Apple legs, the
//! NSArgumentDomain convention) or `BENCH_WORKLOAD` in the environment
//! (desktop legs and Android's `waterui.env.*` intent-extra forwarding);
//! values match case-insensitively. `-bench-drive swipe|auto` /
//! `BENCH_DRIVE` selects the drive: `auto` makes the app run the shared
//! fling program itself, but only after the runner posts `dev.bench.begin`
//! (Apple Darwin-notification handshake) inside the measurement window —
//! never at appear. A missing or unrecognized workload traps; there is no
//! fallback. On Apple targets the selected id also posts
//! `dev.bench.ready.<bundle>.<W>` and rides the accessibility identifier
//! `bench-workload-<id>` so the runner can assert it.
//!
//! W5/W6 have two pacing modes: `BENCH_STEP` in the environment pins one
//! ladder step per launch (the Android leg's protocol, which measures each
//! step as a separate launch); without it, `drive=auto` walks the whole
//! ladder inside the measure window with `dev.bench.step` posts and a
//! bench-steps.log the runner slices by.

use std::time::Duration;
use waterui::Identifiable;
use waterui::animation::Animation;
use waterui::app::App;
#[cfg(any(target_os = "ios", target_os = "macos"))]
use waterui::component::Dynamic;
use waterui::component::list::List;
use waterui::layout::Point;
use waterui::prelude::*;
use waterui::reactive::collection::List as ReactiveList;
use waterui::shape::{Circle, RoundedRectangle, ShapeExt};
use waterui::task::{sleep, spawn_local};

const FEED_ROWS: u64 = 10_000;
const MOTION_RECTS: usize = 200;
const PARAGRAPH_COUNT: usize = 50;

/// Rects wander inside a fixed logical field shared by every contestant.
const FIELD_W: f32 = 720.0;
const FIELD_H: f32 = 440.0;
const RECT_SIZE: f32 = 40.0;

/// Reads `-bench-<name> <value>` from argv — the launch-argument channel
/// the Apple legs drive — falling back to `BENCH_<NAME>` in the process
/// environment, the channel Android's `waterui.env.*` forwarding and the
/// desktop legs use.
fn bench_arg(name: &str) -> Option<String> {
    let key = format!("-bench-{name}");
    let mut args = std::env::args();
    if args.by_ref().any(|a| a == key)
        && let Some(v) = args.next()
    {
        return Some(v);
    }
    std::env::var(format!("BENCH_{}", name.to_ascii_uppercase())).ok()
}

fn workload() -> &'static str {
    match bench_arg("workload")
        .map(|v| v.to_ascii_uppercase())
        .as_deref()
    {
        Some("W1") => "W1",
        Some("W2") => "W2",
        Some("W3") => "W3",
        Some("W4") => "W4",
        Some("W5") => "W5",
        Some("W6") => "W6",
        Some(other) => {
            panic!("unrecognized bench workload value {other:?}; expected W1..=W6")
        }
        None => panic!("missing bench workload (expected -bench-workload or BENCH_WORKLOAD)"),
    }
}

/// `drive=auto`: the app scrolls itself through the same program the
/// external driver performs (8 flings down, 2 back up), but only once the
/// runner signals `dev.bench.begin` during the measure block. The handshake
/// uses Darwin notifications, not accessibility: a workload can stall the
/// AX server for tens of seconds while it materializes (the 10k-row feed),
/// and a timed-out AX query would fail the test instead of driving it.
#[cfg(any(target_os = "ios", target_os = "macos"))]
fn drive_auto() -> bool {
    match bench_arg("drive").as_deref() {
        None | Some("swipe") => false,
        Some("auto") => true,
        Some(other) => panic!("unrecognized bench drive value {other:?}; expected swipe|auto"),
    }
}

/// The Android leg's capacity protocol: one launch per ladder step, the
/// step parameter arriving as `BENCH_STEP` in the environment (forwarded
/// from the `waterui.env.BENCH_STEP` intent extra).
fn bench_step() -> Option<u32> {
    std::env::var("BENCH_STEP").ok()?.parse().ok()
}

/// Darwin-notification handshake for the `auto` drive. The runner posts
/// `dev.bench.begin` inside its `measure` block and waits for the app to
/// post `dev.bench.done` after the scroll program finishes.
#[cfg(any(target_os = "ios", target_os = "macos"))]
mod bench_notify {
    use std::ffi::c_char;
    #[link(name = "System")]
    unsafe extern "C" {
        fn notify_post(name: *const c_char) -> u32;
        fn notify_register_check(name: *const c_char, out_token: *mut i32) -> u32;
        fn notify_check(token: i32, check: *mut i32) -> u32;
    }
    const BEGIN: &[u8] = b"dev.bench.begin\0";
    const DONE: &[u8] = b"dev.bench.done\0";
    const ACK: &[u8] = b"dev.bench.ack\0";

    /// Registers for `dev.bench.begin`. None = registration failed (the
    /// drive never fires and the runner's wait times out loudly). The first
    /// `notify_check` reports the flag's CURRENT state, so a stale post from
    /// a previous run would look like a fresh signal — consume it here so
    /// only a begin posted after registration counts.
    pub fn register_begin() -> Option<i32> {
        let mut token = 0i32;
        unsafe {
            if notify_register_check(BEGIN.as_ptr() as _, &mut token) != 0 {
                return None;
            }
            let mut stale = 0i32;
            notify_check(token, &mut stale);
            Some(token)
        }
    }
    /// True once the runner has posted `dev.bench.begin`.
    pub fn begin_posted(token: i32) -> bool {
        let mut fired = 0i32;
        unsafe { notify_check(token, &mut fired) == 0 && fired != 0 }
    }
    /// Consumes a `begin` that latched while a program was running (a repost
    /// that raced the ack). The runner stops posting once it sees `done`, so
    /// anything already latched at re-arm time is backlog, not a new signal.
    pub fn discard_latched_begin(token: i32) {
        let mut fired = 0i32;
        unsafe { notify_check(token, &mut fired) };
        if fired != 0 {
            crate::drive_log("discarded stale begin at re-arm");
        }
    }
    /// Signals `dev.bench.done` to the runner.
    pub fn post_done() {
        unsafe {
            notify_post(DONE.as_ptr() as _);
        }
    }
    /// Signals `dev.bench.ack` — the program has started, stop reposting.
    pub fn post_ack() {
        unsafe {
            notify_post(ACK.as_ptr() as _);
        }
    }
    /// Signals `dev.bench.step` — posted at each capacity-ladder step start.
    pub fn post_step() {
        unsafe {
            notify_post(b"dev.bench.step\0".as_ptr() as _);
        }
    }
    /// Signals `dev.bench.ready.dev.waterui.bench.<W>` — posted once the
    /// bench workload has resolved. The runner waits for this post instead
    /// of querying the accessibility tree (materializing the 10k-row feed's
    /// tree blocks an AX query for minutes).
    pub fn post_ready(workload: &str) {
        let name = format!("dev.bench.ready.dev.waterui.bench.{workload}\0");
        unsafe {
            notify_post(name.as_ptr() as _);
        }
    }
}

/// Appends `step <k> n=<param> t=<unix-seconds>` to tmp/bench-steps.log —
/// the runner pulls this file and slices the xctrace frame recording by
/// these timestamps, so every contestant reports step boundaries the same
/// way on simulator, device and macOS.
#[cfg(any(target_os = "ios", target_os = "macos"))]
fn step_log(step: usize, param: u64) {
    use std::io::Write;
    let p = std::env::temp_dir().join("bench-steps.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
    {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let _ = writeln!(f, "step {step} n={param} t={secs:.3}");
    }
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    bench_notify::post_step();
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
fn drive_log(line: &str) {
    use std::io::Write;
    let p = std::env::temp_dir().join("bench-drive.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
    {
        let _ = writeln!(f, "{:?} {}", std::time::Instant::now(), line);
    }
}

/// Waits for the runner's `dev.bench.begin` on a long-lived check token.
/// Each `notify_post` re-fires a consumed token, so one registration serves
/// every measured iteration of a run (XCTest may invoke the measure block
/// more than once).
#[cfg(any(target_os = "ios", target_os = "macos"))]
async fn await_begin_cycle(token: i32) {
    while !bench_notify::begin_posted(token) {
        sleep(Duration::from_millis(50)).await;
    }
    drive_log("begin seen — running program");
    // Stop the runner's reposts so no begin backlog piles up on the token.
    bench_notify::post_ack();
}

/// Registers `dev.bench.begin` once per launch; `None` leaves the task dead.
#[cfg(any(target_os = "ios", target_os = "macos"))]
fn bench_begin_token() -> Option<i32> {
    let token = bench_notify::register_begin();
    drive_log(if token.is_some() {
        "registered dev.bench.begin"
    } else {
        "notify_register_check failed"
    });
    token
}

/// Posts `dev.bench.done` after the scroll program finishes (auto only).
#[cfg(any(target_os = "ios", target_os = "macos"))]
fn post_done() {
    drive_log("posting dev.bench.done");
    bench_notify::post_done();
}

/// Drops a begin backlog post latched while the program ran (auto only).
#[cfg(any(target_os = "ios", target_os = "macos"))]
fn discard_stale(token: i32) {
    bench_notify::discard_latched_begin(token);
}

// ---------------------------------------------------------------------------
// W1 Hello
// ---------------------------------------------------------------------------

fn hello() -> impl View {
    let counter = Binding::i32(0);
    vstack((
        text!("Count: {counter}").title(),
        button("Increment")
            .action(|State(count): State<Binding<i32>>| *count.get_mut() += 1)
            .state(&counter)
            .a11y_id("increment-button"),
    ))
    .spacing(16.0)
}

// ---------------------------------------------------------------------------
// W2 Feed
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Identifiable)]
struct FeedRow {
    #[id]
    id: u64,
}

fn avatar_color(index: u64) -> Color {
    match index % 6 {
        0 => Color::srgb_hex("#3B82F6"),
        1 => Color::srgb_hex("#10B981"),
        2 => Color::srgb_hex("#F59E0B"),
        3 => Color::srgb_hex("#EF4444"),
        4 => Color::srgb_hex("#8B5CF6"),
        _ => Color::srgb_hex("#EC4899"),
    }
}

fn timestamp(index: u64) -> Str {
    // Deterministic pseudo-timestamp so every contestant shows the same text.
    Str::from(format!("{:02}:{:02}", (index / 60) % 24, index % 60))
}

fn feed_row(row: FeedRow) -> ListItem {
    let n = row.id;
    ListItem::new(
        hstack((
            Circle.fill(avatar_color(n)).size(40.0, 40.0),
            vstack((
                text(text!("Row title {n}")).sub_headline(),
                text(text!("Second line of subtitle for item {n}"))
                    .caption()
                    .muted(),
            ))
            .leading(),
            spacer(),
            text(timestamp(n)).caption().muted(),
        ))
        .spacing(12.0)
        .padding_with((10.0, 16.0)),
    )
}

/// Same traversal as the external fling program, in row-index space:
/// 8 eased jumps of ~1/8 of the list each, then two jumps back up.
/// Cadence mirrors the shared drive: first move at ~1 s, then ~1.15 s steps.
/// In `auto` mode the program waits for the runner's `dev.bench.begin`
/// Darwin notification inside its measure block, and posts
/// `dev.bench.done` at the end.
#[cfg(any(target_os = "ios", target_os = "macos"))]
async fn auto_scroll_rows(scroll: waterui::component::ScrollController<usize>) {
    if !drive_auto() {
        return;
    }
    let Some(token) = bench_begin_token() else {
        return;
    };
    loop {
        await_begin_cycle(token).await;
        sleep(Duration::from_millis(1000)).await;
        let step = FEED_ROWS / 8;
        for i in 1..=8u64 {
            scroll.scroll_to((i * step).min(FEED_ROWS - 1) as usize);
            sleep(Duration::from_millis(1150)).await;
        }
        for row in [FEED_ROWS as usize / 3, 0] {
            scroll.scroll_to(row);
            sleep(Duration::from_millis(1200)).await;
        }
        post_done();
        discard_stale(token);
    }
}

fn feed() -> impl View {
    let records = ReactiveList::from((0..FEED_ROWS).map(|id| FeedRow { id }).collect::<Vec<_>>());
    let scroll = waterui::component::ScrollController::<usize>::new(0);
    let list = List::for_each(records, feed_row).scroll_controller(&scroll);
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    let list = list.task(auto_scroll_rows(scroll));
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    let _ = scroll;
    list
}

// ---------------------------------------------------------------------------
// W3 Motion
// ---------------------------------------------------------------------------

#[derive(Clone, Identifiable)]
struct MotionRect {
    x: Binding<f32>,
    y: Binding<f32>,
    rotation: Binding<f32>,
    opacity: Binding<f32>,
    color: Color,
    duration_ms: u64,
    #[id]
    index: usize,
}

fn rand01(seed: &mut u64) -> f32 {
    // xorshift64 — deterministic per-rect sequence, identical across platforms.
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed % 10_000) as f32 / 10_000.0
}

/// `cancel`/`mine` retire a step's drive tasks when the ladder rebuilds the
/// scene — the generation cell is bumped on every step change, so tasks
/// from an earlier count exit instead of animating dropped bindings.
fn drive_motion_gen(
    rect: MotionRect,
    cancel: Option<std::rc::Rc<std::cell::Cell<u64>>>,
    mine: u64,
) {
    spawn_local(async move {
        let mut seed = 0x9E3779B97F4A7C15u64 ^ (rect.index as u64).wrapping_mul(0xBF58476D1CE4E5B9);
        loop {
            if let Some(c) = &cancel
                && c.get() != mine
            {
                break;
            }
            let x = rand01(&mut seed) * (FIELD_W - RECT_SIZE);
            let y = rand01(&mut seed) * (FIELD_H - RECT_SIZE);
            let rot = rand01(&mut seed) * 360.0;
            let op = 0.3 + rand01(&mut seed) * 0.7;
            rect.x.set(x);
            rect.y.set(y);
            rect.rotation.set(rot);
            rect.opacity.set(op);
            sleep(Duration::from_millis(rect.duration_ms)).await;
        }
    });
}

fn motion() -> impl View {
    motion_scene(MOTION_RECTS, None, 0)
}

fn motion_scene(
    count: usize,
    cancel: Option<std::rc::Rc<std::cell::Cell<u64>>>,
    my_gen: u64,
) -> impl View {
    let anim = Animation::ease_in_out(Duration::from_millis(1400));
    let rects: Vec<MotionRect> = (0..count)
        .map(|i| {
            let mut seed = 0xD1B54A32D192ED03u64 ^ (i as u64).wrapping_mul(0x2545F4914F6CDD1D);
            MotionRect {
                x: Binding::f32(rand01(&mut seed) * (FIELD_W - RECT_SIZE)),
                y: Binding::f32(rand01(&mut seed) * (FIELD_H - RECT_SIZE)),
                rotation: Binding::f32(rand01(&mut seed) * 360.0),
                opacity: Binding::f32(0.3 + rand01(&mut seed) * 0.7),
                color: avatar_color(i as u64),
                // Stagger periods so every rect animates on its own clock.
                duration_ms: 1200 + (i as u64 % 5) * 200,
                index: i,
            }
        })
        .collect();
    for r in &rects {
        drive_motion_gen(r.clone(), cancel.clone(), my_gen);
    }
    let children: Vec<_> = rects
        .into_iter()
        .map(|r| {
            RoundedRectangle::new(0.25)
                .fill(r.color)
                .size(RECT_SIZE, RECT_SIZE)
                .rotation(
                    r.rotation
                        .with(Animation::ease_in_out(Duration::from_millis(r.duration_ms))),
                )
                .opacity(
                    r.opacity
                        .with(Animation::ease_in_out(Duration::from_millis(r.duration_ms))),
                )
                .position_in_offset(
                    UnitPoint::TOP_LEADING,
                    UnitPoint::TOP_LEADING,
                    r.x.with(anim.clone()),
                    r.y.with(anim.clone()),
                )
        })
        .collect();
    absolute(children).size(FIELD_W, FIELD_H)
}

// ---------------------------------------------------------------------------
// W4 Text
// ---------------------------------------------------------------------------

/// Canonical W4 text — benchmarks/competitive/lib/paragraphs.txt. Every
/// contestant embeds the same ten lines; each language keeps its own copy
/// because the app cannot read the suite's file at runtime.
const PARAGRAPHS: [&str; 10] = [
    "The quick brown fox jumps over the lazy dog. 。🦊🐶 Packing my box with five dozen liquor jugs.",
    "WaterUI renders native widgets from a single Rust view tree. 。🌊 Fine-grained reactivity updates only the widgets that read the value.",
    "Almost all programming can be viewed as state management. ，。📚 Signals flow through the graph and wake the views that observe them.",
    "Sphinx of black quartz, judge my vow. のテキストもぜます。🗻 Typography is the visual component of the written word.",
    "How vexingly quick daft zebras jump! ，。🦓 The first principle is that you must not fool yourself.",
    "Bright vixens jump; dozy fowl quack. ，。🐦 Rendering pipelines measure progress in milliseconds per frame.",
    "。Benchmarks that are honest make optimisation honest. 📏",
    "Two driven jocks help fax my big quiz. ，。🌲 Lazily built lists keep memory flat while content grows without bound.",
    "The five boxing wizards jump quickly. ，。🧙 Every frame has a budget of 8.33 milliseconds at 120 Hz.",
    "Jackdaws love my big sphinx of quartz. ，。🐦‍⬛ Measure, then optimise; never optimise on faith alone.",
];

/// W4 auto drive: identical cadence in scroll-offset space (the backend
/// clamps each target to the content size), gated on the same
/// `dev.bench.begin`/`dev.bench.done` handshake as W2.
#[cfg(any(target_os = "ios", target_os = "macos"))]
async fn auto_scroll_pt(scroll: waterui::component::ScrollController<Point>) {
    if !drive_auto() {
        return;
    }
    let Some(token) = bench_begin_token() else {
        return;
    };
    loop {
        await_begin_cycle(token).await;
        sleep(Duration::from_millis(1000)).await;
        for i in 1..=8u32 {
            scroll.scroll_to(Point::new(0.0, (i * 600) as f32));
            sleep(Duration::from_millis(1150)).await;
        }
        for y in [1600.0f32, 0.0] {
            scroll.scroll_to(Point::new(0.0, y));
            sleep(Duration::from_millis(1200)).await;
        }
        post_done();
        discard_stale(token);
    }
}

fn text_bench() -> impl View {
    let controller = waterui::component::ScrollController::new(Point::zero());
    let view = scroll(
        vstack(
            (0..PARAGRAPH_COUNT)
                .map(|i| {
                    text(PARAGRAPHS[i % PARAGRAPHS.len()])
                        .body()
                        .padding_with((6.0, 16.0))
                })
                .collect::<Vec<_>>(),
        )
        .leading()
        .spacing(0.0),
    )
    .scroll_controller(&controller);
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    let view = view.task(auto_scroll_pt(controller));
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    let _ = controller;
    view
}

// ---------------------------------------------------------------------------
// W5/W6 Capacity ladders
// ---------------------------------------------------------------------------

/// W5 motion steps: W3's scene with the rect count doubled to collapse.
/// W6 feed steps: W2's fling program over rows whose nested text+shape
/// child count doubles. Two pacing modes exist: `BENCH_STEP` pins one step
/// per launch (Android's per-step protocol); `drive=auto` walks the whole
/// ladder inside the measure window — after `dev.bench.begin`, each step
/// logs `step k n=<param> t=<unix>` to tmp/bench-steps.log and posts
/// `dev.bench.step`, settles 1 s, holds 4 s; `dev.bench.done` ends the
/// program. The runner's trace is sliced by the logged step times —
/// identical on every contestant.
const CAPACITY_STEPS_W5: [u32; 8] = [200, 400, 800, 1600, 3200, 6400, 12800, 25600];
const CAPACITY_STEPS_W6: [u32; 7] = [1, 2, 4, 8, 16, 32, 64];
/// Seconds a step settles before its hold begins (step start -> settle end).
#[cfg(any(target_os = "ios", target_os = "macos"))]
const STEP_SETTLE_S: u64 = 1;
/// Seconds a step is held for the frame/CPU slice (settle end -> next step).
#[cfg(any(target_os = "ios", target_os = "macos"))]
const STEP_HOLD_S: u64 = 4;

/// The auto-ladder loop for W5 — only compiled where the Darwin handshake
/// exists; the BENCH_STEP mode needs no async driver at all.
#[cfg(any(target_os = "ios", target_os = "macos"))]
fn motion_capacity() -> impl View {
    let count = Binding::u32(CAPACITY_STEPS_W5[0]);
    let scene_gen = std::rc::Rc::new(std::cell::Cell::new(0u64));
    let gen2 = scene_gen.clone();
    let count2 = count.clone();
    let ladder = async move {
        if !drive_auto() {
            return;
        }
        let Some(token) = bench_begin_token() else {
            return;
        };
        loop {
            await_begin_cycle(token).await;
            for (i, n) in CAPACITY_STEPS_W5.iter().enumerate() {
                gen2.set(gen2.get() + 1);
                count2.set(*n);
                step_log(i, *n as u64);
                sleep(Duration::from_secs(STEP_SETTLE_S + STEP_HOLD_S)).await;
            }
            post_done();
            discard_stale(token);
        }
    };
    Dynamic::watch(count, move |n| {
        let g = scene_gen.get();
        motion_scene(n as usize, Some(scene_gen.clone()), g)
    })
    .task(ladder)
}

/// Non-Apple W5: `BENCH_STEP` pins the rect count for this launch; without
/// it the first step renders (the workload exists so a swipe-driven step
/// run still shows a scene rather than trapping).
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
fn motion_capacity() -> impl View {
    let n = bench_step().unwrap_or(CAPACITY_STEPS_W5[0]);
    motion_scene(n as usize, None, 0)
}

/// W2 row with `complexity` extra nested text+shape children — the per-row
/// load the W6 ladder multiplies.
fn feed_row_complex(row: FeedRow, complexity: u32) -> ListItem {
    let n = row.id;
    let mut extra: Vec<AnyView> = (0..complexity)
        .map(|j| {
            AnyView::new(vstack((
                RoundedRectangle::new(0.3)
                    .fill(avatar_color(n + j as u64))
                    .size(14.0, 14.0),
                text(Str::from(format!("c{j}"))).caption(),
            )))
        })
        .collect();
    let mut children: Vec<AnyView> = vec![
        AnyView::new(Circle.fill(avatar_color(n)).size(40.0, 40.0)),
        AnyView::new(
            vstack((
                text(text!("Row title {n}")).sub_headline(),
                text(text!("Second line of subtitle for item {n}"))
                    .caption()
                    .muted(),
            ))
            .leading(),
        ),
        AnyView::new(spacer()),
    ];
    children.append(&mut extra);
    children.push(AnyView::new(text(timestamp(n)).caption().muted()));
    ListItem::new(hstack(children).spacing(12.0).padding_with((10.0, 16.0)))
}

fn feed_scene(complexity: u32, scroll: waterui::component::ScrollController<usize>) -> impl View {
    let records = ReactiveList::from((0..FEED_ROWS).map(|id| FeedRow { id }).collect::<Vec<_>>());
    List::for_each(records, move |r| feed_row_complex(r, complexity)).scroll_controller(&scroll)
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
fn feed_capacity() -> impl View {
    let complexity = Binding::u32(CAPACITY_STEPS_W6[0]);
    let scroll = waterui::component::ScrollController::<usize>::new(0);
    let complexity2 = complexity.clone();
    let scroll2 = scroll.clone();
    let ladder = async move {
        if !drive_auto() {
            return;
        }
        let Some(token) = bench_begin_token() else {
            return;
        };
        loop {
            await_begin_cycle(token).await;
            for (i, k) in CAPACITY_STEPS_W6.iter().enumerate() {
                complexity2.set(*k);
                step_log(i, *k as u64);
                sleep(Duration::from_secs(STEP_SETTLE_S)).await;
                // Fling the measure window: down to the end and back,
                // the W2 program's cadence (~2 s per sweep).
                for _ in 0..2 {
                    scroll2.scroll_to((FEED_ROWS - 1) as usize);
                    sleep(Duration::from_millis(1000)).await;
                    scroll2.scroll_to(0);
                    sleep(Duration::from_millis(1000)).await;
                }
            }
            post_done();
            discard_stale(token);
        }
    };
    Dynamic::watch(complexity, move |k| feed_scene(k, scroll.clone())).task(ladder)
}

/// Non-Apple W6: `BENCH_STEP` pins the per-row nesting for this launch.
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
fn feed_capacity() -> impl View {
    let k = bench_step().unwrap_or(CAPACITY_STEPS_W6[0]);
    let scroll = waterui::component::ScrollController::<usize>::new(0);
    feed_scene(k, scroll)
}

// ---------------------------------------------------------------------------

#[preview]
fn main() -> impl View {
    let w = workload();
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    bench_notify::post_ready(w);
    let content: AnyView = match w {
        "W2" => AnyView::new(feed()),
        "W3" => AnyView::new(motion()),
        "W4" => AnyView::new(text_bench()),
        "W5" => AnyView::new(motion_capacity()),
        "W6" => AnyView::new(feed_capacity()),
        _ => AnyView::new(hello()),
    };
    // The selected workload id rides on a 1x1 text element as its
    // accessibility identifier — the runner asserts `bench-workload-<id>`
    // after launch. A text is used because a container view with only the
    // identifier set does not enter the macOS accessibility hierarchy; the
    // text overlays the content in a zstack so a greedy List cannot push
    // it out of the rendered frame. The text comes FIRST so the
    // accessibility resolver reaches it before descending into the
    // content's subtree (the 10k-row list exposes every row to AX).
    zstack((
        text(format!("bench-workload-{w}"))
            .a11y_id(Str::from(format!("bench-workload-{w}")))
            .size(1.0, 1.0),
        content,
    ))
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
