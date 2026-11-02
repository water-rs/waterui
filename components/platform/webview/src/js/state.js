// The reactive state mirror.
//
// Values from Rust land here, so the page reads them as ordinary local
// properties — `app.state.theme` costs nothing and never crosses the boundary.
// Writes go the other way through the bridge.
//
// Rust never sends state unasked, and no value is ever part of injected text.
// The seed declares every key and starts the loop; the document's first
// `__wateruiPullState` call asks for every value, and from then on it keeps one
// call open, which Rust answers once something changed after the cursor the
// last reply named. The answer is a reply to this document's own call, so the
// engine delivers it here or nowhere.
//
// Until the first reply lands the document holds no value, and it does not
// pretend to: `waterui.ready` is the boundary. Reading or assigning a key
// before it resolves throws rather than producing a default, `undefined`, or
// a value fetched synchronously; watching a declared key is allowed, and the
// watcher hears the first values when they arrive.
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

  // The ready boundary: settled by the first reply, rejected when the first
  // pull fails. A page that never awaits it still hears of that failure
  // through `waterui.onerror`, so the rejection is not also reported as an
  // unhandled one.
  var ready = false;
  var settleReady;
  var failReady;
  var readyPromise = new Promise(function (resolve, reject) {
    settleReady = resolve;
    failReady = reject;
  });
  readyPromise.catch(function () {});

  function becomeReady() {
    if (!ready) {
      ready = true;
      settleReady();
    }
  }

  function notReady(key) {
    return new Error(
      "WaterUI state '" + key + "' has no value before `waterui.ready` resolves"
    );
  }

  // A watcher is page code. One that throws is reported like any uncaught
  // error, and neither stops the watchers after it nor the pull loop that
  // delivers every later change. Deliveries iterate a snapshot, so what a
  // watcher does inside its own call never reorders this notification.
  function notify(key, value) {
    var list = watchers[key];
    if (list === undefined) {
      return;
    }
    var snapshot = list.slice();
    for (var index = 0; index < snapshot.length; index += 1) {
      if (!snapshot[index].active) {
        continue;
      }
      try {
        snapshot[index].callback(value);
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
        if (!ready) {
          failReady(error);
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
    // A declared key has no value until the first reply.
    declare: function (key, writable) {
      Object.defineProperty(target, key, {
        enumerable: true,
        configurable: true,
        get: function () {
          if (!ready) {
            throw notReady(key);
          }
          return mirror[key];
        },
        set: function (next) {
          if (!writable) {
            throw new TypeError(
              "WaterUI state '" + key + "' is read-only; it is derived in Rust"
            );
          }
          if (!ready) {
            throw notReady(key);
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

    // Starts the pull loop. A document with no keys has nothing to pull, so
    // it is ready at once.
    start: function (pulls) {
      if (!pulls) {
        becomeReady();
        return;
      }
      if (!started) {
        started = true;
        // A document restored from the back/forward cache holds the state it
        // had when it was frozen; it asks for every key again.
        globalThis.addEventListener("pageshow", function (event) {
          if (event.persisted) {
            restart(null);
          }
        });
      }
      restart(null);
    },

    // Applies the keys one reply carries.
    //
    // Every value lands before any watcher runs, and the first reply makes
    // the document ready before its watchers run, so a watcher that reads a
    // sibling key sees this reply's value rather than a not-ready error.
    apply: function (changes) {
      var delivered = [];
      for (var key in changes) {
        if (!Object.prototype.hasOwnProperty.call(changes, key)) {
          continue;
        }
        var entry = changes[key];
        // A reply is self-sufficient: a key the document never saw declared
        // still gets its property defined here, rather than being left with a
        // mirror whose keys throw on every read.
        if (!(key in target)) {
          globalThis.__wateruiState.declare(key, entry.w);
        }
        // The revived value, not the wire form. A watcher used to be handed
        // `{__wateruiBigInt: "…"}` while `waterui.state.x` held the BigInt, so
        // arithmetic inside a watcher silently operated on an object.
        mirror[key] = globalThis.__wateruiBigInts.revive(entry.v);
        epochs[key] = entry.e;
        delivered.push(key);
      }
      becomeReady();
      for (var index = 0; index < delivered.length; index += 1) {
        notify(delivered[index], mirror[delivered[index]]);
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

  // Resolves once the document holds every value — the first reply landed —
  // and rejects when the first pull fails.
  Object.defineProperty(globalThis.waterui, "ready", {
    value: readyPromise,
    enumerable: true,
  });

  Object.defineProperty(globalThis.waterui, "watch", {
    value: function (key, callback) {
      if (!(key in target)) {
        throw new ReferenceError("unknown WaterUI state key '" + key + "'");
      }
      var list = watchers[key] || (watchers[key] = []);
      var registration = { callback: callback, active: true };
      list.push(registration);
      // A handle owns exactly its registration: `active` is the one source
      // of truth, so a repeat call cannot remove a sibling's record.
      return function () {
        if (!registration.active) {
          return;
        }
        registration.active = false;
        list.splice(list.indexOf(registration), 1);
      };
    },
    enumerable: true,
  });
})();
