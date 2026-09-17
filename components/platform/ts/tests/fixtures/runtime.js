// A stand-in for the runtime global a real bundle publishes.
//
// Rust tests cannot bundle — the transform and the bundler are the CLI's, and
// they are not built here — so this classic script installs the same
// `globalThis.__waterui_runtime` shape over a tiny push-based signal
// implementation. It is deliberately minimal: what the bridge calls is
// `read`, `write`, `subscribe`, `isSignal`, `createSignal`, `createMemo` and
// `makeCallback`, and each one behaves exactly as HOST.md describes.
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
    subscribes: 0,
    disposes: 0,
    // A slot the tests park a value in, so both sides can reach it.
    held: undefined,
  };

  function createSignal(initial) {
    let value = initial;
    const subscribers = new Set();
    const signal = () => value;
    signal.__signal = true;
    signal.set = (next) => {
      value = next;
      fixture.sets += 1;
      for (const subscriber of [...subscribers]) {
        subscriber(value);
      }
    };
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

  function write(target, value) {
    fixture.writes += 1;
    if (isSignal(target)) {
      target.set(value);
      return;
    }
    if (target !== null && typeof target === "object" && typeof target.write === "function") {
      target.write(value);
      return;
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
      subscribe(source, (value) => signal.set(value));
    }
    return signal;
  }

  function makeCallback(id) {
    return (...args) => globalThis.__waterui_host.invoke(id, ...args);
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
