# Layout twins

Measured frames for identical view trees in SwiftUI and WaterUI's Apple
backend — the data behind the frozen layout-spec decisions in
water-rs/waterui#1229 (audit: first comment on water-rs/waterui#1227,
sections A2, A3, A11, A12, A13, D1–D5).

Every number below was read off a live render; nothing here is computed or
assumed. Results are committed as JSON under `results/<side>/<os>-s<scale>/<case>.json`.

## Harnesses

- `swiftui/` — SwiftPM executable (`LayoutTwins`). Each fixture is wrapped in a
  `GeometryReader` in global coordinates; proposal answers come from a custom
  `ProbeLayout` (`Layout` protocol) that forwards explicit `ProposedViewSize`s
  and records `sizeThatFits` answers in the `probes` field. Frames are emitted
  as JSON between `TWINS_JSON_BEGIN/END` markers on stdout; `TWINS_CASE`
  selects the case, `TWINS_EXIT=1` terminates the run.
- `waterui/` — the same trees written against the public `waterui` API
  (`src/lib.rs`). Packaged with `water package`; hosted in-process by the
  `harness/` XCTest target, which walks the realized UIKit/AppKit view tree
  and dumps labeled frames the same way. `TWINS_CASE` is set in-process per
  test method.
- `measure.sh ios-sim <udid>` / `measure.sh macos` — builds and runs both
  sides and writes `results/`. `SIDES="swiftui"` skips the WaterUI half.
  The harness links every archive in the backend's products directory (the
  `___swift_bridge__` helpers are required, not just `libwaterui_app.a`).
  It needs an apple-backend checkout: `BACKEND_PATH` env, or a checkout at
  `<repo>/../apple-backend` / `~/repos/apple-backend` plus a
  `<repo>/backends/apple` symlink for `water package` (both are local
  environment, not committed).

## Toolchain

Xcode 26.6 (17F113) · Swift 6.3.3 · cargo/rustc 1.98.1 · water 0.4.3 ·
macOS 26.5.2 (25F84) host · iPhone 17 sim iOS 26.5 (23F73, scale 3) ·
iPad Pro 11-inch (M5) sim iOS 26.5 (23F77, scale 2) · macOS 26.5.2 (scale 1;
no scale-2 display on this machine) · apple-backend `3d87740cc` (4 commits
past the 0.3.0 pin, includes water-rs/apple-backend#259 empty-child
membership — visible in the A3 bare-`E` member slot)

## Environment notes

- Units are points; frames are in window coordinates (`x,y w×h`).
- The SwiftUI side is a real app window: 402×874 (iPhone), 834×1210 (iPad),
  900×450 (mac). The WaterUI side is XCTest-hosted: the runner window is
  393×852 on **both** sims (it does not fill the iPad display, and reports
  safeInsets `{0,0}` there) and 500×500 on macOS. Its iPhone safeInsets are
  `{62 top, 12 bottom}` vs the real app's `{62, 34}`.
- WaterUI macOS frames use flipped window coordinates (`meta.flippedY`); the
  "all" scrollview content extends to negative y. Compare sizes and
  intra-case offsets there, not absolute y.
- The two stacks scroll independently — absolute y values differ between
  sides; compare sizes and x offsets within each side.
- Label mapping: SwiftUI `a3.mem` ≡ WaterUI `twins.a3.framex` (the audit's
  `Frame{E}` member). WaterUI-only extras: `twins.a3.mem` (bare `E` member),
  `twins.a3.memr` (rigid member, child 20×10), `twins.a3.abs` (Absolute
  member), `twins.a11.inf` (v-scroll nested in h-scroll = unspecified-width
  viewport). SwiftUI-only extras: `a3.bare`/`a3.memr` (matching controls),
  `a2.probe`/`a11.probe` (`ProbeLayout` answer matrices).

## A2 — text (16.7pt mono, "mmmmmmmmmm mmmmmmmmmm"; ideal line 217×20)

iOS 26.5 scale 3 unless noted. Host = `H(sp:0){text}` under `.size(100,h)` /
`.frame(100,h)`.

