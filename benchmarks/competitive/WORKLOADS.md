# Competitive workload specification — water-rs/waterui#1262

This file is the single written specification every contestant in
`benchmarks/competitive/` implements. Contestant source hardcodes these
values; a platform's manifest declares the harness parameters it consumes
(repetitions, the fling program, capacity pacing) with identical values —
the manifest is a declaration, this document is the definition.

All lengths are logical points/dp. All arithmetic in the motion model is
on unsigned 64-bit integers with wraparound (`u64`/`uint64_t`/`Long`
unsigned ops/`BigInt.asUintN(64)` semantics); signed `>>` shifts and
floating-point seeds are spec violations.

## Common

- Palette (6 colors, index `i % 6`): `#3B82F6`, `#10B981`, `#F59E0B`,
  `#EF4444`, `#8B5CF6`, `#EC4899`.
- Window size on desktop platforms: 1280×800 logical points. Mobile
  platforms run fullscreen.
- A missing or malformed workload parameter is a hard failure in every
  contestant — no defaults, no fallbacks.

## Workload selection

Workload ids are the exact lowercase strings `w1|w2|w3|w4|w5|w6`. Any
other value — including uppercase or missing — fails the contestant.
Apple legs pass `-bench-workload <id>` as a launch argument; desktop
legs and Android pass `BENCH_WORKLOAD` in the environment (Android
forwards it as the `waterui.env.BENCH_WORKLOAD` intent extra). Capacity
steps arrive as `-bench-step <n>` / `BENCH_STEP`.

Every contestant implements every workload in this document; a leg
selects the subset it measures, but a contestant missing a workload is a
failed cell, never an excluded one.

## W1 Hello

- One label `Count: {i}` starting at 0, font size 20.
- One button `Increment`, accessibility id `increment-button`, increments
  the counter.
- Vertical spacing 16, centred.

## W2 Feed

Lazy list of 10,000 rows. Row `i` (0-based):

- Avatar: 40×40 circle, `palette[i % 6]`, gap to text 12.
- Title `Row title {i}`, font size 16.
- Subtitle `Second line of subtitle for item {i}`, font size 13, muted.
- Title-to-subtitle vertical spacing 4.
- Trailing timestamp `{hh}:{mm}` zero-padded, `hh = (i / 60) % 24`,
  `mm = i % 60`, font size 13, muted. Title/subtitle column and
  timestamp are separated by a horizontal gap of 12.
- Row padding: horizontal 16, vertical 10.

The list is lazily materialized by the framework (that is the point of
the workload); the contestant declares all 10,000 rows up front.

## W3 Motion

200 rects wander continuously inside a fixed 720×440 field.

- Rect: 40×40 rounded rectangle, corner radius 10 (as a ratio: 0.25 of
  size), fill `palette[i % 6]`.
- Field placement: horizontally centred; on mobile layouts it is pinned
  to the top of the content area with a 16-point inset, on desktop it is
  centred vertically as well.
- Channels animated per rect: position (x, y inside the field minus the
  rect), rotation 0–360°, opacity 0.3–1.0.
- Per-rect RNG: two xorshift64 streams with unsigned 64-bit wraparound.
  Step: `s ^= s << 13; s ^= s >> 7; s ^= s << 17; value = (s % 10000) / 10000`.
  - Init stream seed: `0xD1B54A32D192ED03 ^ (i *% 0x2545F4914F6CDD1D)` —
    produces the rect's initial x, y, rotation, opacity in that order. The
    initial pose is drawn as-is: no discarded draws, no "skip first value".
  - Drive stream seed: `0x9E3779B97F4A7C15 ^ (i *% 0xBF58476D1CE4E5B9)` —
    produces each new target in the same order.
- Schedule: the first retarget happens at t=0 (the rect eases from its
  init pose to the first drive target), then each rect retargets when its
  own animation completes, forever. Per-rect duration
  `1200 + (i % 5) * 200` ms, one duration for all channels.
- Easing: one curve for every retarget on every contestant —
  `cubic-bezier(0.42, 0.0, 0.58, 1.0)` (the standard ease-in-out:
  symmetric ease in, ease out; native equivalents: CAMediaTimingFunction
  ease-in-ease-out, PathInterpolator(0.42, 0, 0.58, 1), CSS
  `ease-in-out`).
  `*%` denotes wrapping multiplication.

