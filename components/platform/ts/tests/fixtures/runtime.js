// A stand-in for the runtime global a real bundle publishes.
//
// Rust tests cannot bundle — the transform and the bundler are the CLI's, and
// they are not built here — so this classic script installs the same
// `globalThis.__waterui_runtime` shape over a tiny push-based signal
// implementation. It is deliberately minimal: what the bridge calls is
// `read`, `write`, `subscribe`, `isSignal`, `createSignal`, `createMemo` and
// `makeCallback`, and each one behaves exactly as HOST.md describes.
//
// Minimal is not "simplified". Where a difference would hide a defect from
// the Rust tests it mirrors the real library exactly: `set` takes an updater
// function like the real one, so `write` has to use one to store a value
// verbatim; a signal deduplicates on SameValue, so NaN and signed zero behave
// as they do in `signals.js`; `write` skips a value the target already holds,
// compared structurally as the seam compares it; `subscribe` follows a plain
// thunk by tracking what it reads, and `toSignal` accepts a `{ read }` value
// that announces nothing; `mount` materializes the host's environment before
// the tree runs and releases it on dispose; and `makeCallback` dispatches
// through the installed host's `invoke`, not a global read per call.
// `fixture-parity` in the bun suite runs the same scenarios against this
// script and the real library and asserts they observe the same thing.
//
// Everything the tests need to observe hangs off `globalThis.fixture`: the
// counters that prove one change propagates once, and a slot for handing a
// value between Rust and JavaScript.

