// Type surface for jsx-runtime.js — the `waterui/jsx-runtime` entry the
// compile-time transform targets.
//
// Attribute order is semantic. Modifier attributes apply to the element in
// written order, left to right, exactly like a Rust modifier chain.
// Component and modifier typings are generated from the component catalog
// (#670) — the catalog is the single source of truth for which attribute
// names are modifiers, so this file declares none. Generated modifier prop
// comments open with the same sentence the runtime documents: applied in
// written order, reordering changes the result.

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

/** Whether `props` was built by `spreadProps` — the spread backstop check. */
export declare function hasSpread(props: unknown): boolean;

/**
 * Reads a prop once for classification or one-shot config: a getter is
 * invoked, a data property returns its value.
 */
export declare function propValue(props: object, name: string): unknown;

/** A prop as a lazy reader: `lazyProp(props, name)()` re-evaluates the getter. */
export declare function lazyProp(props: object, name: string): () => unknown;

/**
 * A prop as a reactive input for the host: a getter becomes the accessor
 * itself so later writes propagate, a data property passes as a constant.
 */
export declare function reactiveInput(props: object, name: string): unknown;

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
