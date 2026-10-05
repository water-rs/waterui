//! Competitive benchmark app (water-rs/waterui#1262, harness #1864) — the
//! single WaterUI contestant every platform leg builds. Geometry, palette,
//! motion model, text corpus and fling protocol are the canonical spec in
//! benchmarks/competitive/WORKLOADS.md; this app implements it exactly:
//!
//! - W1 Hello:  one centred label and one button that increments a counter.
//! - W2 Feed: lazy list of 10,000 rows (avatar circle, two text lines,
//!   trailing timestamp).
//! - W3 Motion: 200 independently animated rounded rectangles (position,
//!   rotation, opacity) running continuously.
//! - W4 Text: scrolling screen of 50 paragraphs of mixed Latin/CJK/emoji,
//!   all laid out eagerly.
//! - W5 Motion capacity: the W3 scene stepped geometrically 200→25600 rects.
//! - W6 Feed capacity: the W2 feed with per-row nested text+shape cells
//!   stepped 1→64.
//!
//! Workload selection accepts either channel a leg can deliver:
//! `-bench-workload W1|W2|W3|W4|W5|W6` in argv (Apple legs, the
//! NSArgumentDomain convention) or `BENCH_WORKLOAD` in the environment
//! (desktop legs and Android's `waterui.env.*` intent-extra forwarding);
//! values match case-insensitively. A missing or unrecognized workload
//! traps; there is no fallback. Scrolling is driven from outside the app
//! by OS-level input on every platform — the app never scrolls itself.
//! On Apple targets the selected id also posts
//! `dev.bench.ready.<bundle>.<W>` and rides the accessibility identifier
//! `bench-workload-<id>` so the runner can assert it.
//!
//! W5/W6 have two pacing modes: `BENCH_STEP` in the environment pins one
//! ladder step per launch (the Android/desktop protocol, which measures
//! each step as a separate launch — a missing or malformed step traps);
//! on Apple targets the runner walks the whole ladder inside the measure
//! window: `dev.bench.begin` starts it, `dev.bench.step` posts and a
//! bench-steps.log mark step boundaries, `dev.bench.done` ends it.

use std::time::Duration;
use waterui::Identifiable;
use waterui::animation::Animation;
use waterui::app::App;
use waterui::component::lazy::Lazy;
use waterui::component::list::List;
use waterui::id::SelfId;
use waterui::layout::{AbsoluteLayout, LazyContainer};
use waterui::prelude::*;
use waterui::reactive::collection::{List as ReactiveList, SignalCollection};
use waterui::shape::{Circle, RoundedRectangle, ShapeExt};
use waterui::task::sleep;
use waterui::views::ForEach;

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

/// The capacity protocol on platforms that measure one ladder step per
/// launch: `BENCH_STEP` (or `-bench-step N`) must be present, parse as an
/// integer and name a member of the workload's ladder — a missing or
/// malformed step traps rather than defaulting.
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
fn capacity_step(ladder: &[u32], workload: &str) -> u32 {
    match bench_arg("step").as_deref() {
        None => panic!("{workload}: missing bench step (expected -bench-step or BENCH_STEP)"),
        Some(v) => match v.parse::<u32>() {
            Ok(n) if ladder.contains(&n) => n,
            _ => panic!("{workload}: malformed bench step {v:?}; expected one of {ladder:?}"),
        },
    }
}

/// Darwin-notification handshake for capacity-ladder pacing (W5/W6). The
/// runner posts `dev.bench.begin` inside its `measure` block and waits
/// for the app to post `dev.bench.done` after the ladder finishes.
/// Scrolling itself is always driven from outside the app.
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

/// Posts `dev.bench.done` after the capacity ladder finishes.
#[cfg(any(target_os = "ios", target_os = "macos"))]
fn post_done() {
    drive_log("posting dev.bench.done");
    bench_notify::post_done();
}

/// Drops a begin backlog post latched while the ladder ran.
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
        text!("Count: {counter}").size(20.0),
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
                text!("Row title {n}").size(16.0),
                text!("Second line of subtitle for item {n}")
                    .size(13.0)
                    .muted(),
            ))
            .leading(),
            spacer(),
            text(timestamp(n)).size(13.0).muted(),
        ))
        .spacing(12.0)
        .padding_with((10.0, 16.0)),
    )
}

fn feed() -> impl View {
    let records = ReactiveList::from((0..FEED_ROWS).map(|id| FeedRow { id }).collect::<Vec<_>>());
    List::for_each(records, feed_row)
}

// ---------------------------------------------------------------------------
// W3 Motion — WORKLOADS.md motion model
// ---------------------------------------------------------------------------

/// Per-rect value in [0, 1) from xorshift64: s ^= s<<13; s ^= s>>7;
/// s ^= s<<17 — all arithmetic wraps at 2^64, identical on every platform.
fn rand01(seed: &mut u64) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed % 10_000) as f32 / 10_000.0
}

