// GENERATED — do not edit. Rebuild with:
//     bun run components/platform/ts/scripts/build-test-library.mjs
// from the repository root, or run the script through
// scripts/build-test-library.sh. tests/js/library.test.js fails when this
// file is not what the current sources produce.
//
// One whole application bundle: the WaterUI JavaScript library, the module
// tests/fixtures/mount.entry.js declares, and the installRuntimeGlobal call
// that publishes it — the shape the CLI's bundler produces for a real app.
(() => {
  // src/js/signals.js
  var CLEAN = 0;
  var CHECK = 1;
  var DIRTY = 2;
  var SIGNAL = Symbol("waterui.signal");
  var MEMO = Symbol("waterui.memo");
  var Listener = null;
  var Owner = null;
  var batchDepth = 0;
  var flushing = false;
  var evaluatingDepth = 0;
  var effectQueue = [];
  var MAX_FLUSH_EVALUATIONS = 1e4;
  var sameValue = (a, b) => Object.is(a, b);
  var neverEquals = () => false;
  var EQUALS = Symbol("waterui.equals");
  function comparatorOf(source) {
    return source?.[EQUALS] ?? sameValue;
  }
  var MAX_BRIDGE_DEPTH = 128;
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
      if (a.length !== b.length) {
        return false;
      }
      for (let index = 0;index < a.length; index += 1) {
        const present = index in a;
        if (present !== index in b) {
          return false;
        }
        if (present && !equalAtDepth(a[index], b[index], depth + 1)) {
          return false;
        }
      }
      return true;
    }
    if (isPlainObject(a) && isPlainObject(b)) {
      const keys = Object.keys(a);
      const otherKeys = Object.keys(b);
      if (keys.length !== otherKeys.length) {
        return false;
      }
      for (const [index, key] of keys.entries()) {
        if (key !== otherKeys[index] || !equalAtDepth(a[key], b[key], depth + 1)) {
          return false;
        }
      }
      return true;
    }
    return false;
  }
  function bridgeEquals(a, b) {
    return equalAtDepth(a, b, 0);
  }
  function equalsOf(options) {
    const equals = options?.equals;
    if (equals === undefined) {
      return sameValue;
    }
    if (equals === false) {
      return neverEquals;
    }
    if (typeof equals === "function") {
      return equals;
    }
    throw new TypeError('"equals" must be a comparison function or false');
  }
  function newSource(value, equals) {
    return { observers: new Set, value, equals, compute: null };
  }
  function newNode(owner) {
    const node = {
      owner,
      owned: new Set,
      cleanups: [],
      contexts: null,
      sources: new Set,
      observers: new Set,
      compute: null,
      isEffect: false,
      equals: sameValue,
      queued: false,
      disposed: false,
      evaluating: false,
      hasValue: false,
      state: CLEAN,
      value: undefined
    };
    if (owner !== null) {
      owner.owned.add(node);
    }
    return node;
  }
  function track(source) {
    if (Listener === null) {
      return;
    }
    Listener.sources.add(source);
    source.observers.add(Listener);
  }
  function markStale(node, state) {
    if (node.state >= state) {
      return;
    }
    node.state = state;
    if (node.isEffect && !node.queued) {
      node.queued = true;
      effectQueue.push(node);
    }
    for (const observer of node.observers) {
      markStale(observer, CHECK);
    }
  }
  function setSignalValue(node, value) {
    if (node.equals(node.value, value)) {
      return;
    }
    node.value = value;
    for (const observer of node.observers) {
      markStale(observer, DIRTY);
    }
    flushEffects();
  }
  function updateIfNecessary(node) {
    if (node.disposed) {
      node.state = CLEAN;
      return;
    }
    if (node.evaluating) {
      throw new Error("waterui: circular dependency in a computation");
    }
    if (node.state === CHECK) {
      for (const source of node.sources) {
        if (source.compute !== null) {
          updateIfNecessary(source);
        }
        if (node.state === DIRTY) {
          break;
        }
      }
    }
    if (node.state === DIRTY) {
      evaluate(node);
    }
    if (node.state === CHECK) {
      node.state = CLEAN;
    }
  }
  function flushEffects() {
    if (batchDepth > 0 || flushing || evaluatingDepth > 0) {
      return;
    }
    flushing = true;
    let runs = 0;
    try {
      while (effectQueue.length > 0) {
        if (runs >= MAX_FLUSH_EVALUATIONS) {
          effectQueue.length = 0;
          throw new Error(`waterui: effects did not settle within ${MAX_FLUSH_EVALUATIONS} evaluations — a reactive cycle keeps re-scheduling work`);
        }
        runs += 1;
        const effect = effectQueue.shift();
        effect.queued = false;
        updateIfNecessary(effect);
      }
    } finally {
      flushing = false;
    }
  }
  function runCleanups(node) {
    const cleanups = node.cleanups;
    node.cleanups = [];
    for (const cleanup of cleanups) {
      cleanup();
    }
  }
  function evaluate(node) {
    if (node.evaluating) {
      throw new Error("waterui: circular dependency in a computation");
    }
    for (const child of node.owned) {
      disposeNode(child);
    }
    node.owned.clear();
    runCleanups(node);
    for (const source of node.sources) {
      source.observers.delete(node);
    }
    node.sources.clear();
    const prevListener = Listener;
    const prevOwner = Owner;
    Listener = node;
    Owner = node;
    const previous = node.value;
    let next;
    node.evaluating = true;
    evaluatingDepth += 1;
    node.state = CLEAN;
    try {
      next = node.compute(previous);
    } catch (error) {
      node.state = DIRTY;
      throw error;
    } finally {
      node.evaluating = false;
      evaluatingDepth -= 1;
      Listener = prevListener;
      Owner = prevOwner;
      if (evaluatingDepth === 0) {
        flushEffects();
      }
    }
    if (node.isEffect) {
      node.value = next;
      return;
    }
    if (node.hasValue && node.equals(previous, next)) {
      return;
    }
    node.hasValue = true;
    node.value = next;
    for (const observer of node.observers) {
      markStale(observer, DIRTY);
    }
  }
  function disposeNode(node) {
    if (node.disposed) {
      return;
    }
    node.disposed = true;
    for (const child of node.owned) {
      disposeNode(child);
    }
    node.owned.clear();
    runCleanups(node);
    for (const source of node.sources) {
      source.observers.delete(node);
    }
    node.sources.clear();
    if (node.owner !== null) {
      node.owner.owned.delete(node);
    }
  }
  function readTracked(source) {
    if (source.compute !== null) {
      updateIfNecessary(source);
    }
    track(source);
    return source.value;
  }
  function createSignal(initial, options) {
    const node = newSource(initial, equalsOf(options));
    const signal = function() {
      return readTracked(node);
    };
    signal.set = (value) => {
      setSignalValue(node, typeof value === "function" ? value(node.value) : value);
    };
    signal.update = (f) => {
      setSignalValue(node, f(node.value));
    };
    signal.map = (f) => createMemo(() => f(signal()));
    signal.peek = () => node.value;
    signal[SIGNAL] = true;
    signal[EQUALS] = node.equals;
    signal[Symbol.iterator] = function* () {
      yield signal;
      yield (value) => signal.set(value);
    };
    return signal;
  }
  function createMemo(compute, options) {
    const node = newNode(Owner);
    node.compute = compute;
    node.equals = equalsOf(options);
    evaluate(node);
    const memo = function() {
      return readTracked(node);
    };
    memo.map = (f) => createMemo(() => f(memo()));
    memo.peek = () => {
      updateIfNecessary(node);
      return node.value;
    };
    memo[MEMO] = true;
    memo[EQUALS] = node.equals;
    return memo;
  }
  function createEffect(fn) {
    const node = newNode(Owner);
    node.compute = fn;
    node.isEffect = true;
    evaluate(node);
  }
  function createRoot(fn) {
    const root = newNode(null);
    const prevOwner = Owner;
    const prevListener = Listener;
    Owner = root;
    Listener = null;
    try {
      return fn(() => disposeNode(root));
    } finally {
      Owner = prevOwner;
      Listener = prevListener;
    }
  }
  function createScope(fn) {
    const scope = newNode(Owner);
    const prevOwner = Owner;
    const prevListener = Listener;
    Owner = scope;
    Listener = null;
    try {
      return fn(() => disposeNode(scope));
    } finally {
      Owner = prevOwner;
      Listener = prevListener;
    }
  }
  function onCleanup(fn) {
    if (Owner === null) {
      throw new Error("onCleanup() requires an active reactive owner");
    }
    Owner.cleanups.push(fn);
  }
  function getOwner() {
    return Owner;
  }
  function untrack(fn) {
    const prevListener = Listener;
    Listener = null;
    try {
      return fn();
    } finally {
      Listener = prevListener;
    }
  }
  function isSignal(value) {
    return typeof value === "function" && value[SIGNAL] === true;
  }
  function isMemo(value) {
    return typeof value === "function" && value[MEMO] === true;
  }
  function isAccessor(value) {
    return typeof value === "function" || value !== null && typeof value === "object" && typeof value.read === "function";
  }
  var STORE_NODE = Symbol("waterui.store-node");
  var storeNodes = new WeakMap;
  function createContext(defaultValue) {
    const id = Symbol("waterui.context");
    return {
      id,
      defaultValue,
      Provider(props) {
        if (Owner === null) {
          throw new Error("<Context.Provider> requires an active reactive owner");
        }
        return createScope(() => {
          const scope = Owner;
          scope.contexts = new Map([[id, props.value]]);
          const children = props.children;
          return typeof children === "function" && !isSignal(children) && !isMemo(children) ? children() : children;
        });
      }
    };
  }

  // src/js/host.js
  var installedHost = null;
  function installHost(host) {
    if (installedHost !== null) {
      throw new Error("waterui: a host is already installed");
    }
    for (const name of [
      "create",
      "modify",
      "text",
      "show",
      "each",
      "suspense",
      "environment",
      "invoke"
    ]) {
      if (typeof host[name] !== "function") {
        throw new TypeError(`waterui host is missing the "${name}" entry — see HOST.md`);
      }
    }
    if (Array.isArray(host.modifiers)) {
      installedHost = { ...host, modifiers: new Set(host.modifiers) };
      return;
    }
    if (host.modifiers === null || typeof host.modifiers !== "object" || typeof host.modifiers.has !== "function") {
      throw new TypeError(`waterui host is missing the "modifiers" entry — the catalog's modifier names as an array or a ReadonlySet<string>, see HOST.md`);
    }
    installedHost = host;
  }
  function uninstallHost() {
    installedHost = null;
  }
  function getHost() {
    if (installedHost === null) {
      throw new Error("waterui: no host installed — installHost() must run before a module is mounted");
    }
    return installedHost;
  }
  function trackedRead(source) {
    return typeof source === "function" ? source() : source.read();
  }
  function read(value) {
    if (typeof value === "function" || value !== null && typeof value === "object" && typeof value.read === "function") {
      return untrack(() => trackedRead(value));
    }
    return value;
  }
  function write(target, value, identity = false) {
    const alreadyHolds = (held) => identity ? Object.is(held, value) : bridgeEquals(held, value);
    if (isSignal(target)) {
      if (alreadyHolds(read(target))) {
        return true;
      }
      target.set(() => value);
      return comparatorOf(target)(read(target), value);
    }
    if (target !== null && typeof target === "object" && typeof target.write === "function") {
      if (alreadyHolds(read(target))) {
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
    if (typeof source !== "function") {
      throw new TypeError("subscribe() expects a signal, an accessor, or a { read, subscribe } value");
    }
    return createRoot((dispose) => {
      let first = true;
      createEffect(() => {
        const value = source();
        if (first) {
          first = false;
          return;
        }
        callback(value);
      });
      return dispose;
    });
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
        onCleanup(source.subscribe((value) => signal.set(() => value)));
      }
      return signal;
    }
    if (typeof source === "function") {
      return createMemo(source);
    }
    return createSignal(source);
  }

  // src/js/jsx-runtime.js
  var CONTAINS_SPREAD = Symbol("waterui.contains-spread");
  function hasSpread(props) {
    return props !== null && typeof props === "object" && props[CONTAINS_SPREAD] === true;
  }
  function reactiveInput(props, name) {
    const descriptor = Object.getOwnPropertyDescriptor(props, name);
    if (descriptor === undefined) {
      return;
    }
    return descriptor.get === undefined ? descriptor.value : () => descriptor.get.call(props);
  }
  function normalizeChildren(children, into) {
    if (children === undefined || children === null || typeof children === "boolean") {
      return into;
    }
    if (Array.isArray(children)) {
      for (const child of children) {
        normalizeChildren(child, into);
      }
      return into;
    }
    into.push(children);
    return into;
  }
  function jsx(type, props) {
    if (typeof type === "function") {
      return untrack(() => type(props ?? {}));
    }
    if (typeof type !== "string") {
      throw new TypeError(`waterui: <${String(type)}> is not a component`);
    }
    const host = getHost();
    const config = {};
    const modifiers = [];
    let children;
    if (props !== null && props !== undefined) {
      const spread = hasSpread(props);
      for (const name of Object.keys(props)) {
        if (name === "children") {
          children = reactiveInput(props, name);
          continue;
        }
        if (host.modifiers.has(name)) {
          if (spread) {
            throw new Error(`<${type}> received modifier attribute "${name}" through a spread. ` + "Modifier attributes apply in written order, so they must be written literally on the element.");
          }
          modifiers.push([name, reactiveInput(props, name)]);
          continue;
        }
        config[name] = reactiveInput(props, name);
      }
    }
    let handle = host.create(type, config, normalizeChildren(children, []));
    for (const [name, value] of modifiers) {
      handle = host.modify(handle, name, value);
    }
    return handle;
  }

  // src/js/contexts.js
  var ThemeContext = createContext();
  var LocaleContext = createContext();
  function mount(render) {
    const host = getHost();
    const environment = host.environment();
    return createRoot((dispose) => {
      const owner = getOwner();
      const contexts = new Map;
      contexts.set(ThemeContext.id, toSignal(environment.theme));
      contexts.set(LocaleContext.id, toSignal(environment.locale));
      owner.contexts = contexts;
      return { handle: render(), dispose };
    });
  }

  // src/js/runtime-global.js
  var RUNTIME_GLOBAL = "__waterui_runtime";
  function makeCallback(id) {
    if (typeof id !== "number") {
      throw new TypeError("makeCallback(id) expects the numeric id the bridge assigned");
    }
    const { invoke } = getHost();
    return (...args) => invoke(id, ...args);
  }
  function installRuntimeGlobal(modules, contracts = {}) {
    if (modules === null || typeof modules !== "object") {
      throw new TypeError("installRuntimeGlobal(modules) expects an object mapping each module id to its default export");
    }
    if (contracts === null || typeof contracts !== "object") {
      throw new TypeError("installRuntimeGlobal(modules, contracts) expects an object mapping each module id to the hexadecimal props contract hash it was built against");
    }
    const runtime = Object.freeze({
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
      modules,
      contracts
    });
    globalThis[RUNTIME_GLOBAL] = runtime;
    return runtime;
  }

  // tests/fixtures/mount.entry.js
  var MODULE = "tests/fixtures/promo.tsx";
  function Promo(props) {
    globalThis.__waterui_test_disposals = 0;
    onCleanup(() => {
      globalThis.__waterui_test_disposals += 1;
    });
    return jsx("VStack", {
      spacing: 8,
      children: [
        jsx("Text", { children: props.headline }),
        jsx("Text", { children: () => `${props.unread()} unread` }),
        jsx("Button", { onTap: props.onDismiss, children: "Dismiss" })
      ]
    });
  }
  installRuntimeGlobal({ [MODULE]: Promo }, globalThis.__waterui_test_contracts);
})();
