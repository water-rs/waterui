// Gives the checks the mirrored state they read back.
//
// `WebView::expose` declares keys through the seed and the values arrive as
// the reply to the document's first pull; the real-engine test drives the
// handle directly, so it declares the same two keys and applies the reply
// itself. Both are reached through `__wateruiState`, which `state.js` installs
// before it publishes `waterui.state`, so this file runs even when the
// page-facing half of the mirror is missing — which is precisely the state the
// checks have to be able to observe.
globalThis.__wateruiState.declare("theme", false);
globalThis.__wateruiState.declare("big", false);
globalThis.__wateruiState.apply({
  theme: { v: "dark", e: 1, w: false },
  big: { v: { __wateruiBigInt: "9007199254740993" }, e: 1, w: false },
});
