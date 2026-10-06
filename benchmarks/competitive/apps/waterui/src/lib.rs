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
//! `-bench-workload w1|w2|w3|w4|w5|w6` in argv (Apple legs, the
//! NSArgumentDomain convention) or `BENCH_WORKLOAD` in the environment
//! (desktop legs and Android's `waterui.env.*` intent-extra forwarding);
//! ids are the exact lowercase strings `w1`–`w6` — anything else traps;
//! there is no fallback. Scrolling is driven from outside the app by
//! OS-level input on every platform — the app never scrolls itself.
//! On iOS the selected id also posts `dev.bench.ready.waterui.<w>`
//! (the contestant id, then the workload) when the view first appears.
//!
//! W5/W6 pacing is one model on every leg: one launch renders one
//! ladder step, pinned by `BENCH_STEP` (or `-bench-step N`) — a missing
//! or malformed step traps.

use std::time::Duration;
use waterui::Identifiable;
use waterui::animation::Animation;
use waterui::app::App;
use waterui::component::list::List;
use waterui::id::SelfId;
use waterui::layout::{AbsoluteLayout, LazyContainer};
use waterui::prelude::*;
use waterui::reactive::collection::List as ReactiveList;
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
    match bench_arg("workload").as_deref() {
        Some("w1") => "w1",
        Some("w2") => "w2",
        Some("w3") => "w3",
        Some("w4") => "w4",
        Some("w5") => "w5",
        Some("w6") => "w6",
        Some(other) => {
            panic!("unrecognized bench workload value {other:?}; expected w1..=w6")
        }
        None => panic!("missing bench workload (expected -bench-workload or BENCH_WORKLOAD)"),
    }
}

/// The capacity protocol on every leg: `BENCH_STEP` (or `-bench-step N`)
/// must be present, parse as an integer and name a member of the
/// workload's ladder — a missing or malformed step traps rather than
/// defaulting.
fn capacity_step(ladder: &[u32], workload: &str) -> u32 {
    match bench_arg("step").as_deref() {
        None => panic!("{workload}: missing bench step (expected -bench-step or BENCH_STEP)"),
        Some(v) => match v.parse::<u32>() {
            Ok(n) if ladder.contains(&n) => n,
            _ => panic!("{workload}: malformed bench step {v:?}; expected one of {ladder:?}"),
        },
    }
}

/// Darwin-notification readiness signal: the iOS runner waits for
/// `dev.bench.ready.<contestant>.<w>` instead of querying the
/// accessibility tree (materializing the 10k-row feed's tree blocks an
/// AX query for minutes). Every iOS contestant is installed under one
/// shared bundle id, so the post names the contestant itself: the id
/// below is this app's `contestants[].id` in the Apple manifest, and the
/// runner waits for the id of the stage entry it installed.
#[cfg(target_os = "ios")]
mod bench_notify {
    use std::ffi::c_char;

    /// This app's contestant id in `benchmarks/competitive/apple/manifest.json`.
    const CONTESTANT: &str = "waterui";

    #[link(name = "System")]
    unsafe extern "C" {
        fn notify_post(name: *const c_char) -> u32;
    }

    /// Signals `dev.bench.ready.<contestant>.<w>` — posted once the bench
    /// workload's view first appears.
    pub fn post_ready(workload: &str) {
        let name = format!("dev.bench.ready.{CONTESTANT}.{workload}\0");
        // SAFETY: `name` is NUL-terminated and outlives the call.
        let status = unsafe { notify_post(name.as_ptr().cast()) };
        assert_eq!(status, 0, "notify_post({name:?}) failed with {status}");
    }
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
            .leading()
            .spacing(4.0),
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
/// doubles. One pacing model on every leg: one launch renders one step,
/// pinned by BENCH_STEP — the runner measures each step as its own
/// launch and slices its frame record per METHOD.
const CAPACITY_STEPS_W5: [u32; 8] = [200, 400, 800, 1600, 3200, 6400, 12800, 25600];
const CAPACITY_STEPS_W6: [u32; 7] = [1, 2, 4, 8, 16, 32, 64];

/// W5 — W3's scene at the step's rect count; `ForEach` keys each rect by
/// identity so a rect's view (and its per-rect animation) lives as long
/// as the rect exists — no `Dynamic::watch`, no generation counters.
fn motion_capacity() -> impl View {
    let count = capacity_step(&CAPACITY_STEPS_W5, "w5");
    motion_scene(ForEach::new(
        (0..count as usize).map(SelfId::new).collect::<Vec<_>>(),
        |id: SelfId<usize>| motion_rect(*id),
    ))
}

/// W2 row with `complexity` extra nested text+shape cells. One launch
/// pins one step, so the cell count is fixed for the launch and every
/// cell is laid out eagerly (a `Vec` of views — no lazy cell container).
fn feed_row_complex(row: FeedRow, complexity: u32) -> ListItem {
    let n = row.id;
    let cells: Vec<_> = (0..complexity)
        .map(|j| {
            vstack((
                RoundedRectangle::new(0.3)
                    .fill(avatar_color(n.wrapping_add(j as u64)))
                    .size(14.0, 14.0),
                text(Str::from(format!("c{j}"))).size(12.0),
            ))
        })
        .collect();
    ListItem::new(
        hstack((
            Circle.fill(avatar_color(n)).size(40.0, 40.0),
            vstack((
                text!("Row title {n}").size(16.0),
                text!("Second line of subtitle for item {n}")
                    .size(13.0)
                    .muted(),
            ))
            .leading()
            .spacing(4.0),
            spacer(),
            hstack(cells).spacing(4.0),
            text(timestamp(n)).size(13.0).muted(),
        ))
        .spacing(12.0)
        .padding_with((10.0, 16.0)),
    )
}

fn feed_scene(complexity: u32) -> impl View {
    let records = ReactiveList::from((0..FEED_ROWS).map(|id| FeedRow { id }).collect::<Vec<_>>());
    List::for_each(records, move |r| feed_row_complex(r, complexity))
}

fn feed_capacity() -> impl View {
    feed_scene(capacity_step(&CAPACITY_STEPS_W6, "w6"))
}

// ---------------------------------------------------------------------------

#[preview]
fn main() -> impl View {
    let w = workload();
    let content: AnyView = match w {
        "w1" => AnyView::new(hello()),
        "w2" => AnyView::new(feed()),
        "w3" => AnyView::new(motion()),
        "w4" => AnyView::new(text_bench()),
        "w5" => AnyView::new(motion_capacity()),
        "w6" => AnyView::new(feed_capacity()),
        _ => unreachable!("workload() only yields w1..=w6"),
    };
    // `ready` fires when the workload view first appears — the earliest
    // rendered frame — never at view construction.
    #[cfg(target_os = "ios")]
    let content = content.on_appear(move || bench_notify::post_ready(w));
    content
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