| fixture | SwiftUI | WaterUI | observation |
|---|---|---|---|
| wrap @100×20 | text `93×20` | text `93×80` | S clamps to the 1-line proposal; W wraps to full height, ignoring the h clamp (overflows host) |
| lineLimit(1) @100×60 | `93×20` | `93×20` | agree |
| lineLimit(2) @100×60 | `93×40` | `93×20` | W `line_limit(2)` still reports a 1-line height |
| subline @100×8 | `93×20` | `93×80` | neither side suppresses drawing under a sub-line height |
| w=0,h=20 host | `0×20` | `103.3×40` | S collapses to zero width; W keeps ~103w and wraps to 2 lines |
| w=100,h=0 host | `93×20` | `93×80` | both draw past the zero-height host |
| short ("mm mm …") @100×60 | `93×60` | `93×60` | agree (3 lines) |
| text in scroll (w=None) | probe `(100,nil)→93×80` | `inf.text 93×80` | agree |

SwiftUI `sizeThatFits` answers for the text (probe `pw×ph → aw×ah`):

| proposal | answer | | proposal | answer |
|---|---|---|---|---|
| 100×20 | 93×20 | | 100×nil | 93×80 |
| 100×40 | 93×40 | | nil×20 | 217×20 |
| 0×20 | 0×20 | | nil×0 | 217×20 |
| 100×0 | 93×20 | | nil×nil | 217×20 |
| 0×0 | 0×0 | | ∞×20 | 217×20 |
| 0×nil | 0×398.67 | | 100×∞ | 93×80 |

WaterUI `None`-axis cells (driven via scroll wrappers; W has no proposal API):

| cell | WaterUI frame | nearest SwiftUI answer | observation |
|---|---|---|---|
| w=None,h=20 (`nonew`) | `217×20` | `(nil,20)→217×20` | agree — intrinsic single line |
| w=None,h=0 (`noneh0`) | `217×20` | `(nil,0)→217×20` | agree |
| w=None,h=None (`nonenone`) | `217×20` | `(nil,nil)→217×20` | agree |
| w=0,h=None (`w0none`) | `103.3×40` | `(0,nil)→0×398.67` | diverge — S wraps per-glyph at 0w; W keeps ~103w |

## A3 — wrapper membership `H(sp:10)[R20×10, W{D}, R20×10]` in 100×60 host

Member slot inferred from the `l`/`r` x positions; `.w` is the wrapper's own
recorded frame.

| W{D} | SwiftUI `.w` | WaterUI `.w` | observation |
|---|---|---|---|
| bare `E` (control) | `0×0` member slot | `0×0` member slot | both keep a 0-wide member; S keeps 10pt spacing both sides, W places E flush against the left rect (drops leading gap) |
| rigid `R` (control) | `20×10` member | `20×10` member | member is the child box, as expected |
| `Frame(40){R}` | `40×10` | `40×10` | agree |
| `Frame(40){E}` | `40×0` | `40×0` | agree — fixed-width frame is a member even when empty |
| `Padding(10){R}` | `40×30` | `40×30` | agree — padded box is the member |
| `Padding(10){E}` | `20×20` | `20×20` | agree — padding around empty reports padding only |
| `AspectRatio(2,fit){R}` | `20×10` | `40×20` | **diverge** — W forces a 2:1 box (fills proposal height); S keeps the child's size |
| `AspectRatio(2,fit){E}` | `0×0` | `40×20` | **diverge** — W reports the ratio box even for an empty child |
| `ZStack{R}` | `20×10` | `20×10` | agree |
| `ZStack{E}` | `0×0` | `0×0` | agree |
| `overlay(R20)` on R | `20×10` | `20×10` | agree |
| `overlay(R20)` on E | `w 0×0`; deco `20×10` centered on it | same | agree — deco draws centered on the empty child |
| `background(R20)` on R | `20×10` | `20×10` | agree |
| `background(R20)` on E | `w 0×0`; deco `20×10` | same | agree |
| leading guide −5 on R | `20×10`, no shift | `20×10`, no shift | agree — the guide does not move a stack member's slot |
| `Grid(1){R}` member | `20×10` | `20×10` | agree |
| `Grid(1){E}` member | `0×0` | `0×0` | agree |
| `Absolute{E pin 40×10}` | — (no twin) | `40×60` member | W-only: member slot 40 wide but fills the host's 60 height |

## A11 / D4 — scroll containers (host `.size`/`.frame` 100×80)

| fixture | SwiftUI | WaterUI | observation |
|---|---|---|---|
| v-scroll, content 60×300 | viewport `100×80`; content x +20 (centered) | viewport `100×80`; content x +20 | agree — D4's "hug content" does not show in the *placed* frame of a fixed host |
| v-scroll, content 140×300 | content x −20 | content x −20 | agree — wider-than-viewport content centered on cross axis |
| h-scroll, content 300×60 | content y +10 | content y +10 | agree — cross-axis centered |
| both-axis, content 60×30 | content +20,+25 (centered) | content at 0,0 (top-left) | **diverge** — W does not center cross-axis under a two-axis scroll |
| w=0 host | viewport `0×80`, content x −30 | viewport `0×80`, content x −30 | agree |
| inner v-scroll under w=None | `(nil,80)→60×80` (probe) | inner viewport `0×80` | **diverge** — S answers content-hug width 60; W viewport collapses to 0 |

