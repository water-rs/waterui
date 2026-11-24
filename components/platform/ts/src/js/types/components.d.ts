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

/** A primitive value that identifies one row of `<For>` across the native boundary. */
export type Key = string | number | bigint | boolean;

export type ForProps<T> = {
  each: MaybeReactive<readonly T[]>;
  children: (item: T, index: Accessor<number>) => Element;
} & ([T] extends [Key]
  ? {
      /**
       * Item identity; optional when items are already keys (`string`,
       * `number`, `bigint`, `boolean`).
       */
      by?: (item: T) => Key;
    }
  : {
      /**
       * Item identity; required because non-key items cross the native
       * boundary as data without referential identity.
       */
      by: (item: T) => Key;
    });

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