/// One rect of the motion scene: its own pose bindings plus a drive task
/// that retargets the pose starting at t=0 and again each time the
/// rect's own animation period elapses, forever. The task lives exactly
/// as long as the view — when the W5 ladder drops a rect its drive dies
/// with it, no generation tracking needed.
fn motion_rect(index: usize) -> impl View {
    let mut init = 0xD1B54A32D192ED03u64 ^ (index as u64).wrapping_mul(0x2545F4914F6CDD1D);
    let x = Binding::f32(rand01(&mut init) * (FIELD_W - RECT_SIZE));
    let y = Binding::f32(rand01(&mut init) * (FIELD_H - RECT_SIZE));
    let rotation = Binding::f32(rand01(&mut init) * 360.0);
    let opacity = Binding::f32(0.3 + rand01(&mut init) * 0.7);
    let color = avatar_color(index as u64);
    // Stagger periods so every rect animates on its own clock.
    let duration = Duration::from_millis(1200 + (index as u64 % 5) * 200);
    let (x2, y2, r2, o2) = (x.clone(), y.clone(), rotation.clone(), opacity.clone());
    let drive = async move {
        let mut seed = 0x9E3779B97F4A7C15u64 ^ (index as u64).wrapping_mul(0xBF58476D1CE4E5B9);
        loop {
            x2.set(rand01(&mut seed) * (FIELD_W - RECT_SIZE));
            y2.set(rand01(&mut seed) * (FIELD_H - RECT_SIZE));
            r2.set(rand01(&mut seed) * 360.0);
            o2.set(0.3 + rand01(&mut seed) * 0.7);
            sleep(duration).await;
        }
    };
    RoundedRectangle::new(0.25)
        .fill(color)
        .size(RECT_SIZE, RECT_SIZE)
        .rotation(rotation.with(Animation::ease_in_out(duration)))
        .opacity(opacity.with(Animation::ease_in_out(duration)))
        .position_in_offset(
            UnitPoint::TOP_LEADING,
            UnitPoint::TOP_LEADING,
            x.with(Animation::ease_in_out(duration)),
            y.with(Animation::ease_in_out(duration)),
        )
        .task(drive)
}

/// The motion scene over `ids` — a `Views` collection so the W5 ladder can
/// hand it a reactive rect count: `ForEach` diffs by identity, so an
/// unchanged rect's view (and its drive task) survives a step change.
fn motion_scene<V: View>(ids: impl waterui::views::Views<View = V> + 'static) -> impl View {
    LazyContainer::new(AbsoluteLayout, ids).size(FIELD_W, FIELD_H)
}

fn motion() -> impl View {
    motion_scene(ForEach::new(
        (0..MOTION_RECTS).map(SelfId::new).collect::<Vec<_>>(),
        |id: SelfId<usize>| motion_rect(*id),
    ))
}

// ---------------------------------------------------------------------------
// W4 Text
// ---------------------------------------------------------------------------

/// Canonical W4 text — benchmarks/competitive/lib/paragraphs.txt. Every
/// contestant embeds the same ten lines; each language keeps its own copy
/// because the app cannot read the suite's file at runtime.
const PARAGRAPHS: [&str; 10] = [
    "The quick brown fox jumps over the lazy dog. 敏捷的棕色狐狸跳過懶惰的狗。🦊🐶 Packing my box with five dozen liquor jugs.",
    "WaterUI renders native widgets from a single Rust view tree. 水のインターフェースはネイティブウィジェットを描画する。🌊",
    "Almost all programming can be viewed as state management. 几乎所有的编程都可以视为状态管理。📚 Signals flow through the graph.",
    "Sphinx of black quartz, judge my vow. 黒い水晶のスフィンクス、私の誓いを裁け。🗻 Typography is the visual component of the written word.",
    "How vexingly quick daft zebras jump! 빠른 얼룩말이 얼마나 성가시게 뛰는가! 🦓 The first principle is that you must not fool yourself.",
    "Bright vixens jump; dozy fowl quack. 밝은 여우가 뛰고 졸린 새가 꽥꽥 운다. 🐦 Rendering pipelines measure progress in milliseconds per frame.",
    "ベンチマークが正直であれば最適化も正直になる。Benchmarks that are honest make optimisation honest. 📏",
    "Two driven jocks help fax my big quiz. 두 명의 조키가 내 큰 퀴즈를 팩스로 보내는 것을 돕는다. 🌲 Lazily built lists keep memory flat.",
    "The five boxing wizards jump quickly. 五個拳擊巫師跳得很快。🧙 Every frame has a budget of 8.33 milliseconds at 120 Hz.",
    "Jackdaws love my big sphinx of quartz. 寒鸦喜欢我巨大的石英斯芬克斯。🐦‍⬛ Measure, then optimise; never optimise on faith alone.",
];

/// W4: all fifty paragraphs laid out eagerly inside one scroll view —
/// layout cost is part of the measurement, so nothing may be lazy.
fn text_bench() -> impl View {
    scroll(
        vstack(
            (0..PARAGRAPH_COUNT)
                .map(|i| {
                    text(PARAGRAPHS[i % PARAGRAPHS.len()])
                        .size(16.0)
                        .padding_with((10.0, 16.0))
                })
                .collect::<Vec<_>>(),
        )
        .leading()
        .spacing(6.0),
    )
}

