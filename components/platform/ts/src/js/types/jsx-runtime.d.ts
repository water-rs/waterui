// Type surface for jsx-runtime.js — the `waterui/jsx-runtime` entry the
// compile-time transform targets.
//
// Attribute order is semantic. Modifier attributes apply to the element in
// written order, left to right, exactly like a Rust modifier chain.
// Component typings themselves are generated from the component catalog
// (#670); the modifier prop comments below establish the convention that
// generated `.d.ts` follows.

import type { MaybeReactive } from "./signals.js";

/** A mounted view. Elements evaluate once into opaque native handles. */
export type Element = unknown;

export declare namespace JSX {
  type Element = unknown;

  interface ElementChildrenAttribute {
    children: unknown;
  }

  /**
   * WaterUI components by name. The generated catalog typings replace this
   * permissive map; the attribute names and their modifier-vs-configuration
   * split are authoritative there, not here.
   */
  interface IntrinsicElements {
    [component: string]: Record<string, unknown> | undefined;
  }

  interface IntrinsicAttributes {
    children?: unknown;
  }
}

/** Marker for props objects emitted by the transform that contain a spread. */
export declare function spreadProps<P extends object>(props: P): P;

export declare function Fragment(props: { children?: unknown }): unknown;

export declare function jsx(type: unknown, props: unknown): Element;
export declare const jsxs: typeof jsx;
export declare function jsxDEV(
  type: unknown,
  props: unknown,
  key?: unknown,
  isStaticChildren?: boolean,
  source?: unknown,
  self?: unknown,
): Element;

/**
 * The modifier attributes. A component's own `.d.ts` spells the subset it
 * accepts; every one of these prop doc comments must open with the same
 * sentence: applied in written order, reordering changes the result.
 */
export interface ModifierProps {
  /** Applied in written order; reordering changes the result. `true` is the default insets, a number all edges, or an `EdgeInsets` object `{ top, bottom, leading, trailing, horizontal, vertical }`. */
  padding?: MaybeReactive<true | number | EdgeInsets>;
  /** Applied in written order; reordering changes the result. */
  background?: MaybeReactive<unknown>;
  /** Applied in written order; reordering changes the result. */
  foreground?: MaybeReactive<unknown>;
  /** Applied in written order; reordering changes the result. */
  font?: MaybeReactive<unknown>;
  /** Applied in written order; reordering changes the result. */
  frame?: MaybeReactive<unknown>;
  /** Applied in written order; reordering changes the result. */
  opacity?: MaybeReactive<number>;
  /** Applied in written order; reordering changes the result. */
  cornerRadius?: MaybeReactive<number>;
  /** Applied in written order; reordering changes the result. */
  shadow?: MaybeReactive<unknown>;
  /** Applied in written order; reordering changes the result. */
  border?: MaybeReactive<unknown>;
  /** Applied in written order; reordering changes the result. */
  blur?: MaybeReactive<number>;
  /** Applied in written order; reordering changes the result. */
  offset?: MaybeReactive<unknown>;
  /** Applied in written order; reordering changes the result. */
  overlay?: MaybeReactive<unknown>;
  /** Applied in written order; reordering changes the result. */
  clipped?: MaybeReactive<boolean>;
  /** Applied in written order; reordering changes the result. */
  aspectRatio?: MaybeReactive<number>;
  /** Applied in written order; reordering changes the result. */
  disabled?: MaybeReactive<boolean>;
  /** Applied in written order; reordering changes the result. */
  hidden?: MaybeReactive<boolean>;
  /** Applied in written order; reordering changes the result. */
  safeArea?: MaybeReactive<unknown>;
  /** Applied in written order; reordering changes the result. */
  bold?: MaybeReactive<boolean>;
  /** Applied in written order; reordering changes the result. */
  italic?: MaybeReactive<boolean>;
  /** Applied in written order; reordering changes the result. */
  accessibilityLabel?: MaybeReactive<string>;
  /** Applied in written order; reordering changes the result. */
  accessibilityValue?: MaybeReactive<string>;
}

export interface EdgeInsets {
  top?: MaybeReactive<number>;
  bottom?: MaybeReactive<number>;
  leading?: MaybeReactive<number>;
  trailing?: MaybeReactive<number>;
  horizontal?: MaybeReactive<number>;
  vertical?: MaybeReactive<number>;
}
