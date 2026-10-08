// Runs one `call_async_javascript` body and reports the value its promise
// settles with.
//
// Android has no awaiting evaluation API: `WebView.evaluateJavascript` hands
// back the *synchronous* value of the script, and the shared wrapper in
// `js/eval.js` is `async`, so the envelope every typed evaluation needs would
// cross as `{}` — which is what made `WebView::eval`, `WebView::exec` and every
// mirrored-state push fail. The promise is therefore awaited here, in
// JavaScript, and the settled value is posted back through the web message
// listener the wrapper matches to the waiting native callback by id.
//
// A function expression: Rust composes a call as
// `(<this source>)(<call id>, async function () { <body> });` and evaluates it.
// The synchronous value a launched call evaluates to is the sentinel string
// `__wateruiAsyncCallStarted`, which the caller compares against the
// JSON-quoted form `evaluateJavascript` delivers.
(function (id, body) {
  var report = function (ok, value) {
    __wateruiAsyncResult.postMessage(
      JSON.stringify({ id: id, ok: ok, value: value }),
    );
  };
  var fail = function (error) {
    report(false, String((error && error.message) || error));
  };
  try {
    body().then(function (value) {
      // The shared wrapper always resolves with its JSON envelope as a string,
      // and the value crosses to Rust unmodified. Anything else is a broken
      // contract rather than a result to reshape into one.
      if (typeof value !== "string") {
        report(
          false,
          "WaterUI evaluation resolved with a " +
            typeof value +
            " rather than the shared wrapper's envelope",
        );
        return;
      }
      report(true, value);
    }, fail);
  } catch (error) {
    fail(error);
  }
  // Read by the wrapper: a script that fails to parse never runs a line of the
  // above, so nothing would ever settle the call. It is the one failure the
  // promise cannot report.
  return "__wateruiAsyncCallStarted";
});
