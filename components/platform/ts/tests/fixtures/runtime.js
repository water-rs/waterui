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
// as they do in `signals.js`; and `makeCallback` dispatches through the
// installed host's `invoke`, not a global read per call. `fixture-parity` in
// the bun suite runs the same scenarios against this script and the real
// library and asserts they observe the same thing.
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

  function createSignal(initial) {
    fixture.signals += 1;
    let value = initial;
    const subscribers = new Set();
    const signal = () => value;
    signal.__signal = true;
    // Like the real `set`: a function argument is an updater, called with the
    // current value, and the write is deduplicated on SameValue.
    signal.set = (next) => {
      fixture.sets += 1;
      const resolved = typeof next === "function" ? next(value) : next;
      if (Object.is(value, resolved)) {
        return;
      }
      value = resolved;
      for (const subscriber of [...subscribers]) {
        subscriber(value);
      }
    };
    signal.__equals = Object.is;
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

  // The bridge only ever calls `createMemo(signal)`, to hand JavaScript a
  // read-only view of a signal it pushes into, so the fake memo forwards both
  // the read and the subscription to what it was given.
  function createMemo(compute) {
    const memo = () => compute();
    memo.__memo = true;
    memo.__subscribe = (callback) => compute.__subscribe(callback);
    return memo;
  }

  const isSignal = (value) => typeof value === "function" && value.__signal === true;

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
  // comparator the target itself settles on.
  const comparatorOf = (source) => source?.__equals ?? Object.is;

  function write(target, value) {
    fixture.writes += 1;
    if (isSignal(target)) {
      target.set(() => value);
      return comparatorOf(target)(read(target), value);
    }
    if (target !== null && typeof target === "object" && typeof target.write === "function") {
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
    if (isSignal(source)) {
      return source;
    }
    const signal = createSignal(read(source));
    if (isAccessor(source)) {
      subscribe(source, (value) => signal.set(() => value));
    }
    return signal;
  }

  // Through the installed host's `invoke`, captured once, exactly as the real
  // `makeCallback` does: reading the mutable global per call would let a
  // bundle divert every callback in the runtime.
  function makeCallback(id) {
    if (fixture.host === undefined) {
      throw new Error("waterui: no host installed — makeCallback needs its invoke");
    }
    const { invoke } = fixture.host;
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

  function mount(render) {
    return { handle: render(), dispose: () => {} };
  }

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
