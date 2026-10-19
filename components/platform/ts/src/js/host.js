// The seam between the JS runtime and the native side.
//
// Everything the runtime needs from Rust — creating views, applying modifiers,
// control flow, the environment — goes through the object installed by
// `installHost`. The bridge installs it right after evaluating the bundle and
// before any module is mounted; the contract it must satisfy is specified in
// HOST.md next to this file.
//
// The other exports are the bridge's materialization helpers: the Rust host
// table calls `isSignal` / `isAccessor` / `read` / `write` / `subscribe` /
// `toSignal` to turn JS values into real `Binding<T>` / `Computed<T>` objects.

import {
  bridgeEquals,
  comparatorOf,
  createEffect,
  createMemo,
  createRoot,
  createSignal,
  isAccessor,
  isMemo,
  isSignal,
  onCleanup,
  untrack,
} from "./signals.js";

export { isAccessor, isMemo, isSignal };

/**
 * @typedef {unknown} Handle
 * An opaque native view handle. The runtime never inspects it; only the host
 * interprets it. See HOST.md.
 */

/**
 * @typedef {object} Branch
 * One realized branch of a control-flow view, created under its own scope.
 * @property {Handle} handle - The native view to mount for this branch.
 * @property {() => void} dispose - Tears the branch down; the host calls it
 *   exactly once when the branch stops being presented.
 */

/**
 * @typedef {object} HostEnvironment
 * @property {unknown} theme - Reactive-or-plain theme value; see `toSignal`.
 * @property {unknown} locale - Reactive-or-plain locale value.
 */

/**
 * @typedef {object} Host
 * The native side of the runtime. Specified in full in HOST.md. `when`,
 * `each`, `content`, config values, and modifier values are reactive inputs:
 * `T | Signal<T> | (() => T) | { read, subscribe? }`.
 * @property {(component: string, config: object, children: unknown[]) => Handle} create
 * @property {(handle: Handle, name: string, value: unknown) => Handle} modify
 * @property {(content: unknown) => Handle} text
 * @property {(when: unknown, render: (item: () => unknown) => Branch, fallback?: () => Branch) => Handle} show
 * @property {(each: unknown, render: (item: unknown, index: () => number) => Branch, by?: (item: unknown) => unknown) => Handle} each
 * @property {(children: () => Branch, fallback?: () => Branch) => Handle} suspense
 * @property {() => HostEnvironment} environment
 * @property {(id: number, ...args: unknown[]) => unknown} invoke - Dispatches
 *   one Rust closure held in the bridge's registry. It travels with the table
 *   rather than being read off `globalThis.__waterui_host` at call time,
 *   because that global is writable and a bundle that reassigns an entry of it
 *   while it evaluates would otherwise be what every callback wrapper calls.
 * @property {ReadonlySet<string> | readonly string[]} modifiers - The
 *   catalog's modifier attribute names. The runtime splits config from
 *   modifier attributes against it and rejects spread-carried modifiers; the
 *   Rust host table provides it because the catalog is the single source of
 *   truth. A `Set` cannot cross the engine seam as a value, so the Rust table
 *   sends the names as an array and `installHost` builds the set once.
 */

let installedHost = null;

/**
 * Installs the native host table. Called once by the bridge, after the bundle
 * is evaluated and before any module is mounted; every component creation
 * flows through it.
 *
 * The runtime keeps the table with its `modifiers` entry normalized to a real
 * `Set`, which may be a copy of the object passed in — a host entry must
 * therefore be a plain function and must not depend on `this`.
 *
 * @param {import("./host.js").Host} host
 */
export function installHost(host) {
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
    "invoke",
  ]) {
    if (typeof host[name] !== "function") {
      throw new TypeError(`waterui host is missing the "${name}" entry — see HOST.md`);
    }
  }
  if (Array.isArray(host.modifiers)) {
    installedHost = { ...host, modifiers: new Set(host.modifiers) };
    return;
  }
  if (
    host.modifiers === null ||
    typeof host.modifiers !== "object" ||
    typeof host.modifiers.has !== "function"
  ) {
    throw new TypeError(
      'waterui host is missing the "modifiers" entry — the catalog\'s modifier names as an array or a ReadonlySet<string>, see HOST.md',
    );
  }
  installedHost = host;
}

/** Removes the installed host. The bridge calls this when unloading a bundle. */
export function uninstallHost() {
  installedHost = null;
}

/** The installed host, or a precise error when the bundle ran without one. */
export function getHost() {
  if (installedHost === null) {
    throw new Error(
      "waterui: no host installed — installHost() must run before a module is mounted",
    );
  }
  return installedHost;
}