SwiftUI scroll `sizeThatFits` answers:

| proposal | answer | | proposal | answer |
|---|---|---|---|---|
| 100×80 | 60×80 | | nil×80 | 60×80 |
| 0×80 | 60×80 | | 100×nil | 60×300 |
| ∞×80 | 60×80 | | 100×∞ | 60×∞ |

The viewport *answer* hugs the content on the cross axis (60) for every width
proposal, and hugs content height when the height is unbounded. See a12:
under a root proposal the hugged answer becomes the placed frame.

## A12 — root scroll against the window's safe area

SwiftUI window 402×874, safeInsets `{62 top, 34 bottom}`. WaterUI runner
window 393×852, safeInsets `{62 top, 12 bottom}`.

| case | SwiftUI | WaterUI | observation |
|---|---|---|---|
| `a12.scroll` | viewport `171,62 60×778`; content y=62 | viewport `0,62 393×778`; content y=62 | **diverge** — S cross-axis *hugs* content (60w); W fills the safe region width. Both respect top inset |
| `a12.ignore` | viewport `60×778` (still safe-bounded); content y=0 | viewport `0,0 393×852` (full window); content y=62 | opposite split — S keeps a safe viewport but draws content into the unsafe zone; W moves the viewport past the safe area but content still starts at 62 |
| `a12.bar` | bar `0,0 402×44` — touches the chrome | bar `0,62 393×44` — inside safe region | **diverge** — S overlay reaches the unsafe top edge; W overlay clamps to the safe area |

macOS s1 (AppKit window coords, bottom-left origin): S scroll `420,32
60×418` below the 32pt titlebar; W scroll `0,0 500×500` full runner window —
same viewport divergence as iOS. Both bars reach the window top: S bar
`0,0 900×44`, W bar `0,456 500×44` (the top 44pt strip of the 500-tall
window). The runner window has no safe insets, so the safe-area split of
`a12.ignore` does not appear on macOS.

## A13 — scale snapping

`H(sp:0){Fill,Fill,Fill}` under 100×10, and three 8.4pt mono texts.

| scale | SwiftUI fills | WaterUI fills | observation |
|---|---|---|---|
| 3 (iPhone) | `33.3333` ×3, contiguous | `33.3333` ×3, contiguous | agree |
| 2 (iPad) | `33.3333` ×3 | `33.5` ×3 → cumulative `100.5` | **diverge** — W snaps each fill up to the half-point, overflowing the host by 0.5pt; S keeps exact thirds |
| 1 (macOS) | `33.3333` ×3 | `34` ×3 → cumulative `102` | **diverge** — same pattern, 2pt overflow at s1 |

| scale | SwiftUI text h | WaterUI text h | observation |
|---|---|---|---|
| 3 | `10.3333` | `10.3333` | agree — snapped to whole pixels (31px) |
| 2 | `10.5` | `10.5` | agree (21px) |
| 1 | `10` | `10` | agree (10px) |

## D — contested divergences

| case | SwiftUI | WaterUI | observation |
|---|---|---|---|
| D1 grid, cells 20×10 + 80×10 in 200×50 | c1 `20×10`, c2 `80×10`, pair centered (intrinsic cols, spacing 8) | a `20×10`, b `80×10`, each centered in ~100-wide equal cols | **confirmed** — W grid = equal-width columns; S grid = intrinsic columns |
| D2 `AspectRatio(2,fit){R30×10}` in 100×100 | child keeps `30×10`, centered | child forced to `100×50` | **confirmed** — S aspect is proposal-only; W forces the ratio box |
| D3 `R100×50.background(R20×10)` in 100×50 | deco keeps `20×10`, centered | deco forced to `100×50` | **confirmed** — W background fills the base frame |
| D4 v-scroll cross-axis | viewport answer `60×80` (hug); placed `100×80` in a fixed host, `60×778` at root | viewport fills proposal `100×80` / `393×778` | **confirmed** — S scroll answers content width; W fills |
| D5 `Z{R100×100 p0, R20×20 p1}` in 200×200 | both children centered; envelope = union | same | **hypothesis not observed** — the priority child does not shrink the container on either side |
