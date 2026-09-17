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
  safeArea: unknown;
}

/** The native host table. Specified in full in HOST.md. */
export interface Host {
  create(component: string, config: object, children: unknown[]): Handle;
  modify(handle: Handle, name: string, value: unknown): Handle;
  text(content: MaybeReactive<string | number>): Handle;
  show(
    when: Accessor<unknown>,
    render: (item: Accessor<unknown>) => Branch,
    fallback?: () => Branch,
  ): Handle;
  each(
    each: Accessor<readonly unknown[]>,
    render: (item: unknown, index: Accessor<number>) => Branch,
    by?: (item: unknown) => unknown,
  ): Handle;
  suspense(children: () => Branch, fallback?: () => Branch): Handle;
  environment(): HostEnvironment;
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

/** Writes a signal or a writable host value; throws on read-only inputs. */
export declare function write<T>(target: Signal<T> | HostReactive<T>, value: T): void;

/** Runs `callback` on every settled change, with no initial call. */
export declare function subscribe<T>(
  source: Accessor<T> | Signal<T> | HostReactive<T>,
  callback: (value: T) => void,
): () => void;

/** Lifts `T | Signal<T> | (() => T)` to an accessor. */
export declare function toAccessor<T>(value: MaybeReactive<T> | HostReactive<T>): Accessor<T>;

/** Materializes a reactive-or-plain input as a real `Signal<T>`. */
export declare function toSignal<T>(source: MaybeReactive<T> | HostReactive<T>): Signal<T>;
