// Type surface for signals.js.

/** `false` disables equality; a function compares old and new. */
export type EqualsFn<T> = false | ((previous: T, next: T) => boolean);

export interface SignalOptions<T> {
  equals?: EqualsFn<T>;
}

/** A read-only reactive value. Call it to read and track. */
export interface Accessor<T> {
  (): T;
  /** Derives a memo of `f` applied to this value. Mirrors `SignalExt::map`. */
  map<U>(f: (value: T) => U): Accessor<U>;
  /** Reads without tracking. */
  peek(): T;
}

/**
 * A writable reactive value — the Binding shape. `count()` reads and tracks;
 * `count.set(v)` writes (a function argument is an updater); `count.update(f)`
 * writes `f(previous)`. Iterating yields `[read, write]`, so both
 * `const [count, setCount] = createSignal(0)` and `const done =
 * createSignal(false)` work.
 */
export interface Signal<T> extends Accessor<T> {
  set(value: T | ((previous: T) => T)): void;
  update(f: (previous: T) => T): void;
  [Symbol.iterator](): IterableIterator<[Accessor<T>, (value: T | ((previous: T) => T)) => void]>;
}

/** Anything accepted at a dynamic position: lifted to a computed. */
export type MaybeReactive<T> = T | Accessor<T> | Signal<T>;

/** An opaque reactive owner. Create one with `createRoot`/`createScope`. */
export interface Owner {
  readonly __brand: "waterui.owner";
}

export declare function createSignal<T>(initial: T, options?: SignalOptions<T>): Signal<T>;

export declare function createMemo<T>(
  compute: (previous: T | undefined) => T,
  options?: SignalOptions<T>,
): Accessor<T>;

export declare function createEffect(fn: (previous: unknown) => void): void;

export declare function createRoot<T>(fn: (dispose: () => void) => T): T;

export declare function createScope<T>(fn: (dispose: () => void) => T): T;

export declare function onCleanup(fn: () => void): void;

export declare function getOwner(): Owner | null;

export declare function runWithOwner<T>(owner: Owner | null, fn: () => T): T;

export declare function untrack<T>(fn: () => T): T;

export declare function batch<T>(fn: () => T): T;

export declare function isSignal(value: unknown): value is Signal<unknown>;

export declare function isMemo(value: unknown): value is Accessor<unknown>;

export declare function isAccessor(value: unknown): boolean;

/** A store is the proxied form of `T` — read and write it like `T`. */
export type Store<T> = T;

/**
 * `setStore(path..., value | updater)` or `setStore(producer)`. A single
 * function is a producer that mutates the store; a returned object replaces
 * the root's contents.
 */
export interface SetStore<T> {
  (producer: (store: Store<T>) => void | T): void;
  (...args: unknown[]): void;
}

export declare function createStore<T extends object>(initial: T): [Store<T>, SetStore<T>];

export interface Context<T> {
  /** Not namespaced — the id is unique per context. */
  readonly id: symbol;
  readonly defaultValue: T;
  Provider(props: { value: T; children?: unknown }): unknown;
}

export declare function createContext<T>(defaultValue?: T): Context<T>;

export declare function useContext<T>(context: Context<T>): T;
