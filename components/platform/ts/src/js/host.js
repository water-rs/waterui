// The seam between the JS runtime and the native side.
//
// Everything the runtime needs from Rust — creating views, applying modifiers,
// control flow, the environment — goes through the object installed by
// `installHost`. The engine installs it before evaluating the bundle; the
// contract it must satisfy is specified in HOST.md next to this file.
//
// The other exports are the bridge's materialization helpers: the Rust host
// table calls `isSignal` / `isAccessor` / `read` / `write` / `subscribe` /
// `toSignal` to turn JS values into real `Binding<T>` / `Computed<T>` objects.

import {
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
 * @property {unknown} safeArea - Reactive-or-plain safe-area insets.
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
 * @property {ReadonlySet<string>} modifiers - The catalog's modifier
 *   attribute names. The runtime splits config from modifier attributes
 *   against it and rejects spread-carried modifiers; the Rust host table
 *   provides it because the catalog is the single source of truth.
 */

let installedHost = null;

/**
 * Installs the native host table. Called once by the engine before the bundle
 * is evaluated; every component creation flows through it.
 *
 * @param {import("./host.js").Host} host
 */
export function installHost(host) {
  if (installedHost !== null) {
    throw new Error("waterui: a host is already installed");
  }
  for (const name of ["create", "modify", "text", "show", "each", "suspense", "environment"]) {
    if (typeof host[name] !== "function") {
      throw new TypeError(`waterui host is missing the "${name}" entry — see HOST.md`);
    }
  }
  if (
    host.modifiers === null ||
    typeof host.modifiers !== "object" ||
    typeof host.modifiers.has !== "function"
  ) {
    throw new TypeError(
      'waterui host is missing the "modifiers" entry — a ReadonlySet<string> of the catalog\'s modifier names, see HOST.md',
    );
  }
  installedHost = host;
}

/** Removes the installed host. The engine calls this when unloading a bundle. */
export function uninstallHost() {
  installedHost = null;
}

/** The installed host, or a precise error when the bundle ran without one. */
export function getHost() {
  if (installedHost === null) {
    throw new Error(
      "waterui: no host installed — installHost() must run before the bundle is evaluated",
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
 * Writes `value` into a signal or a host-side writable reactive value.
 * Anything else — memos, plain accessors, constants — is read-only and fails
 * loudly rather than being silently dropped.
 */
export function write(target, value) {
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
      onCleanup(source.subscribe((value) => signal.set(value)));
    }
    return signal;
  }
  if (typeof source === "function") {
    return createMemo(source);
  }
  return createSignal(source);
}