// ---------------------------------------------------------------------------
// W5/W6 Capacity ladders
// ---------------------------------------------------------------------------

/// W5 motion steps: W3's scene with the rect count doubled to collapse.
/// W6 feed steps: W2's feed rows whose nested text+shape cell count
/// doubles. Two pacing modes exist: `BENCH_STEP` pins one step per launch
/// (Android/desktop — missing or malformed traps); on Apple targets the
/// ladder walks itself inside the measure window — after
/// `dev.bench.begin` each step logs `step k n=<param> t=<unix>` to
/// tmp/bench-steps.log and posts `dev.bench.step`, settles 1 s, holds 4 s;
/// `dev.bench.done` ends it. The runner's trace is sliced by the logged
/// step times — identical on every contestant.
const CAPACITY_STEPS_W5: [u32; 8] = [200, 400, 800, 1600, 3200, 6400, 12800, 25600];
const CAPACITY_STEPS_W6: [u32; 7] = [1, 2, 4, 8, 16, 32, 64];
/// Seconds a step settles before its hold begins (step start -> settle end).
#[cfg(any(target_os = "ios", target_os = "macos"))]
const STEP_SETTLE_S: u64 = 1;
/// Seconds a step is held for the frame/CPU slice (settle end -> next step).
#[cfg(any(target_os = "ios", target_os = "macos"))]
const STEP_HOLD_S: u64 = 4;

/// One `dev.bench.begin` cycle of a capacity ladder: drives `param` through
/// every step with step markers and a hold each, then posts `done`.
/// Runs only where the Darwin handshake exists.
#[cfg(any(target_os = "ios", target_os = "macos"))]
async fn capacity_ladder(param: Binding<u32>, ladder: &[u32]) {
    let Some(token) = bench_begin_token() else {
        return;
    };
    loop {
        await_begin_cycle(token).await;
        for (i, n) in ladder.iter().enumerate() {
            param.set(*n);
            step_log(i, *n as u64);
            sleep(Duration::from_secs(STEP_SETTLE_S + STEP_HOLD_S)).await;
        }
        post_done();
        discard_stale(token);
    }
}

/// W5 — the rect count is a signal; `ForEach` diffs by identity so a rect's
/// view (and its per-rect animation) lives as long as the rect exists. The
/// collection is `SignalCollection` over the count — no `Dynamic::watch`,
/// no generation counters: a rect the ladder drops loses its view and its
/// drive task dies with the view.
fn motion_capacity() -> impl View {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    let count = Binding::u32(CAPACITY_STEPS_W5[0]);
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    let count = Binding::u32(capacity_step(&CAPACITY_STEPS_W5, "W5"));
    let rects = SignalCollection::new(
        count
            .map(|n| (0..n as usize).map(SelfId::new).collect::<Vec<_>>())
            .computed(),
    );
    let scene = motion_scene(ForEach::new(rects, |id: SelfId<usize>| motion_rect(*id)));
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    let scene = scene.task(capacity_ladder(count, &CAPACITY_STEPS_W5));
    scene
}

/// W2 row with `complexity` extra nested text+shape cells — a signal of
/// cells per row so the ladder adds/drops cells in place.
fn feed_row_complex(row: FeedRow, complexity: Binding<u32>) -> ListItem {
    let n = row.id;
    let cells = SignalCollection::new(
        complexity
            .map(|k| (0..k).map(SelfId::new).collect::<Vec<_>>())
            .computed(),
    );
    let extra = ForEach::new(cells, move |j: SelfId<u32>| {
        let j = *j;
        vstack((
            RoundedRectangle::new(0.3)
                .fill(avatar_color(n.wrapping_add(j as u64)))
                .size(14.0, 14.0),
            text(Str::from(format!("c{j}"))).size(13.0),
        ))
    });
    ListItem::new(
        hstack((
            Circle.fill(avatar_color(n)).size(40.0, 40.0),
            vstack((
                text!("Row title {n}").size(16.0),
                text!("Second line of subtitle for item {n}")
                    .size(13.0)
                    .muted(),
            ))
            .leading(),
            spacer(),
            Lazy::hstack(extra),
            text(timestamp(n)).size(13.0).muted(),
        ))
        .spacing(12.0)
        .padding_with((10.0, 16.0)),
    )
}

fn feed_scene(complexity: Binding<u32>) -> impl View {
    let records = ReactiveList::from((0..FEED_ROWS).map(|id| FeedRow { id }).collect::<Vec<_>>());
    List::for_each(records, move |r| feed_row_complex(r, complexity.clone()))
}

fn feed_capacity() -> impl View {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    let complexity = Binding::u32(CAPACITY_STEPS_W6[0]);
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    let complexity = Binding::u32(capacity_step(&CAPACITY_STEPS_W6, "W6"));
    let list = feed_scene(complexity.clone());
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    let list = list.task(capacity_ladder(complexity, &CAPACITY_STEPS_W6));
    list
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
