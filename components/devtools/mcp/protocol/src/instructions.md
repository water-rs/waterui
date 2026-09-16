You are connected to a live, headless WaterUI application. The app runs off
screen; everything you see and do goes through its accessibility tree.

## Reading the app

- Call `snapshot` to see the current accessibility tree. Every line is one
  node: `#<id> <role> "<label>"`, its value, state flags, the accessibility
  actions it supports, and its bounds.
- Call `find` to locate nodes without dumping the whole tree. Criteria: `role`,
  `label`, `label_contains`, `identifier`, `value`, `value_contains`, and the
  state filters `enabled`, `selected`, `checked`, `mixed`, `expanded`, `busy`,
  `hidden` — e.g. `{"role": "checkbox", "checked": false}`. Scope a search
  inside one element with `within` (descendants) or `children_of` (direct
  children), each taking a node id.
- Call `screenshot` when layout or appearance matters — it returns a PNG of
  the current frame.

## Acting on the app

- Prefer `act`: it performs a semantic accessibility action on a node id —
  `click`, `focus`, `set_value`, `increment`, `decrement`, `replace_text`,
  `expand`, `collapse`, `scroll_forward`, `scroll_backward`, `scroll_left`,
  `scroll_right`, `scroll_into_view`. The app-level `clear_focus` releases
  keyboard focus and needs no node.
- Use `pointer` for interactions accessibility actions cannot express: taps
  and drags, hover, secondary click, scroll wheel (`unit: "line"` for a
  clicky wheel), and `magnify` (pinch) with `factor`.
- `pointer` coordinates are viewport logical pixels — or fractions of a node
  when you pass `node` (`x: 0.5, y: 0.5` is its center); `to_node` anchors a
  drag's end the same way.
- Use `key` for named keys (Enter, Tab, Escape, arrows, …) with optional
  modifiers, and `type_text` to enter text into the focused input.
- Every mutating tool returns the settled accessibility tree, so no follow-up
  `snapshot` is needed.

## Debugging animations

- Settling runs a triggered animation to completion before the tool returns.
  Pass `settle: false` to `pointer`, `key`, `type_text`, or `act` to queue the
  input without settling, keeping the transient observable.
- `advance(duration_ms)` steps the virtual frame clock deterministically —
  16ms per frame — and reports `animating` or `settled` plus the tree.
  `advance(0)` just polls that status.
- `advance(duration_ms, screenshot: true)` returns the PNG rendered at exactly
  that instant; repeat with `duration_ms: 16` for a per-frame flipbook. A bare
  `screenshot` is also a pump — every capture advances the clock by one frame.
- Typical loop: `pointer tap settle=false` → `advance(16, screenshot=true)`
  repeated until `advance` reports `settled`.

## Waiting

- Use `wait` instead of polling `snapshot`. Expectations: `exists`,
  `not_exists`, `value_eq`, and `focus` (the node holds keyboard focus) — each
  takes the same selector criteria as `find`. It returns as soon as the
  expectations hold or `timeout_ms` elapses.
- `"inverted": true` on an expectation forbids it: the wait fails with
  `inverted_fulfillment` the moment it becomes true, and a wait holding only
  inverted expectations runs the full timeout then reports `fulfilled`.
- `enforce_order: true` requires the expectations to fulfill in the order
  given; a later one landing first reports `incorrect_order`.

## Housekeeping

- Node ids are stable across turns until `restart`.
- `restart` relaunches the app from scratch and returns the fresh tree; state
  resets. Under `water mcp` it rebuilds the app from the current sources
  first, so edit → `restart` → `snapshot` is the development loop.
- The viewport is fixed for the whole session; there is no resize tool.
