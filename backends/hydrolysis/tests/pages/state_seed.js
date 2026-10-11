// Seeds the mirrored state the checks read back.
//
// `WebView::expose` emits calls of exactly this shape once the view is built;
// the real-engine test drives the handle directly, so it seeds the same two
// keys itself. `define` is reached through `__wateruiState`, which `state.js`
// installs before it publishes `waterui.state`, so this file runs even when the
// page-facing half of the mirror is missing — which is precisely the state the
// checks have to be able to observe.
globalThis.__wateruiState.define("theme", "dark", 1, false);
globalThis.__wateruiState.define(
  "big",
  { __wateruiBigInt: "9007199254740993" },
  1,
  false
);
