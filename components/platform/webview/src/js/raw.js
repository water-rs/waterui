// The raw evaluation wrapper behind `WebViewHandle::run_javascript`.
//
// Engines report an evaluated value each their own way — WebKit as an
// Objective-C object graph, CDP as a `RemoteObject`, JavaScriptCore-GLib as a
// `JSCValue`, Android as text that is already JSON — and every backend used to
// turn that into JSON with its own encoder, so a string came back quoted on one
// engine and bare on the next. The value is therefore encoded here, by the
// page's own `JSON.stringify`, and the evaluation always yields a string that
// every backend hands back unchanged.
//
// The raw path also runs in documents WaterUI installed nothing into — a page
// outside the bridge policy, one that redefined `__wateruiEval` — so this is not
// an injected global. It is a function expression Rust inlines into every raw
// evaluation as
//
//   (<this source>)((
//   <expression>
//   ))
//
// The expression is the argument: the doubled parentheses keep a comma
// expression one value, and the line breaks keep a trailing `//` comment from
// swallowing the call. Its exceptions propagate out of the evaluation untouched,
// and so does the `TypeError` `JSON.stringify` throws on a cycle or a `BigInt`.
// `undefined`, a function and a symbol have no JSON spelling, so
// `JSON.stringify` answers `undefined` for them and the reply is `null`. A
// promise is not awaited: it has no enumerable properties and encodes as `{}`.
function (value) {
  const json = JSON.stringify(value);
  return json === undefined ? "null" : json;
}
