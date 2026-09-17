// GENERATED — do not edit. Rebuild with:
//     bun run components/platform/ts/scripts/build-test-library.mjs
// from the repository root, or run the script through
// scripts/build-test-library.sh. tests/js/library.test.js fails when this
// file is not what the current src/js produces.
//
// The WaterUI JavaScript library, flattened into one classic script and
// published on globalThis.waterui, which is how the Rust tests reach it: the
// engine evaluates a bundle as a script, so nothing an ES module declares
// would otherwise be reachable.
(() => {
  var __defProp = Object.defineProperty;
  var __export = (target, all) => {
    for (var name in all)
      __defProp(target, name, {
        get: all[name],
        enumerable: true,
        configurable: true,
        set: (newValue) => all[name] = () => newValue
      });
  };

  // src/js/index.js
  var exports_js = {};
  __export(exports_js, {
    write: () => write,
    useTheme: () => useTheme,
    useLocale: () => useLocale,
    useContext: () => useContext,
    untrack: () => untrack,
    uninstallHost: () => uninstallHost,
    toSignal: () => toSignal,
    toAccessor: () => toAccessor,
    subscribe: () => subscribe,
    spreadProps: () => spreadProps,
    runWithOwner: () => runWithOwner,
    read: () => read,
    onCleanup: () => onCleanup,
    mount: () => mount,
    makeCallback: () => makeCallback,
    jsxs: () => jsxs,
    jsxDEV: () => jsxDEV,
    jsx: () => jsx,
    isSignal: () => isSignal,
    isMemo: () => isMemo,
    isAccessor: () => isAccessor,
    installRuntimeGlobal: () => installRuntimeGlobal,
    installHost: () => installHost,
    getOwner: () => getOwner,
    getHost: () => getHost,
    createStore: () => createStore,
    createSignal: () => createSignal,
    createScope: () => createScope,
    createRoot: () => createRoot,
    createMemo: () => createMemo,
    createEffect: () => createEffect,
    createContext: () => createContext,
    batch: () => batch,
    Suspense: () => Suspense,
    Show: () => Show,
    Fragment: () => Fragment,
    For: () => For,
    Box: () => Box
  });

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
  function runWithOwner(owner, fn) {
    const prevOwner = Owner;
    const prevListener = Listener;
    Owner = owner;
    Listener = owner !== null && owner.compute !== null ? owner : null;
    try {
      return fn();
    } finally {
      Owner = prevOwner;
      Listener = prevListener;
    }
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
  function batch(fn) {
    batchDepth += 1;
    try {
      return fn();
    } finally {
      batchDepth -= 1;
      if (batchDepth === 0) {
        flushEffects();
      }
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
  function isStoreable(value) {
    return value !== null && typeof value === "object";
  }
  function nodeFor(raw) {
    let node = storeNodes.get(raw);
    if (node === undefined) {
      node = { raw, signals: new Map, keys: null, children: new Map };
      storeNodes.set(raw, node);
    }
    return node;
  }
  function unwrapStore(value) {
    if (isStoreable(value)) {
      const node = value[STORE_NODE];
      if (node !== undefined) {
        return node.raw;
      }
    }
    return value;
  }
  function trackProp(node, key) {
    let signal = node.signals.get(key);
    if (signal === undefined) {
      signal = newSource(node.raw[key], sameValue);
      node.signals.set(key, signal);
    }
    track(signal);
  }
  function notifyProp(node, key) {
    const signal = node.signals.get(key);
    if (signal !== undefined) {
      setSignalValue(signal, node.raw[key]);
    }
  }
  function bumpKeys(node) {
    if (node.keys !== null) {
      setSignalValue(node.keys, node.keys.value + 1);
    }
  }
  function wrapChild(node, key, value) {
    if (!isStoreable(value)) {
      return value;
    }
    let child = node.children.get(key);
    if (child === undefined || child[STORE_NODE].raw !== value) {
      child = new Proxy(value, storeHandler);
      node.children.set(key, child);
    }
    return child;
  }
  var storeHandler = {
    get(target, key) {
      if (key === STORE_NODE) {
        return nodeFor(target);
      }
      const node = nodeFor(target);
      const value = target[key];
      trackProp(node, key);
      return wrapChild(node, key, value);
    },
    has(target, key) {
      trackProp(nodeFor(target), key);
      return Reflect.has(target, key);
    },
    ownKeys(target) {
      const node = nodeFor(target);
      if (node.keys === null) {
        node.keys = newSource(0, neverEquals);
      }
      track(node.keys);
      return Reflect.ownKeys(target);
    },
    set(target, key, value) {
      const node = nodeFor(target);
      if (Array.isArray(target) && key === "length") {
        const next = unwrapStore(value);
        const previous = target.length;
        for (let i = next;i < previous; i += 1) {
          const indexKey = String(i);
          if (indexKey in target) {
            delete target[indexKey];
            node.children.delete(indexKey);
            notifyProp(node, indexKey);
          }
        }
        target.length = next;
        bumpKeys(node);
        notifyProp(node, "length");
        return true;
      }
      const raw = unwrapStore(value);
      const had = Object.prototype.hasOwnProperty.call(target, key);
      target[key] = raw;
      if (!had) {
        bumpKeys(node);
      }
      notifyProp(node, key);
      return true;
    },
    deleteProperty(target, key) {
      if (!Object.prototype.hasOwnProperty.call(target, key)) {
        return true;
      }
      delete target[key];
      const node = nodeFor(target);
      bumpKeys(node);
      notifyProp(node, key);
      return true;
    }
  };
  function applySetStore(root, args) {
    if (args.length === 0) {
      throw new Error("setStore() requires a value or a producer function");
    }
    if (args.length === 1) {
      const update = args[0];
      if (typeof update === "function") {
        const result = update(root);
        if (result !== undefined && result !== root) {
          replaceContents(root, result);
        }
        return;
      }
      replaceContents(root, update);
      return;
    }
    let target = root;
    for (let i = 0;i < args.length - 2; i += 1) {
      target = target[args[i]];
      if (!isStoreable(unwrapStore(target))) {
        throw new Error(`setStore() path segment "${String(args[i])}" does not resolve to a store object`);
      }
    }
    const key = args[args.length - 2];
    const value = args[args.length - 1];
    if (typeof value !== "function") {
      target[key] = value;
      return;
    }
    const previous = target[key];
    if (isStoreable(unwrapStore(previous))) {
      const result = value(previous);
      if (result !== undefined) {
        target[key] = result;
      }
      return;
    }
    const next = value(previous);
    if (next === undefined) {
      throw new Error(`setStore() updater at "${String(key)}" returned undefined for a non-object value`);
    }
    target[key] = next;
  }
  function replaceContents(proxy, source) {
    const raw = unwrapStore(source);
    const target = proxy[STORE_NODE].raw;
    if (!isStoreable(raw) || Array.isArray(raw) !== Array.isArray(target)) {
      throw new TypeError("setStore() replacement must be an object of the same kind");
    }
    for (const key of Object.keys(target)) {
      if (!(key in raw)) {
        delete proxy[key];
      }
    }
    if (Array.isArray(target)) {
      proxy.length = raw.length;
    }
    for (const key of Object.keys(raw)) {
      proxy[key] = raw[key];
    }
  }
  function createStore(initial) {
    if (!isStoreable(initial)) {
      throw new TypeError("createStore() expects an object or an array");
    }
    const store = new Proxy(initial, storeHandler);
    return [store, (...args) => applySetStore(store, args)];
  }
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
  function useContext(context) {
    for (let owner = Owner;owner !== null; owner = owner.owner) {
      if (owner.contexts !== null && owner.contexts.has(context.id)) {
        return owner.contexts.get(context.id);
      }
    }
    return context.defaultValue;
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
  function write(target, value) {
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
  function spreadProps(props) {
    Object.defineProperty(props, CONTAINS_SPREAD, { value: true });
    return props;
  }
  function hasSpread(props) {
    return props !== null && typeof props === "object" && props[CONTAINS_SPREAD] === true;
  }
  function propValue(props, name) {
    const descriptor = Object.getOwnPropertyDescriptor(props, name);
    if (descriptor === undefined) {
      return;
    }
    return descriptor.get === undefined ? descriptor.value : descriptor.get.call(props);
  }
  function lazyProp(props, name) {
    return () => propValue(props, name);
  }
  function reactiveInput(props, name) {
    const descriptor = Object.getOwnPropertyDescriptor(props, name);
    if (descriptor === undefined) {
      return;
    }
    return descriptor.get === undefined ? descriptor.value : () => descriptor.get.call(props);
  }
  function Fragment(props) {
    return props.children;
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
  var jsxs = jsx;
  function jsxDEV(type, props) {
    return jsx(type, props);
  }

  // src/js/components.js
  function branch(render) {
    const owner = getOwner();
    return (...args) => runWithOwner(owner, () => createScope((dispose) => ({ handle: render(...args), dispose })));
  }
  function resolveElement(value) {
    return typeof value === "function" && !isSignal(value) && !isMemo(value) ? value() : value;
  }
  function isFunctionValue(value) {
    return typeof value === "function" && !isSignal(value) && !isMemo(value);
  }
  function Show(props) {
    const host = getHost();
    const children = propValue(props, "children");
    const render = isFunctionValue(children) ? branch((item) => children(item)) : branch(() => resolveElement(lazyProp(props, "children")()));
    const fallback = propValue(props, "fallback") === undefined ? undefined : branch(() => resolveElement(lazyProp(props, "fallback")()));
    return host.show(reactiveInput(props, "when"), render, fallback);
  }
  function For(props) {
    const host = getHost();
    const renderFn = propValue(props, "children");
    if (!isFunctionValue(renderFn)) {
      throw new TypeError("<For> requires a function child: <For each={items}>{(item, index) => …}</For>");
    }
    return host.each(reactiveInput(props, "each"), branch((item, index) => renderFn(item, index)), propValue(props, "by"));
  }
  function Suspense(props) {
    const host = getHost();
    const fallback = propValue(props, "fallback") === undefined ? undefined : branch(() => resolveElement(lazyProp(props, "fallback")()));
    return host.suspense(branch(() => resolveElement(lazyProp(props, "children")())), fallback);
  }
  function Box(props) {
    const host = getHost();
    const child = resolveElement(propValue(props, "children"));
    if (child === undefined || child === null) {
      throw new Error("<Box> requires exactly one child element, got none");
    }
    if (Array.isArray(child)) {
      throw new Error(`<Box> accepts exactly one child element, got ${child.length}`);
    }
    if (typeof child !== "object") {
      throw new TypeError(`<Box> requires a host element as its child, got ${typeof child === "function" ? "a reactive child" : JSON.stringify(child)}`);
    }
    const spread = hasSpread(props);
    let handle = child;
    for (const name of Object.keys(props)) {
      if (name === "children") {
        continue;
      }
      if (!host.modifiers.has(name)) {
        throw new Error(`<Box> takes only modifier attributes and a single child; got "${name}"`);
      }
      if (spread) {
        throw new Error(`<Box> received modifier attribute "${name}" through a spread. ` + "Modifier attributes apply in written order, so they must be written literally on the element.");
      }
      handle = host.modify(handle, name, reactiveInput(props, name));
    }
    return handle;
  }
  // src/js/contexts.js
  var ThemeContext = createContext();
  var LocaleContext = createContext();
  function required(context, name) {
    const value = useContext(context);
    if (value === undefined) {
      throw new Error(`${name}() is only available inside a mounted WaterUI tree`);
    }
    return value;
  }
  function useTheme() {
    return required(ThemeContext, "useTheme");
  }
  function useLocale() {
    return required(LocaleContext, "useLocale");
  }
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
  function installRuntimeGlobal(modules) {
    if (modules === null || typeof modules !== "object") {
      throw new TypeError("installRuntimeGlobal(modules) expects an object mapping each module id to its default export");
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
      modules
    });
    globalThis[RUNTIME_GLOBAL] = runtime;
    return runtime;
  }
  // tests/fixtures/library.entry.js
  globalThis.waterui = exports_js;
})();
