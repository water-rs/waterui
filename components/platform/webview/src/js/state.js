// The reactive state mirror.
//
// Values pushed from Rust land here, so the page reads them as ordinary local
// properties — `app.state.theme` costs nothing and never crosses the boundary.
// Writes go the other way through the bridge.
//
// Keys are fixed when the web view is created, so each is a real property rather
// than a Proxy trap. The Proxy exists only to reject unknown keys: a typo like
// `state.thmee = "dark"` would otherwise create a new property and vanish, which
// is the classic way this kind of API wastes an afternoon.
(function () {
  if (globalThis.__wateruiState !== undefined) {
    return;
  }

  var mirror = Object.create(null);
  var epochs = Object.create(null);
  var watchers = Object.create(null);
  var target = Object.create(null);

  function notify(key, value) {
    var list = watchers[key];
    if (list === undefined) {
      return;
    }
    for (var index = 0; index < list.length; index += 1) {
      list[index](value);
    }
  }

  globalThis.__wateruiState = {
    // Declares a key. `writable` false means the page may read but not assign.
    define: function (key, value, epoch, writable) {
      mirror[key] = value;
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
          globalThis.waterui
            .invoke("__wateruiSetState", { key: key, value: next, epoch: epochs[key] })
            .catch(function (error) {
              if (globalThis.waterui.onerror) {
                globalThis.waterui.onerror(error);
              }
            });
        },
      });
    },

    // Applies one coalesced patch from Rust.
    apply: function (patch) {
      for (var key in patch) {
        if (!Object.prototype.hasOwnProperty.call(patch, key)) {
          continue;
        }
        var entry = patch[key];
        mirror[key] = entry.v;
        epochs[key] = entry.e;
        notify(key, entry.v);
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
