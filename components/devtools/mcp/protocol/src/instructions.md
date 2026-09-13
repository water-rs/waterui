You are connected to a live, headless WaterUI application. The app runs off
screen; everything you see and do goes through its accessibility tree.

## Reading the app

- Call `snapshot` to see the current accessibility tree. Every line is one
  node: `#<id> <role> "<label>"`, its value, state flags, the accessibility
  actions it supports, and its bounds.
- Call `find` to locate nodes by role, label, identifier, or value without
  dumping the whole tree.
- Call `screenshot` when layout or appearance matters — it returns a PNG of
  the current frame.

## Acting on the app

- Prefer `act`: it performs a semantic accessibility action (click, focus,
  set_value, increment, decrement, replace_text, expand, collapse,
  scroll_forward, scroll_backward) on a node id.
- Use `pointer` for interactions accessibility actions cannot express: taps
  and drags at coordinates, hover, secondary click, scroll wheel.
- Use `key` for named keys (Enter, Tab, Escape, arrows, …) with optional
  modifiers, and `type_text` to enter text into the focused input.
- Every mutating tool returns the settled accessibility tree, so no follow-up
  `snapshot` is needed.

## Waiting

- Use `wait` with `exists` / `not_exists` / `value_eq` expectations instead of
  polling `snapshot` in a loop. It returns as soon as the expectations hold or
  the timeout elapses.

## Housekeeping

- Node ids are stable across turns until `restart`.
- `restart` relaunches the app from scratch and returns the fresh tree; state
  resets. Under `water mcp` it rebuilds the app from the current sources
  first, so edit → `restart` → `snapshot` is the development loop.
- The viewport is fixed for the whole session; there is no resize tool.
