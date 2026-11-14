// The `waterui/jsx-runtime` entry the compile-time transform targets.
//
// Elements evaluate once into opaque native view handles. There is no vdom
// and no diffing: a function tag is a component invoked once with
// getter-props, a string tag is a WaterUI component name resolved through the
// host.
//
// Attribute order is semantic. Attributes whose names are in the host's
// modifier table (`host.modifiers`, the catalog's modifier names) apply to
// the element in written attribute order, left to right, through
// `host.modify` — `<Text padding={12} background={a}>` is
// `text().padding(12).background(a)`. Configuration attributes do not
// participate in ordering. The transform preserves written order by emitting
// properties in source order, and this module consumes them in enumeration
// order.
//
// A spread may not carry a modifier attribute — order would be invisible at
// the element. The transform rejects it statically; this module is the
// runtime backstop: the transform wraps any props object containing a spread
// in `spreadProps({...})`, and a modifier name found on a flagged object
// throws at element creation, naming the attribute and the element.
//
// Getter-props are reactive inputs. The transform emits
// `get padding() { return count() }`, so reading the property once at setup
// would capture a snapshot and the position would never update. Every prop
// is therefore forwarded through its own-property descriptor: a getter
// becomes the accessor passed to the host (`T | Signal<T> | (() => T)` per
// HOST.md), a plain value passes as a constant.

import { getHost } from "./host.js";
import { untrack } from "./signals.js";

const CONTAINS_SPREAD = Symbol("waterui.contains-spread");

/**
 * Marks a props object as containing a JSX spread. The transform wraps props
 * literals that use `{...expr}`: `jsx("Text", spreadProps({ a: 1, ...rest }))`.
 * The flag is non-enumerable so further spreads do not copy it.
 */
export function spreadProps(props) {
  Object.defineProperty(props, CONTAINS_SPREAD, { value: true });
  return props;
}

/** Whether `props` was built by `spreadProps` — the spread backstop check. */
export function hasSpread(props) {
  return props !== null && typeof props === "object" && props[CONTAINS_SPREAD] === true;
}

/**
 * Reads a prop once for classification or one-shot config: a getter is
 * invoked (it is an accessor by convention), a data property returns its
 * value. Use `reactiveInput` instead whenever the prop reaches the host.
 */
export function propValue(props, name) {
  const descriptor = Object.getOwnPropertyDescriptor(props, name);
  if (descriptor === undefined) {
    return undefined;
  }
  return descriptor.get === undefined ? descriptor.value : descriptor.get.call(props);
}

/** A prop as a lazy reader: `lazyProp(props, name)()` re-evaluates the getter. */
export function lazyProp(props, name) {
  return () => propValue(props, name);
}

/**
 * A prop as a reactive input for the host: a getter becomes the accessor
 * itself so later writes propagate, a data property passes as a constant.
 */
export function reactiveInput(props, name) {
  const descriptor = Object.getOwnPropertyDescriptor(props, name);
  if (descriptor === undefined) {
    return undefined;
  }
  return descriptor.get === undefined ? descriptor.value : () => descriptor.get.call(props);
}

/** The fragment tag: evaluates to its children, nothing else. */
export function Fragment(props) {
  return props.children;
}

function normalizeChildren(children, into) {
  if (children === undefined || children === null || typeof children === "boolean") {
    return into;
  }
  if (Array.isArray(children)) {
    for (const child of children) {
      normalizeChildren(child, into);
    }
    return into;
  }
  into.push(children);
  return into;
}

/**
 * The JSX factory. `type` is a WaterUI component name (string) or a component
 * function; `props` is the transform-emitted props object with getter-props.
 */
export function jsx(type, props) {
  if (typeof type === "function") {
    // Component setup is non-reactive: the function runs once under the
    // current owner, and signal reads inside it bind nothing upstream.
    return untrack(() => type(props ?? {}));
  }
  if (typeof type !== "string") {
    throw new TypeError(`waterui: <${String(type)}> is not a component`);
  }
  const host = getHost();
  const config = {};
  const modifiers = [];
  let children;
  if (props !== null && props !== undefined) {
    const spread = hasSpread(props);
    for (const name of Object.keys(props)) {
      if (name === "children") {
        children = reactiveInput(props, name);
        continue;
      }
      if (host.modifiers.has(name)) {
        if (spread) {
          throw new Error(
            `<${type}> received modifier attribute "${name}" through a spread. ` +
              "Modifier attributes apply in written order, so they must be written literally on the element.",
          );
        }
        modifiers.push([name, reactiveInput(props, name)]);
        continue;
      }
      config[name] = reactiveInput(props, name);
    }
  }
  let handle = host.create(type, config, normalizeChildren(children, []));
  for (const [name, value] of modifiers) {
    handle = host.modify(handle, name, value);
  }
  return handle;
}

export const jsxs = jsx;

export function jsxDEV(type, props) {
  return jsx(type, props);
}