(function () {
  "use strict";

  const fixture = {
    // `write()` calls the bridge made — the Rust-to-JavaScript direction.
    writes: 0,
    // Every `set` on a fixture signal, whoever made it.
    sets: 0,
    // Signals the bridge asked for: one per value exported out of Rust.
    signals: 0,
    subscribes: 0,
    disposes: 0,
    // A slot the tests park a value in, so both sides can reach it.
    held: undefined,
  };

  // What a tracked run read. A plain thunk says nothing about what it
  // depends on, so `subscribe` learns it the way the library's effects do:
  // run the thunk with tracking on and record every signal it touched.
  let tracking = null;

  function trackedRun(compute) {
    const previous = tracking;
    const read = new Set();
    tracking = read;
    try {
      compute();
    } finally {
      tracking = previous;
    }
    return read;
  }

  // Where a subscription made while a tree is mounting is parked, so the
  // mount's `dispose` releases it — the fixture's stand-in for `onCleanup`.
  let cleanups = null;

  // `options.equals` is the library's own: a comparator, or `false` for a
  // signal that calls every write a change. The bridge never passes one, but
  // a module does, and `write` answers with the target's own comparator — so
  // a fixture that hard-coded SameValue would settle writes the library
  // reports differently.
  function equalsOf(options) {
    const equals = options?.equals;
    if (equals === undefined) {
      return Object.is;
    }
    if (equals === false) {
      return () => false;
    }
    if (typeof equals === "function") {
      return equals;
    }
    throw new TypeError('"equals" must be a comparison function or false');
  }

  function createSignal(initial, options) {
    fixture.signals += 1;
    let value = initial;
    const equals = equalsOf(options);
    const subscribers = new Set();
    const signal = () => {
      if (tracking !== null) {
        tracking.add(signal);
      }
      return value;
    };
    signal.__signal = true;
    // Like the real `set`: a function argument is an updater, called with the
    // current value, and the write is deduplicated on SameValue.
    signal.set = (next) => {
      fixture.sets += 1;
      const resolved = typeof next === "function" ? next(value) : next;
      if (equals(value, resolved)) {
        return;
      }
      value = resolved;
      for (const subscriber of [...subscribers]) {
        subscriber(value);
      }
    };
    signal.__equals = equals;
    signal.__subscribe = (callback) => {
      subscribers.add(callback);
      fixture.subscribes += 1;
      return () => {
        subscribers.delete(callback);
        fixture.disposes += 1;
      };
    };
    return signal;
  }

  // A read-only view of whatever it computes: the bridge creates one over a
  // signal it pushes into, and `toSignal` creates one over a thunk. Both the
  // read and the subscription forward to the computation, which `subscribe`
  // knows how to follow whether it announces itself or has to be tracked.
  function createMemo(compute) {
    const memo = () => compute();
    memo.__memo = true;
    memo.__subscribe = (callback) => subscribe(compute, callback);
    return memo;
  }

  const isSignal = (value) => typeof value === "function" && value.__signal === true;

  const isMemo = (value) => typeof value === "function" && value.__memo === true;

  const isAccessor = (value) =>
    typeof value === "function" ||
    (value !== null && typeof value === "object" && typeof value.read === "function");

  function read(value) {
    if (typeof value === "function") {
      return value();
    }
    if (value !== null && typeof value === "object" && typeof value.read === "function") {
      return value.read();
    }
    return value;
  }

  // Like the library's own `write`: an updater so the value is stored and
  // never called, and the answer is whether it stood, compared with the
  // comparator the target itself settles on. A value the target already
  // holds — by the seam's structural equality, because everything crossing
  // from Rust is a fresh copy — is not written at all and stands.
  const comparatorOf = (source) => source?.__equals ?? Object.is;

  const MAX_BRIDGE_DEPTH = 128;

  function isPlainObject(value) {
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      return false;
    }
    const prototype = Object.getPrototypeOf(value);
    return prototype === Object.prototype || prototype === null;
  }

  function equalAtDepth(a, b, depth) {
    if (Object.is(a, b)) {
      return true;
    }
    if (depth >= MAX_BRIDGE_DEPTH) {
      return false;
    }
    if (Array.isArray(a) && Array.isArray(b)) {
      return (
        a.length === b.length && a.every((item, index) => equalAtDepth(item, b[index], depth + 1))
      );
    }
    if (isPlainObject(a) && isPlainObject(b)) {
      const keys = Object.keys(a);
      return (
        keys.length === Object.keys(b).length &&
        keys.every((key) => Object.hasOwn(b, key) && equalAtDepth(a[key], b[key], depth + 1))
      );
    }
    return false;
  }

  const bridgeEquals = (a, b) => equalAtDepth(a, b, 0);

  function write(target, value) {
    fixture.writes += 1;
    if (isSignal(target)) {
      if (bridgeEquals(read(target), value)) {
        return true;
      }
      target.set(() => value);
      return comparatorOf(target)(read(target), value);
    }
    if (target !== null && typeof target === "object" && typeof target.write === "function") {
      if (bridgeEquals(read(target), value)) {
        return true;
      }
      target.write(value);
      return comparatorOf(target)(read(target), value);
    }
    throw new TypeError("write() expects a signal or a writable reactive value");
  }

  function subscribe(source, callback) {
    if (source !== null && typeof source === "object" && typeof source.subscribe === "function") {
      return source.subscribe(callback);
    }
    if (typeof source === "function" && typeof source.__subscribe === "function") {
      return source.__subscribe(callback);
    }
    if (typeof source === "function") {
      // A plain thunk, which the library subscribes to with an effect. Here
      // its dependencies are learned once by tracking and answered with the
      // recomputed value; a thunk that reads nothing reactive announces
      // nothing, exactly as an effect over constants would.
      const disposers = [...trackedRun(source)].map((dependency) =>
        dependency.__subscribe(() => callback(source())),
      );
      return () => {
        for (const dispose of disposers) {
          dispose();
        }
      };
    }
    throw new TypeError("subscribe() expects a signal, an accessor, or a { read, subscribe } value");
  }

  function toAccessor(value) {
    if (typeof value === "function") {
      return value;
    }
    if (value !== null && typeof value === "object" && typeof value.read === "function") {
      return () => value.read();
    }
    return () => value;
  }

  function toSignal(source) {
    if (isSignal(source) || isMemo(source)) {
      return source;
    }
    if (source !== null && typeof source === "object" && typeof source.read === "function") {
      const signal = createSignal(source.read());
      if (typeof source.subscribe === "function") {
        const dispose = source.subscribe((value) => signal.set(() => value));
        if (cleanups !== null) {
          cleanups.push(dispose);
        }
      }
      // A `{ read }` value that announces nothing is a value that never
      // changes: it seeds the signal and nothing more, rather than failing
      // the way subscribing to it would.
      return signal;
    }
    if (typeof source === "function") {
      return createMemo(source);
    }
    return createSignal(source);
  }

  // Through the installed host's `invoke`, captured once, exactly as the real
  // `makeCallback` does: reading the mutable global per call would let a
  // bundle divert every callback in the runtime.
  function requireHost() {
    if (fixture.host === undefined) {
      throw new Error(
        "waterui: no host installed — installHost() must run before a module is mounted",
      );
    }
    return fixture.host;
  }

  function makeCallback(id) {
    const { invoke } = requireHost();
    return (...args) => invoke(id, ...args);
  }

  function installHost(host) {
    if (fixture.host !== undefined) {
      throw new Error("waterui: a host is already installed");
    }
    fixture.host = Array.isArray(host.modifiers)
      ? { ...host, modifiers: new Set(host.modifiers) }
      : host;
  }

  function uninstallHost() {
    fixture.host = undefined;
  }

  // Like the library's `mount`: the environment the host offers is
  // materialized through `toSignal` before the tree runs, and released when
  // the mount is disposed. The library publishes the two on the root owner
  // for `useContext`; the fixture has no owner tree, so it parks them where
  // `useTheme` / `useLocale` below read them.
  function mount(render) {
    const host = requireHost();
    const environment = host.environment();
    const previous = cleanups;
    const mountCleanups = [];
    cleanups = mountCleanups;
    try {
      fixture.theme = toSignal(environment.theme);
      fixture.locale = toSignal(environment.locale);
      const handle = render();
      return {
        handle,
        dispose: () => {
          for (const dispose of mountCleanups) {
            dispose();
          }
          fixture.theme = undefined;
          fixture.locale = undefined;
        },
      };
    } finally {
      cleanups = previous;
    }
  }

  function mountedContext(signal, name) {
    if (signal === undefined) {
      throw new Error(`${name}() is only available inside a mounted WaterUI tree`);
    }
    return signal;
  }

  const useTheme = () => mountedContext(fixture.theme, "useTheme");
  const useLocale = () => mountedContext(fixture.locale, "useLocale");

  // The values the tests bind to. `counter` is a writable signal, `label` is a
  // host-shaped accessor, `doubled` is a function accessor that pushes, and
  // `readOnce` is readable but announces nothing, so the bridge treats it as a
  // constant.
  fixture.counter = createSignal(1);
  fixture.labelValue = "start";
  fixture.labelSubscribers = new Set();
  fixture.label = {
    read: () => fixture.labelValue,
    subscribe(callback) {
      fixture.labelSubscribers.add(callback);
      fixture.subscribes += 1;
      return () => {
        fixture.labelSubscribers.delete(callback);
        fixture.disposes += 1;
      };
    },
  };
  fixture.pushLabel = (value) => {
    fixture.labelValue = value;
    for (const subscriber of [...fixture.labelSubscribers]) {
      subscriber(value);
    }
  };
  fixture.doubled = () => fixture.counter() * 2;
  fixture.doubled.__subscribe = (callback) =>
    fixture.counter.__subscribe((value) => callback(value * 2));
  fixture.readOnce = { read: () => 41 };
  // A function the tests push through `write`, to prove a value that happens
  // to be callable is stored and not called.
  fixture.probeCalls = 0;
  fixture.probe = (...args) => {
    fixture.probeCalls += 1;
    fixture.probeArgs = args;
    return "called";
  };
  fixture.hold = (value) => {
    fixture.held = value;
  };
  // The mounted environment, reachable the way `useTheme` / `useLocale` are
  // in the library. They are not runtime-global entries there either: a
  // module imports them from the library, and the bridge never calls them.
  fixture.useTheme = useTheme;
  fixture.useLocale = useLocale;

  globalThis.fixture = fixture;
  globalThis.__waterui_runtime = {
    installHost,
    uninstallHost,
    mount,
    isSignal,
    isAccessor,
    read,
    write,
    subscribe,
    toSignal,
    toAccessor,
    createSignal,
    createMemo,
    makeCallback,
    modules: {
      "src/promo.tsx": (props) => props,
    },
  };
})();
