// The WaterUI JavaScript bridge.
//
// One copy, shared by every backend. Each backend supplies only a transport: a
// function that hands `__wateruiSend` a request envelope as a string, and a call
// back into `__wateruiResolve` with the reply.
//
// Page-facing API:
//
//   const reply = await waterui.invoke("greet", { name: "Lexo" });  // → { text: "Hi Lexo" }
//   const bytes = await waterui.invoke("read", "/a.txt");           // → Uint8Array
//
// One call for both: what comes back is decided by what the handler returns,
// which the page cannot influence. There was a second entry point that asked
// for bytes, but a handler returning a value could not honour it and one
// returning bytes needed no asking.
//
// `waterui.invoke` is namespaced deliberately: an earlier design installed each
// handler as `window[name]`, which silently overwrote page globals whenever a
// handler was called `open`, `name`, `print`, and so on.
(function () {
  if (globalThis.waterui !== undefined) {
    return;
  }

  var pending = new Map();
  var nextId = 0;

  function toBase64(bytes) {
    var binary = "";
    var view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    for (var i = 0; i < view.length; i += 1) {
      binary += String.fromCharCode(view[i]);
    }
    return btoa(binary);
  }

  function fromBase64(text) {
    var binary = atob(text);
    var bytes = new Uint8Array(binary.length);
    for (var i = 0; i < binary.length; i += 1) {
      bytes[i] = binary.charCodeAt(i);
    }
    return bytes;
  }

  // Called by the native side exactly once per request. An unknown id is ignored
  // rather than thrown: page script can reach this function, and a stray call must
  // not surface as an unhandled exception.
  globalThis.__wateruiResolve = function (id, ok, payload) {
    var entry = pending.get(id);
    if (entry === undefined) {
      return;
    }
    pending.delete(id);
    if (!ok) {
      var error = new Error(String((payload && payload.message) || "WaterUI handler failed"));
      if (payload && payload.data !== undefined) {
        error.data = payload.data;
      }
      entry.reject(error);
      return;
    }
    // Which branch runs is the handler's choice, not the caller's: a handler
    // that returns bytes sends `b64`, one that returns a value sends `json`.
    // Base64 is how bytes travel, never the answer itself, so it is always
    // decoded.
    if (payload && typeof payload.b64 === "string") {
      entry.resolve(fromBase64(payload.b64));
      return;
    }
    entry.resolve(payload ? payload.json : undefined);
  };

  function send(name, payload) {
    var id = (nextId += 1);
    return new Promise(function (resolve, reject) {
      pending.set(id, { resolve: resolve, reject: reject });
      var envelope = { id: id, name: String(name) };
      if (payload instanceof Uint8Array || payload instanceof ArrayBuffer) {
        envelope.b64 = toBase64(payload);
      } else {
        // JSON is the common path and stays out of base64: it is what the engine's
        // own JSON.stringify produces and what the native side parses directly.
        envelope.json = payload === undefined ? null : payload;
      }
      globalThis.__wateruiSend(JSON.stringify(envelope));
    });
  }

  globalThis.waterui = Object.freeze({
    invoke: function (name, payload) {
      return send(name, payload);
    },
  });
})();
