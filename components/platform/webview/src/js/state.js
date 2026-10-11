// The reactive state mirror.
//
// Values from Rust land here, so the page reads them as ordinary local
// properties — `app.state.theme` costs nothing and never crosses the boundary.
// Writes go the other way through the bridge.
//
// Rust never sends state unasked. The seed defines every key and hands `start`
// the cursor its values are current at; from then on the document keeps one
// `__wateruiPullState` call open, and Rust answers it once something changed
// after that cursor. The answer is a reply to this document's own call, so the
// engine delivers it here or nowhere.
//
// Keys are fixed when the web view is created, so each is a real property rather
// than a Proxy trap. The Proxy exists only to reject unknown keys: a typo like
// `state.thmee = "dark"` would otherwise create a new property and vanish, which
// is the classic way this kind of API wastes an afternoon.
(function () {
  if (globalThis.__wateruiState !== undefined) {
    return;
  }

  // Captured now, while the bridge is the one that just ran, so the pull loop
  // and writes keep reaching it after page script reassigns `waterui`.
  var invoke = globalThis.waterui.invoke;

  var mirror = Object.create(null);
  var epochs = Object.create(null);
  var watchers = Object.create(null);
  var target = Object.create(null);

  // A watcher is page code. One that throws is reported like any uncaught
  // error, and neither stops the watchers after it nor the pull loop that
  // delivers every later change.
  function notify(key, value) {
    var list = watchers[key];
    if (list === undefined) {
      return;
    }
    for (var index = 0; index < list.length; index += 1) {
      try {
        list[index](value);
      } catch (error) {
        globalThis.reportError(error);
      }
    }
  }

  // The cursor of the state this document holds, and which pull loop is the
  // live one. A document restored from the back/forward cache starts a new
  // loop; the old loop's call may still be answered, and is then ignored.
  var cursor = null;
  var generation = 0;
  var started = false;

  function pull(loop) {
    invoke("__wateruiPullState", { since: cursor }).then(
      function (reply) {
        if (loop !== generation) {
          return;
        }
        // The next pull is open before any page code runs for this one.
        cursor = reply.cursor;
        pull(loop);
        globalThis.__wateruiState.apply(reply.changes);
      },
      function (error) {
        if (loop !== generation) {
          return;
        }
        if (globalThis.waterui.onerror) {
          globalThis.waterui.onerror(error);
        }
      }
    );
  }

  function restart(since) {
    cursor = since;
    generation += 1;
    pull(generation);
  }

  globalThis.__wateruiState = {
    // Declares a key. `writable` false means the page may read but not assign.
    define: function (key, value, epoch, writable) {
      mirror[key] = globalThis.__wateruiBigInts.revive(value);
      epochs[key] = epoch;
      Object.defineProperty(target, key, {
        enumerable: true,
        configurable: true,
        get: function () {
          return mirror[key];
        },
        set: function (next) {
          if (!writable) {
            throw new TypeError(
              "WaterUI state '" + key + "' is read-only; it is derived in Rust"
            );
          }
          // Update locally first so a read in the same turn is consistent, then
          // send. Rust answers with the authoritative value, which may differ.
          mirror[key] = next;
          notify(key, next);
          invoke("__wateruiSetState", { key: key, value: next, epoch: epochs[key] })
            .catch(function (error) {
              if (globalThis.waterui.onerror) {
                globalThis.waterui.onerror(error);
              }
            });
        },
      });
    },

    // Starts pulling from `since`, the cursor the seed's values are current at.
    start: function (since) {
      if (!started) {
        started = true;
        // A document restored from the back/forward cache runs no seed, and
        // holds the state it had when it was frozen; it asks for every key.
        globalThis.addEventListener("pageshow", function (event) {
          if (event.persisted) {
            restart(null);
          }
        });
      }
      restart(since);
    },

    // Applies the keys one reply carries.
    apply: function (changes) {
      for (var key in changes) {
        if (!Object.prototype.hasOwnProperty.call(changes, key)) {
          continue;
        }
        var entry = changes[key];
        // A reply is self-sufficient: a key the document never saw seeded
        // still gets its property defined here, rather than being left with a
        // mirror whose keys throw on every read.
        if (!(key in target)) {
          globalThis.__wateruiState.define(key, entry.v, entry.e, entry.w);
          continue;
        }
        var value = globalThis.__wateruiBigInts.revive(entry.v);
        mirror[key] = value;
        epochs[key] = entry.e;
        // The revived value, not the wire form. A watcher used to be handed
        // `{__wateruiBigInt: "…"}` while `waterui.state.x` held the BigInt, so
        // arithmetic inside a watcher silently operated on an object.
        notify(key, value);
      }
    },
  };

  var state = new Proxy(target, {
    get: function (object, key) {
      if (typeof key === "string" && !(key in object)) {
        throw new ReferenceError("unknown WaterUI state key '" + key + "'");
      }
      return object[key];
    },
    set: function (object, key, value) {
      if (typeof key === "string" && !(key in object)) {
        throw new ReferenceError("unknown WaterUI state key '" + key + "'");
      }
      object[key] = value;
      return true;
    },
  });

  Object.defineProperty(globalThis.waterui, "state", {
    value: state,
    enumerable: true,
  });

  Object.defineProperty(globalThis.waterui, "watch", {
    value: function (key, callback) {
      if (!(key in target)) {
        throw new ReferenceError("unknown WaterUI state key '" + key + "'");
      }
      var list = watchers[key] || (watchers[key] = []);
      list.push(callback);
      return function () {
        var index = list.indexOf(callback);
        if (index >= 0) {
          list.splice(index, 1);
        }
      };
    },
    enumerable: true,
  });
})();