## W4 Text

Scroll view containing `lib/paragraphs.txt` — the canonical corpus of ten
paragraphs, each mixing Latin, Han ideographs, kana, Hangul and emoji.
Every contestant embeds its own copy of these exact lines.

- Paragraph count: 50 (the corpus lines repeated `i % 10`), **all laid out
  eagerly** — a lazy container defeats the measurement.
- Font size 16; paragraph spacing 6; paragraph padding horizontal 16,
  vertical 10.

## W5 Motion capacity

The W3 scene (identical field, rect geometry, palette, seeds, schedule)
with `rect_count` stepped geometrically. One launch renders one step.

Ladder: `200, 400, 800, 1600, 3200, 6400, 12800, 25600`.

Pacing — one model on every leg: one launch renders one step. The
runner launches the contestant once per ladder step with `BENCH_STEP`
(`-bench-step` on Apple) naming the step — required, missing or
malformed fails the contestant — then measures per METHOD below. A
step collapses when fewer than half its presents land inside two 60 Hz
frame budgets (33.3 ms).

## W6 Feed capacity

The W2 feed (identical rows, 10,000 rows, palette, templates, lazy list)
whose every row additionally carries `complexity` cells appended after the
text column. Cell `j` on row `i`: a 14×14 rounded square (radius ratio
0.3) of `palette[(i + j) % 6]` over the text `c{j}`, font size 12.

Every row materializes its full cell count — no lazy per-cell
containers that skip cells. Row-level view reuse/recycling is allowed
(and idiomatic) as long as binding a row repopulates all of its cells.

Ladder (`complexity`): `1, 2, 4, 8, 16, 32, 64`. Cells are separated by
4 horizontally; the cell group keeps the row's standard 12 gap to the
text column and to the trailing timestamp.
Same pacing model and step protocol as W5; the fling program below runs
during each step's hold. The scroll drive kind is identical for W2, W4
and W6 on each platform (whatever OS-level injector that leg uses).

## Scroll drive — shared fling protocol

All scrolling in W2, W4 and W6 is driven from **outside the app** by
OS-level input on every platform: Android `input swipe` / UiAutomator,
XCTest coordinate drags on iOS devices and CGEvent scroll-wheel detents
posted by the host driver on macOS and the iOS Simulator (a synthesized
swipe cannot reach a simulator window), the compositor's virtual
pointer on Linux, `SendInput` on Windows. No contestant scrolls itself;
there is no in-app drive.

The protocol is identical for every contestant on a platform and is
declared once in that platform's manifest:

- 8 flings down (content moves up), then 2 flings up.
- Each fling: horizontal centre, from 75% to 15% of the scroll surface's
  height, gesture duration 250 ms.
- 350 ms pause between flings.
- Wheel-based injectors express one fling as ~12 detents (15 px each) over
  the 250 ms window; the burst and pause timings are identical.

## METHOD — one frame-statistics definition for every leg

Every leg reports frame statistics computed by
`lib/frame_stats.py::frame_statistics`, with identical semantics:

- Input: present timestamps attributable to the contestant's owned
  processes, the measurement window, the display refresh period.
- The measurement window is `[first owned present + declared warmup,
  + capture_s]` on every platform. Startup frames sit before the window
  and never enter the data. The warmup is declared in each leg's
  manifest and is never 0 (android `[pacing].warmup_ms`, linux
  `[pacing].warmup_ms`, apple `harness.warmup_ms`, windows
  `[runner].warmup_seconds`).
- The drive program starts at window start.
- `startup_ms` is computed from launch to the first owned present, never
  from windowed data.
- A gap longer than 100 ms with no present ends an active run and is
  excluded — it is neither a frame nor a drop.
- Within active runs, frame time = present interval, and an interval
  longer than 1.5 refresh periods contributes
  `round(interval / period) − 1` missed vsyncs.

A contestant signals only readiness (its first frame). Apps never
signal completion: the host driver owns the end of every cell from the
declared program and duration, under its own notification names.
