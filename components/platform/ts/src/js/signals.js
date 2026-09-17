// The reactive core of the WaterUI TypeScript runtime.
//
// A self-contained signal library with Solid semantics: automatic dependency
// tracking, glitch-free propagation (a diamond dependency evaluates its sink
// once per change), and explicit ownership instead of hook-style positional
// state. Every primitive is owned by the current reactive owner — the
// computation or scope active when it is created — never by call order.
//
// Propagation is mark-then-pull. A write marks its direct observers DIRTY and
// everything downstream of them CHECK, then effects are drained from a queue.
// A CHECK node pulls its sources: each source recomputes only if dirty, and a
// memo that recomputes to an equal value leaves its observers untouched, so a
// sink that saw two changed inputs still runs exactly once.
//
// Signal states: CLEAN (value current), CHECK (a source may have changed),
// DIRTY (a source definitely changed). Signals themselves have no state —
// a write is always a real change once `equals` has accepted it.

const CLEAN = 0;
const CHECK = 1;
const DIRTY = 2;

const SIGNAL = Symbol("waterui.signal");
const MEMO = Symbol("waterui.memo");

// The computation currently collecting dependencies, and the node that owns
// whatever is being created. They differ while a scope runs code that must
// not track (`runWithOwner` of a scope, component invocation).
let Listener = null;
let Owner = null;

let batchDepth = 0;
let flushing = false;
// How many computations are mid-`evaluate`. Writes inside an evaluation must
// not flush: the node being evaluated is in the queue and cannot re-run until
// its own run finishes, so draining happens when the outermost evaluation
// ends.
let evaluatingDepth = 0;
const effectQueue = [];

// A flush that keeps finding work past this point is a reactive cycle —
// effects writing each other's sources forever. Legitimate propagation runs
// each queued effect once, so the bound is generous, not tight.
const MAX_FLUSH_EVALUATIONS = 10000;

const referenceEquals = (a, b) => a === b;
const neverEquals = () => false;

function equalsOf(options) {
  const equals = options?.equals;
  if (equals === undefined) {
    return referenceEquals;
  }
  if (equals === false) {
    return neverEquals;
  }
  if (typeof equals === "function") {
    return equals;
  }
  throw new TypeError('"equals" must be a comparison function or false');
}

// A readable source: signals and memos both take this shape. `compute` is set
// on memos — the same object plays computation and source.
function newSource(value, equals) {
  return { observers: new Set(), value, equals, compute: null };
}

