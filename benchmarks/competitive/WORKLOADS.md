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

`W1|W2|W3|W4|W5|W6`, matched case-insensitively. Apple legs pass
`-bench-workload <id>` as a launch argument; desktop legs and Android
pass `BENCH_WORKLOAD` in the environment (Android forwards it as the
`waterui.env.BENCH_WORKLOAD` intent extra). Capacity steps arrive as
`-bench-step <n>` / `BENCH_STEP`.

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
- Trailing timestamp `{hh}:{mm}` zero-padded, `hh = (i / 60) % 24`,
  `mm = i % 60`, font size 13, muted.
- Row padding: horizontal 16, vertical 10.

The list is lazily materialized by the framework (that is the point of
the workload); the contestant declares all 10,000 rows up front.

## W3 Motion

200 rects wander continuously inside a fixed 720×440 field.

- Rect: 40×40 rounded rectangle, corner radius 10 (as a ratio: 0.25 of
  size), fill `palette[i % 6]`.
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
  `1200 + (i % 5) * 200` ms, ease-in-out, one duration for all channels.
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

Pacing: Android/desktop legs measure one step per launch (`BENCH_STEP`,
required — missing/malformed fails the contestant). The Apple leg walks
the whole ladder inside the measure window, marking each step's start
(`dev.bench.step` Darwin post plus `step k n=<n> t=<unix>` in
`tmp/bench-steps.log`), settling 1 s and holding 4 s per step, ending with
`dev.bench.done`. A step collapses when fewer than half its presents land
inside two 60 Hz frame budgets (33.3 ms).

## W6 Feed capacity

The W2 feed (identical rows, 10,000 rows, palette, templates, lazy list)
whose every row additionally carries `complexity` cells appended after the
text column. Cell `j` on row `i`: a 14×14 rounded square (radius ratio
0.3) of `palette[(i + j) % 6]` over the text `c{j}`, font size 12.

Ladder (`complexity`): `1, 2, 4, 8, 16, 32, 64`. Same pacing modes and
step protocol as W5; the fling program below runs during each step's hold.

## Scroll drive — shared fling protocol

All scrolling in W2, W4 and W6 is driven from **outside the app** by
OS-level input on every platform: Android `input swipe` / UiAutomator,
XCTest coordinate drags on Apple, the compositor's virtual pointer on
Linux, `SendInput` on Windows. No contestant scrolls itself; there is no
in-app drive.

The protocol is identical for every contestant on a platform and is
declared once in that platform's manifest:

- 8 flings down (content moves up), then 2 flings up.
- Each fling: horizontal centre, from 75% to 15% of the scroll surface's
  height, gesture duration 250 ms.
- 350 ms pause between flings.
- Wheel-based injectors express one fling as ~12 detents (15 px each) over
  the 250 ms window; the burst and pause timings are identical.
