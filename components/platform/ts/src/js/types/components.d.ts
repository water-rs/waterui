// Type surface for components.js — control flow as real components.

import type { Accessor, MaybeReactive } from "./signals.js";
import type { Element, ModifierProps } from "./jsx-runtime.js";

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

export interface BoxProps extends ModifierProps {
  /**
   * Exactly one element. `Box` creates no node: its modifier attributes
   * apply to this child in written order.
   */
  children: Element;
}

export declare function Box(props: BoxProps): Element;
