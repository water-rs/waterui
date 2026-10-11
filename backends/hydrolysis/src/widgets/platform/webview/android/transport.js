// Adapts the shared bridge's one-function transport onto Android's injected
// web message object, registered under `__wateruiBridge`.
//
// The main-frame check lives here as well as in Kotlin so that a call from a
// subframe *rejects* the page's promise: `send` in `js/bridge.js` calls this
// from inside the Promise executor, so throwing here is what the caller awaits.
// The authoritative refusal is still the native one — page script can replace
// this function, and does not get to authenticate itself — but a page that
// plays by the rules gets an answer instead of a promise that never settles.
//
// Replies come back on the same object: the native side posts each one through
// the `JavaScriptReplyProxy` of the message that asked, which the engine binds
// to this document and drops once the document is gone. Each reply is the
// `{ id, ok, payload }` message `bridge::Reply::message` renders.
(function () {
  var channel = __wateruiBridge;
  channel.onmessage = function (event) {
    var reply = JSON.parse(event.data);
    globalThis.__wateruiResolve(reply.id, reply.ok, reply.payload);
  };
  globalThis.__wateruiSend = function (envelope) {
    if (globalThis.top !== globalThis) {
      throw new Error("the WaterUI bridge is available to the main frame only");
    }
    channel.postMessage(envelope);
  };
})();
