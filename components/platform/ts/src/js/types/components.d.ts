// Type surface for components.js — control flow as real components.

import type { Accessor, MaybeReactive } from "./signals.js";
import type { Element } from "./jsx-runtime.js";

export interface ShowProps<T> {
  /** Presents `children` while this reads truthy. */
  when: MaybeReactive<T>;
  fallback?: Element | (() => Element);
  /** A render prop receives an accessor of the current truthy value. */
  children?: Element | ((item: Accessor<NonNullable<T>>) => Element);
}

export declare function Show<T>(props: ShowProps<T>): Element;

export interface ForProps<T> {
  each: MaybeReactive<readonly T[]>;
  /** Item identity; defaults to referential identity, as in Solid. */
  by?: (item: T) => unknown;
  children: (item: T, index: Accessor<number>) => Element;
}

export declare function For<T>(props: ForProps<T>): Element;

export interface SuspenseProps {
  fallback?: Element | (() => Element);
  children?: Element | (() => Element);
}

export declare function Suspense(props: SuspenseProps): Element;

export interface BoxProps {
  /**
   * Exactly one element. `Box` creates no node: its modifier attributes
   * apply to this child in written order.
   */
  children: Element;
  /**
   * Applied in written order; reordering changes the result. The accepted
   * names are the catalog's modifier set — the host's `modifiers` — spelled
   * concretely by the generated component typings.
   */
  [modifier: string]: unknown;
}

export declare function Box(props: BoxProps): Element;