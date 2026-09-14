# Driving an app over `water mcp`

## Contents

- Tool surface
- Finding nodes
- Acting
- Debugging animations
- Waiting
- Development loop

`water mcp` serves the project app headless over MCP. The client connects through the
generated `.mcp.json` (or registers `water mcp` by hand), and gets ten tools. Everything
the agent sees is the real accessibility tree — the same nodes `#[waterui::test]` drives.

## Tool surface

| Tool | Returns |
|---|---|
| `snapshot` | the accessibility tree (text lines, or `format: "json"`) |
| `find` | node lines matching criteria |
| `act` | semantic action on a node → settled tree |
| `pointer` | coordinate/element input → settled tree |
| `key` | named/character key + modifiers → settled tree |
| `type_text` | text into the focused input → settled tree |
| `wait` | `fulfilled`/`timed out`/… + tree |
| `screenshot` | PNG of the current frame (pumps one frame) |
| `advance` | virtual-clock step → `animating`/`settled` + tree, or PNG |
| `restart` | rebuild + relaunch → fresh tree |

Mutating tools return the settled tree — never follow up with `snapshot` just to see the
result. Node ids are stable until `restart`.

## Finding nodes

`find` and every `wait` expectation share one selector shape:

```json
{"role": "checkbox", "checked": false}
{"identifier": "settings.wifi"}
{"label_contains": "Wi-Fi", "within": 42}
{"value_contains": "Downloading", "enabled": true}
```

Criteria: `role` (snake_case name from `snapshot`), `label`, `label_contains`,
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

`key` takes a character or W3C name (`Enter`, `Tab`, `Escape`, arrows, `F1`–`F12`) plus
`modifiers` (`shift`, `ctrl`, `alt`, `meta`). `type_text` types into the focused input.

## Debugging animations

Every mutating tool settles the virtual clock before returning, so an animation normally
lands at its end state. To see intermediate frames:

1. `pointer`/`act`/`key`/`type_text` with `settle: false` — input is queued, not pumped.
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
```

`wait` returns `fulfilled`, `timed out`, `incorrect_order`, `inverted_fulfillment`, or
`interrupted` plus the tree. `inverted` forbids an expectation — a wait of only inverted
expectations runs the full timeout and fulfills if none ever held.

## Development loop

Edit source → `restart` (rebuilds under `water mcp`) → `snapshot`. For one component
without driving the app, the `preview` tool renders a `#[preview]` function or an
expression like `text("hi")` and returns the PNG directly. Frame benchmarks are a CLI
concern — `water bench`, not MCP.
