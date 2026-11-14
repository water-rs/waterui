// What the page half of the bridge looks like from inside a real engine.
//
// Everything here is observation, not assertion: each answer is recorded and
// the whole record is handed to Rust through `waterui.invoke`, which is the
// only channel out. The assertions live in `tests/real_engine.rs`, so a failure
// prints the value that was actually seen rather than a JavaScript stack with
// no context.
//
// The record travels through `JSON.stringify` with the bridge's own BigInt
// replacer, so every field here is a string, a number or a boolean.
(async function () {
  var record = { page: document.title, location: location.href };
  try {
    // The bridge object itself. `state` and `watch` are added by `state.js`
    // after `bridge.js` has already defined `invoke`, so a `waterui` that were
    // frozen would leave these two undefined while `invoke` still worked.
    record.invokeType = typeof waterui.invoke;
    record.stateType = typeof waterui.state;
    record.watchType = typeof waterui.watch;
    record.stateTheme = waterui.state.theme;
    record.stateBigType = typeof waterui.state.big;
    record.stateBigText = String(waterui.state.big);
    record.watchResultType = typeof waterui.watch("theme", function () {});

    // A handler returning text. The value itself has to arrive, not its base64.
    var greeting = await waterui.invoke("greet", { name: "Lexo" });
    record.greetingType = typeof greeting;
    record.greeting = greeting;

    // Rust to page, for an integer a double cannot hold and one it can.
    var large = await waterui.invoke("largest-id", null);
    record.largeType = typeof large;
    record.largeText = String(large);
    var small = await waterui.invoke("small-id", null);
    record.smallType = typeof small;
    record.smallValue = small;

    // Page to Rust, same two values. The reply is the payload Rust parsed,
    // spelled back as text, so the page sees what crossed as well.
    record.echoed = await waterui.invoke("echo-id", {
      id: 9007199254740993n,
      small: 42,
    });
  } catch (error) {
    record.failure = String((error && error.stack) || error);
  }
  await waterui.invoke("report", record);
})();
