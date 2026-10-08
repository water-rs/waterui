// Adapts the shared bridge's one-function transport onto Android's injected
// web message object, registered under `__wateruiBridge`.
//
// The main-frame check lives here as well as in Kotlin so that a call from a
// subframe *rejects* the page's promise: `send` in `js/bridge.js` calls this
// from inside the Promise executor, so throwing here is what the caller awaits.
// The authoritative refusal is still the native one — page script can replace
// this function, and does not get to authenticate itself — but a page that
// plays by the rules gets an answer instead of a promise that never settles.
(function () {
  globalThis.__wateruiSend = function (envelope) {
    if (globalThis.top !== globalThis) {
      throw new Error("the WaterUI bridge is available to the main frame only");
    }
    __wateruiBridge.postMessage(envelope);
  };
})();
