# Driving an app over `water mcp`

## Contents

- Tool surface
- Finding nodes
- Acting
- Debugging animations
- Waiting
- Development loop
- Session and startup failures

`water mcp` serves the project app headless over MCP. The client connects through the
generated `.mcp.json` (or registers `water mcp` by hand), and gets twelve tools: eleven
drive the running app, `preview` renders a component. Everything the agent sees is the
real accessibility tree — the same nodes `#[waterui::test]` drives. Flags: `--path`
(project directory), `--viewport WIDTHxHEIGHT` (default `390x844`, fixed for the
session; there is no resize tool), `--scale` (default `2`), `--offline`.

## Tool surface

| Tool | Returns |
|---|---|
| `snapshot` | the accessibility tree (text lines, or `format: "json"`) |
| `find` | node lines matching criteria |
| `act` | semantic action on a node → settled tree |
| `pointer` | coordinate/element input → settled tree |
| `key` | named/character key + modifiers → settled tree |
| `drop_files` | OS file drop at a point/node → settled tree |
| `type_text` | text into the focused input → settled tree |
| `wait` | `fulfilled`/`timed out`/… + tree |
| `screenshot` | PNG of the current frame (pumps one frame) |
| `advance` | virtual-clock step → `animating`/`settled` + tree, or PNG |
| `restart` | rebuild + relaunch → fresh tree |
| `preview` | PNG of a `#[preview]` function or expression (no running app needed) |

Mutating tools return the settled tree — never follow up with `snapshot` just to see the
result. Node ids are stable until `restart`.

## Finding nodes

`find` and every `wait` expectation share one selector shape:

```json
{"role": "check_box", "checked": false}
{"identifier": "settings.wifi"}
{"label_contains": "Wi-Fi", "within": 42}
{"value_contains": "Downloading", "enabled": true}
```

Criteria: `role` (the snake_case AccessKit name `snapshot` prints — `button`,
`check_box`, `combo_box`, `text_input`, `progress_indicator`; an unknown name fails and
lists the valid ones), `label`, `label_contains`,
`identifier` (the `a11y_id`), `value`, `value_contains`, and state filters `enabled`,
`selected`, `checked`, `mixed`, `expanded`, `busy`, `hidden` (hidden nodes are excluded
unless `hidden: true`). `within`/`children_of` scope the match inside one node id.

## Acting

`act` is preferred — semantic actions survive layout changes:

```json
{"node": 42, "action": "click"}
{"node": 7, "action": "set_value", "value": "0.8"}
{"node": 15, "action": "scroll_into_view"}
{"action": "clear_focus"}
```

Actions: `click`, `focus`, `set_value`, `increment`, `decrement`, `replace_text`,
`expand`, `collapse`, `scroll_forward`, `scroll_backward`, `scroll_left`,
`scroll_right`, `scroll_into_view`, plus app-level `clear_focus` (no `node`).

`pointer` covers what semantics cannot:

```json
{"kind": "tap", "x": 120, "y": 340}
{"kind": "tap", "node": 42, "x": 0.5, "y": 0.5}
{"kind": "drag", "node": 3, "x": 0.5, "y": 0.5, "to_node": 8, "to_x": 0.5, "to_y": 0.5, "steps": 8}
{"kind": "scroll", "x": 200, "y": 400, "dy": -300}
{"kind": "scroll", "node": 12, "x": 0.5, "y": 0.5, "dy": -3, "unit": "line"}
{"kind": "magnify", "node": 5, "x": 0.5, "y": 0.5, "factor": 2.0}
```

Kinds: `tap`, `down`, `up`, `move`, `hover`, `secondary_click`, `drag`, `scroll`,
`magnify`. With `node`/`to_node` set, coordinates are `0`–`1` fractions of that node's
bounds (values outside extrapolate past the edge); without them they are viewport
logical pixels. `scroll` deltas are pixels unless `unit: "line"`.