// ---------------------------------------------------------------------------
// Materialization helpers — what the Rust bridge calls.
// ---------------------------------------------------------------------------

function trackedRead(source) {
  return typeof source === "function" ? source() : source.read();
}

/**
 * Reads `value` without recording a dependency. Constants pass through, so
 * `read` also answers "the current value of anything positionally dynamic".
 */
export function read(value) {
  if (typeof value === "function" || (value !== null && typeof value === "object" && typeof value.read === "function")) {
    return untrack(() => trackedRead(value));
  }
  return value;
}

/**
 * Writes `value` into a signal or a host-side writable reactive value, and
 * answers whether the value stood: `true` when reading the target back gives
 * exactly what was written, `false` when an effect changed it while the write
 * settled. Anything else — memos, plain accessors, constants — is read-only
 * and fails loudly rather than being silently dropped.
 *
 * The bridge needs that answer and cannot work it out for itself. A write
 * settles its effects synchronously, and one of them may clamp, round or
 * reject the value; the comparison that catches it turns on object identity,
 * and the same signal or callback crossing back out to Rust is a fresh
 * reference there. Here it is the same object, so the comparison is exact.
 *
 * A value equal to what the target already holds — by `bridgeEquals`, the
 * seam's structural equality — is not written at all, and stands by
 * definition. The bridge confirms every inbound change by writing back what
 * the native side settled on, and that confirmation must cost no propagation:
 * the payload is a fresh object every time it crosses, so a reference
 * comparison would announce a change to every subscriber for a value nobody
 * touched. Values that do differ are written, and the target's own comparator
 * decides whether that write is a change.
 *
 * `identity` turns that comparison into `Object.is`. The native side passes
 * it when the value it sent is a retained handle — a live object, a function,
 * an opaque value — rather than data: a handle crosses as itself, so two
 * handles that look alike are still two different things, and JavaScript
 * cannot tell one from a plain object on its own.
 *
 * @param {unknown} target
 * @param {unknown} value
 * @param {boolean} [identity] - Compare with `Object.is` instead of
 *   structurally, because `value` crossed as a handle.
 * @returns {boolean} Whether the target now holds exactly `value`.
 */
export function write(target, value, identity = false) {
  const alreadyHolds = (held) => (identity ? Object.is(held, value) : bridgeEquals(held, value));
  if (isSignal(target)) {
    if (alreadyHolds(read(target))) {
      return true;
    }
    // An updater, not the value: `set` calls a function argument with the old
    // value and stores the result, so a callback or a signal written straight
    // through it would be invoked instead of stored. An updater that returns
    // the value stores it verbatim, whatever its type.
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

/**
 * Runs `callback(value)` on every settled change of `source`, without an
 * initial call. Sources are signals, memos, thunks (tracked), or host-side
 * `{ read, subscribe }` values. Returns a function that unsubscribes and
 * disposes the subscription.
 */
export function subscribe(source, callback) {
  if (
    source !== null &&
    typeof source === "object" &&
    typeof source.subscribe === "function"
  ) {
    return source.subscribe(callback);
  }
  if (typeof source !== "function") {
    throw new TypeError(
      "subscribe() expects a signal, an accessor, or a { read, subscribe } value",
    );
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

/**
 * Lifts `T | Signal<T> | (() => T)` to an accessor. Anything callable passes
 * through untouched; a `{ read() }` host value is wrapped so the result is
 * always callable — use `toSignal` instead when the subscription matters.
 */
export function toAccessor(value) {
  if (typeof value === "function") {
    return value;
  }
  if (value !== null && typeof value === "object" && typeof value.read === "function") {
    return () => value.read();
  }
  return () => value;
}

/**
 * Materializes any reactive-or-constant input as a real `Signal<T>`. Signals
 * and memos pass through; host-side `{ read, subscribe }` values become a
 * push-fed signal whose subscription is released with the current owner;
 * thunks become memos; constants become constant signals.
 */
export function toSignal(source) {
  if (isSignal(source) || isMemo(source)) {
    return source;
  }
  if (
    source !== null &&
    typeof source === "object" &&
    typeof source.read === "function"
  ) {
    const signal = createSignal(source.read());
    if (typeof source.subscribe === "function") {
      // An updater, for the same reason `write` uses one: a host value
      // carrying a function must be stored, not called.
      onCleanup(source.subscribe((value) => signal.set(() => value)));
    }
    return signal;
  }
  if (typeof source === "function") {
    return createMemo(source);
  }
  return createSignal(source);
}
