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

  // Ids are unique to this document, not just within it.
  //
  // A handler may still be awaiting I/O when the page navigates. The reply is
  // then evaluated in the *new* document, whose own `pending` map started over
  // from the same low numbers — so a stale reply could settle an unrelated call
  // with another call's value. Starting each document at a random point makes
  // an id from a previous document essentially certain not to be pending here,
  // and `__wateruiResolve` already ignores ids it does not know.
  //
  // 26 bits of document entropy and 26 bits of counter stay well inside the
  // range a double holds exactly, so the id crosses as an ordinary number.
  var nextId = Math.floor(Math.random() * 0x4000000) * 0x4000000;

  // Integers Rust can hold and a JS number cannot. JSON has no spelling for
  // them, so they cross as {"__wateruiBigInt": "123..."} and become BigInt
  // here; anything that fits in a double is an ordinary number and stays one,
  // so page arithmetic on normal values is unaffected. Shared with state.js
  // through the internal global below.
  var BIG_INT_TAG = "__wateruiBigInt";

  function reviveBigInts(value) {
    if (value === null || typeof value !== "object") {
      return value;
    }
    if (Array.isArray(value)) {
      for (var i = 0; i < value.length; i += 1) {
        value[i] = reviveBigInts(value[i]);
      }
      return value;
    }
    var keys = Object.keys(value);
    if (keys.length === 1 && keys[0] === BIG_INT_TAG && typeof value[BIG_INT_TAG] === "string") {
      return BigInt(value[BIG_INT_TAG]);
    }
    for (var k = 0; k < keys.length; k += 1) {
      value[keys[k]] = reviveBigInts(value[keys[k]]);
    }
    return value;
  }

  // JSON.stringify throws on a BigInt rather than guessing, so every value
  // leaving the page goes through this.
  function replaceBigInts(key, value) {
    if (typeof value === "bigint") {
      var tagged = {};
      tagged[BIG_INT_TAG] = value.toString();
      return tagged;
    }
    return value;
  }

  globalThis.__wateruiBigInts = {
    revive: reviveBigInts,
    replacer: replaceBigInts,
  };

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
    entry.resolve(payload ? reviveBigInts(payload.json) : undefined);
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
      globalThis.__wateruiSend(JSON.stringify(envelope, replaceBigInts));
    });
  }

  globalThis.waterui = {};

  // Not writable and not configurable, so page script cannot swap the transport
  // out from under the handlers — but the object itself stays extensible.
  // Freezing it whole is what this used to do, and `state.js` runs after this
  // file and adds `state` and `watch` to the same object: `defineProperty` on a
  // frozen object throws, so neither has ever existed in a page and the entire
  // mirrored-state feature was unreachable from JavaScript.
  Object.defineProperty(globalThis.waterui, "invoke", {
    value: function (name, payload) {
      return send(name, payload);
    },
    enumerable: true,
  });
})();