`key` takes a single character or a W3C name (`Enter`, `Tab`, `Escape`, `Backspace`,
`Delete`, `ArrowLeft`/`Right`/`Up`/`Down`, `Home`, `End`, `PageUp`, `PageDown`,
`F1`–`F12`) plus `modifiers` (`shift`, `ctrl`, `alt`, `meta`). `type_text` types into the focused input.

`drop_files` delivers an OS file drop: absolute `paths` plus a point (`x`, `y`, or a
`node` anchor like `pointer`). The drop destination under the point sees the same
`FileHovered` → `FileDropped` input a platform drag emits and receives one `Files`
payload. A hover without the drop has no tool — `waterui-testing` callers use
`queue_hover_files_at`/`queue_cancel_files_hover` instead.

## Debugging animations

Every mutating tool settles the virtual clock before returning, so an animation normally
lands at its end state. To see intermediate frames:

1. `pointer`/`act`/`key`/`type_text`/`drop_files` with `settle: false` — input is queued,
   not pumped, and the tool answers `queued` instead of a tree.
2. `advance(duration_ms)` — steps the clock in fixed 16ms frames, reports `animating` or
   `settled` plus the tree. `advance(0)` polls status.
3. `advance(16, {"screenshot": true})` — PNG at exactly that instant; repeat for a
   per-frame flipbook.

The clock is virtual: results do not depend on host timing, and a `screenshot` is itself
a one-frame pump.

## Waiting

```json
{"exists": {"role": "progress_indicator", "busy": false}}
{"focus": {"identifier": "form.name"}}
{"exists": {"label": "Saved"}, "not_exists": {"label": "Error"}, "enforce_order": true}
{"exists": {"label": "Crash"}, "inverted": true, "timeout_ms": 2000}
{"value_eq": {"identifier": "counter", "value": "3"}}
```

Expectations: `exists`, `not_exists`, `value_eq` (selector plus `value`), `focus`, at
most one of each; each takes the `find` selector and an optional `inverted`.
`enforce_order` applies to the non-inverted ones, and `timeout_ms` defaults to 5000.
`wait` returns `fulfilled`, `timed out`, `incorrect_order`, `inverted_fulfillment`, or
`interrupted` plus the tree. `inverted` forbids an expectation — a wait of only inverted
expectations runs the full timeout and fulfills if none ever held.

## Development loop

Edit source → `restart` (rebuilds under `water mcp`) → `snapshot`. For one component
without driving the app, the `preview` tool renders a `#[preview]` function or an
expression like `text("hi")` and returns the PNG directly. Its arguments mirror
`water preview`: `target` (a function path such as `views::home`, or the expression
when `expr: true`), `frame` (`WIDTHxHEIGHT`, default `375x667`), `backend`
(`apple`/`hydrolysis`), `theme` (`material3`), `platform`
(`ios`/`macos`/`android`/`linux`/`windows`), `device` (Android only),
`content_timeout` (seconds, default `10`). Frame benchmarks are a CLI concern —
`water bench`, not MCP.

## Session and startup failures

The app runs on Hydrolysis and renders on the GPU. A hardware GPU is preferred; a
software adapter (Vulkan: Mesa lavapipe) is used when the host has none, so a cloud VM
without a GPU needs a software driver installed. With no eligible adapter at all the app
panics at startup. `WATER_HYDROLYSIS_FORCE_FALLBACK_ADAPTER=1` makes the software adapter
win even where a GPU exists, to reproduce a software-adapter run.

`water mcp` answers the client's `initialize` and `tools/list` itself, so a cold first
build never stalls the handshake; tool calls made during the build wait for it. The app's
panic is written to `water mcp`'s stderr (app logs flow there too), while every tool call
fails with "the app binary did not answer `initialize`" — read the server's stderr for
the cause. A build failure does not end the session either: tool calls report the error
until `restart` rebuilds. The session lasts as long as the client keeps stdio open;
closing it kills the app.
