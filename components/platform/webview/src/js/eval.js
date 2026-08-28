// The shared evaluation wrapper.
//
// Every backend runs user JavaScript through this, so the *result* is encoded in
// one place. Without it each engine reported values its own way: WebKit handed
// back an Objective-C object graph, Android a JSON-encoded string (so
// `document.title` arrived with its quotes), CDP a `RemoteObject`. Here the value
// is turned into JSON by the engine's own `JSON.stringify` — a C++ implementation
// no JavaScript-side codec can beat — and the envelope says explicitly whether the
// script threw, returned nothing, or produced something that cannot be
// represented.
//
// `fn` is called with the arguments the caller bound, so interpolated values
// never appear in the source text.
(function () {
  globalThis.__wateruiEval = async function (fn, args) {
    let value;
    try {
      value = await fn.apply(null, args);
    } catch (error) {
      return JSON.stringify({
        ok: false,
        message: String((error && error.message) || error),
        stack: (error && error.stack) || null,
      });
    }
    if (value === undefined) {
      // Distinct from an explicit null: the key is simply absent.
      return JSON.stringify({ ok: true });
    }
    try {
      return JSON.stringify({ ok: true, value: value });
    } catch (error) {
      return JSON.stringify({
        ok: false,
        unserializable: true,
        message: String((error && error.message) || error),
      });
    }
  };
})();
