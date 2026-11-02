// Type surface for host.js — the seam to the native side. See HOST.md.

import type { Accessor, MaybeReactive, Signal } from "./signals.js";

/** An opaque native view handle. */
export type Handle = unknown;

/** One realized branch of a control-flow view. */
export interface Branch {
  handle: Handle;
  dispose(): void;
}

/** Environment values the host provides at mount. */
export interface HostEnvironment {
  theme: unknown;
  locale: unknown;
}

/** Anywhere a value can be dynamic: constant, signal, accessor, or host value. */
export type ReactiveInput<T> = MaybeReactive<T> | HostReactive<T>;

/** The native host table. Specified in full in HOST.md. */
export interface Host {
  create(component: string, config: object, children: unknown[]): Handle;
  modify(handle: Handle, name: string, value: unknown): Handle;
  text(content: MaybeReactive<string | number>): Handle;
  show(
    when: ReactiveInput<unknown>,
    render: (item: Accessor<unknown>) => Branch,
    fallback?: () => Branch,
  ): Handle;
  each(
    each: ReactiveInput<readonly unknown[]>,
    render: (item: unknown, index: Accessor<number>) => Branch,
    by?: (item: unknown) => unknown,
  ): Handle;
  suspense(children: () => Branch, fallback?: () => Branch): Handle;
  environment(): HostEnvironment;
  /**
   * Dispatches one Rust closure held in the bridge's registry. `makeCallback`
   * wraps this one, never `globalThis.__waterui_host.invoke`: that global is
   * writable, and the table carries the function the engine registered.
   */
  invoke(id: number, ...args: unknown[]): unknown;
  /**
   * The catalog's modifier attribute names — the runtime keeps no table of
   * its own. A `Set` cannot cross the engine seam, so the Rust host table
   * sends an array and `installHost` builds the set once.
   */
  modifiers: ReadonlySet<string> | readonly string[];
}

/** A host-side reactive value: read plus push subscription. */
export interface HostReactive<T> {
  read(): T;
  subscribe(callback: (value: T) => void): () => void;
  write?(value: T): void;
}

export declare function installHost(host: Host): void;
export declare function uninstallHost(): void;
export declare function getHost(): Host;

export declare function isSignal(value: unknown): value is Signal<unknown>;
export declare function isMemo(value: unknown): value is Accessor<unknown>;
export declare function isAccessor(value: unknown): boolean;

/** Reads a reactive-or-plain value without tracking. */
export declare function read<T>(value: MaybeReactive<T> | HostReactive<T>): T;

/**
 * Writes a signal or a writable host value; throws on read-only inputs. The
 * value is stored verbatim: a function is the value, never an updater.
 * Answers whether the value stood — `true` when reading the target back gives
 * exactly what was written, `false` when an effect changed it — compared with
 * the comparator the target settles on.
 *
 * A value the target already holds is not written at all: data is compared
 * structurally, and a value the native side sent as a retained handle is
 * compared by identity, which it asks for with `identity`.
 */
export declare function write<T>(
  target: Signal<T> | HostReactive<T>,
  value: T,
  identity?: boolean,
): boolean;

/** Runs `callback` on every settled change, with no initial call. */
export declare function subscribe<T>(
  source: Accessor<T> | Signal<T> | HostReactive<T>,
  callback: (value: T) => void,
): () => void;

/** Lifts `T | Signal<T> | (() => T)` to an accessor. */
export declare function toAccessor<T>(value: MaybeReactive<T> | HostReactive<T>): Accessor<T>;

/**
 * Materializes a reactive-or-plain input as a runtime-tracked reactive value.
 * A signal passes through and stays writable; a `{ read, subscribe }` host
 * value becomes a writable signal that pushes through `subscribe`; a thunk or
 * memo is read-only, so the return type is `Signal<T> | Accessor<T>` — treat
 * the result as an `Accessor<T>` unless the input was a `Signal<T>`.
 */
export declare function toSignal<T>(
  source: MaybeReactive<T> | HostReactive<T>,
): Signal<T> | Accessor<T>;