// An owner: scopes and computations. `sources`/`state` are only meaningful
// when `compute` is set, but keeping the shape uniform keeps every node
// markable and disposable the same way.
function newNode(owner) {
  const node = {
    owner,
    owned: new Set(),
    cleanups: [],
    contexts: null,
    sources: new Set(),
    observers: new Set(),
    compute: null,
    isEffect: false,
    equals: referenceEquals,
    queued: false,
    disposed: false,
    evaluating: false,
    hasValue: false,
    state: CLEAN,
    value: undefined,
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

// The resolved write. `value` is final — the public `set` unwraps updater
// functions before calling this, and internal writers (stores) must never
// have a stored function mistaken for an updater.
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
    // The only way to reach a mid-evaluation node through a pull is a true
    // dependency cycle — its value is being recomputed right now.
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
  // `evaluate` leaves the node CLEAN unless it re-marked itself mid-run —
  // a write to one of its own sources. That mark is real: the node is queued
  // again and will re-run from the flush, so only settle a node still in
  // CHECK here.
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
    // A write inside an effect can queue more effects; drain until empty.
    while (effectQueue.length > 0) {
      if (runs >= MAX_FLUSH_EVALUATIONS) {
        effectQueue.length = 0;
        throw new Error(
          `waterui: effects did not settle within ${MAX_FLUSH_EVALUATIONS} evaluations — a reactive cycle keeps re-scheduling work`,
        );
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

// Re-runs a computation. Owned children and cleanups from the previous run
// are torn down first — children before the node's own cleanups — then
// sources are detached and the function re-tracks from scratch.
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
  // Present as clean while running: a write to one of this node's own sources
  // re-marks it DIRTY and queues it, and the mark survives — it converges on
  // the re-run rather than being swallowed by the caller's settle.
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
  // A memo that recomputes to an equal value does not disturb its observers:
  // that is what makes a diamond evaluate its sink once.
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

/**
 * Creates a signal: a callable reactive value.
 *
 * `signal()` reads and tracks; `signal.set(v)` writes (a function argument is
 * an updater — wrap a function value as `set(() => f)` to store it);
 * `signal.update(f)` writes `f(previous)`; `signal.map(f)` derives a memo;
 * iterating the signal yields `[read, write]` for destructuring.
 */
export function createSignal(initial, options) {
  const node = newSource(initial, equalsOf(options));
  const signal = function () {
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
  signal[Symbol.iterator] = function* () {
    yield signal;
    yield (value) => signal.set(value);
  };
  return signal;
}

/**
 * Creates a memo: a cached derived value. Evaluates eagerly, then recomputes
 * only when a source changed and only invalidates its observers when the
 * result actually differs per `equals`.
 */
export function createMemo(compute, options) {
  const node = newNode(Owner);
  node.compute = compute;
  node.equals = equalsOf(options);
  evaluate(node);
  const memo = function () {
    return readTracked(node);
  };
  memo.map = (f) => createMemo(() => f(memo()));
  memo.peek = () => {
    updateIfNecessary(node);
    return node.value;
  };
  memo[MEMO] = true;
  return memo;
}

/**
 * Creates an effect: a computation that runs for its side effects. Runs
 * synchronously at creation, then once per settled change to its sources.
 */
export function createEffect(fn) {
  const node = newNode(Owner);
  node.compute = fn;
  node.isEffect = true;
  evaluate(node);
}

/**
 * Runs `fn` under a fresh detached owner and returns its result. `fn` receives
 * the scope's `dispose`, which tears down everything the scope owns.
 */
export function createRoot(fn) {
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

/**
 * Like `createRoot`, but the scope is owned by the current owner: it inherits
 * the owner's context chain and is disposed with it. Control-flow branches use
 * this so a branch created later still resolves contexts from the owner that
 * rendered it.
 */
export function createScope(fn) {
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

/** Registers a cleanup on the current owner, run on re-evaluation or dispose. */
export function onCleanup(fn) {
  if (Owner === null) {
    throw new Error("onCleanup() requires an active reactive owner");
  }
  Owner.cleanups.push(fn);
}

/** Returns the current reactive owner, or null outside any scope. */
export function getOwner() {
  return Owner;
}

/** Runs `fn` with `owner` as the current owner. */
export function runWithOwner(owner, fn) {
  const prevOwner = Owner;
  const prevListener = Listener;
  Owner = owner;
  // A scope has no computation of its own; reads inside it track into
  // whatever computation is running, not the scope.
  Listener = owner !== null && owner.compute !== null ? owner : null;
  try {
    return fn();
  } finally {
    Owner = prevOwner;
    Listener = prevListener;
  }
}

/** Runs `fn` without tracking any signal reads. */
export function untrack(fn) {
  const prevListener = Listener;
  Listener = null;
  try {
    return fn();
  } finally {
    Listener = prevListener;
  }
}

/** Coalesces all writes inside `fn` into a single propagation pass. */
export function batch(fn) {
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

/** True for values produced by `createSignal` (writable, callable). */
export function isSignal(value) {
  return typeof value === "function" && value[SIGNAL] === true;
}

/** True for values produced by `createMemo` (read-only, callable). */
export function isMemo(value) {
  return typeof value === "function" && value[MEMO] === true;
}

/**
 * True for anything readable as a reactive value: signals, memos, plain
 * thunks, and host-side `{ read() }` objects.
 */
export function isAccessor(value) {
  return (
    typeof value === "function" ||
    (value !== null && typeof value === "object" && typeof value.read === "function")
  );
}

// ---------------------------------------------------------------------------
// Stores
// ---------------------------------------------------------------------------

const STORE_NODE = Symbol("waterui.store-node");
const storeNodes = new WeakMap();

function isStorable(value) {
  return value !== null && typeof value === "object";
}

function nodeFor(raw) {
  let node = storeNodes.get(raw);
  if (node === undefined) {
    // `signals` tracks each property; `keys` tracks key-set membership so
    // iteration reacts to added and removed properties. `children` caches the
    // proxy handed out per property so `store.a === store.a` holds.
    node = { raw, signals: new Map(), keys: null, children: new Map() };
    storeNodes.set(raw, node);
  }
  return node;
}

function unwrapStore(value) {
  if (isStorable(value)) {
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
    signal = newSource(node.raw[key], referenceEquals);
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
  if (!isStorable(value)) {
    return value;
  }
  let child = node.children.get(key);
  if (child === undefined || child[STORE_NODE].raw !== value) {
    child = new Proxy(value, storeHandler);
    node.children.set(key, child);
  }
  return child;
}

const storeHandler = {
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
    // Truncating an array by `length` removes indices without going through
    // deleteProperty — notify every dropped index and the key set explicitly.
    if (Array.isArray(target) && key === "length") {
      const next = unwrapStore(value);
      const previous = target.length;
      for (let i = next; i < previous; i += 1) {
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
  },
};

// `setStore(path..., value)`. With a single function argument the function is
// a producer called with the store itself — mutations through it notify as
// ordinary sets — and a returned object replaces the root's contents.
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
  for (let i = 0; i < args.length - 2; i += 1) {
    target = target[args[i]];
    if (!isStorable(unwrapStore(target))) {
      throw new Error(
        `setStore() path segment "${String(args[i])}" does not resolve to a store object`,
      );
    }
  }
  const key = args[args.length - 2];
  const value = args[args.length - 1];
  if (typeof value !== "function") {
    target[key] = value;
    return;
  }
  const previous = target[key];
  if (isStorable(unwrapStore(previous))) {
    // An object leaf gets the proxy so the function can mutate it; a
    // non-undefined return replaces the leaf instead.
    const result = value(previous);
    if (result !== undefined) {
      target[key] = result;
    }
    return;
  }
  const next = value(previous);
  if (next === undefined) {
    throw new Error(
      `setStore() updater at "${String(key)}" returned undefined for a non-object value`,
    );
  }
  target[key] = next;
}

function replaceContents(proxy, source) {
  const raw = unwrapStore(source);
  const target = proxy[STORE_NODE].raw;
  if (!isStorable(raw) || Array.isArray(raw) !== Array.isArray(target)) {
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

/**
 * Creates a nested reactive store. Returns `[store, setStore]`.
 *
 * Every property is tracked independently: an effect reading `store.a` does
 * not re-run when `store.b` changes, and nested objects are wrapped lazily so
 * `setStore("a", "b", v)` only invalidates readers of that path. `setStore`
 * accepts either a path ending in a value or updater, or a single producer
 * that mutates the store directly.
 */
export function createStore(initial) {
  if (!isStorable(initial)) {
    throw new TypeError("createStore() expects an object or an array");
  }
  const store = new Proxy(initial, storeHandler);
  return [store, (...args) => applySetStore(store, args)];
}

// ---------------------------------------------------------------------------
// Contexts
// ---------------------------------------------------------------------------

/**
 * Creates a context. `useContext` resolves a context by walking the owner
 * chain, so a value provided by `<Context.Provider>` is visible to every
 * component the provider's subtree creates — across function-component
 * boundaries and inside control-flow branches — regardless of evaluation
 * order.
 */
export function createContext(defaultValue) {
  const id = Symbol("waterui.context");
  return {
    id,
    defaultValue,
    Provider(props) {
      if (Owner === null) {
        throw new Error("<Context.Provider> requires an active reactive owner");
      }
      // Each provider gets its own scope so nested providers of the same
      // context shadow rather than overwrite each other.
      return createScope(() => {
        const scope = Owner;
        scope.contexts = new Map([[id, props.value]]);
        const children = props.children;
        // Resolve an unevaluated child thunk now, under this scope, so the
        // provided value is visible wherever the children end up mounted.
        return typeof children === "function" && !isSignal(children) && !isMemo(children)
          ? children()
          : children;
      });
    },
  };
}

/** Resolves `context` from the nearest enclosing owner that provided it. */
export function useContext(context) {
  for (let owner = Owner; owner !== null; owner = owner.owner) {
    if (owner.contexts !== null && owner.contexts.has(context.id)) {
      return owner.contexts.get(context.id);
    }
  }
  return context.defaultValue;
}
