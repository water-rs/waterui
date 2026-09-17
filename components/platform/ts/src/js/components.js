// Control-flow components, implemented on the host primitives of HOST.md.
//
// Each render callback handed to the host is wrapped in a *branch*: it runs
// under a fresh reactive scope that inherits the owner active when this
// element was created — so contexts resolve and `onCleanup`s die with the
// branch — and returns `{ handle, dispose }` for the host to present and
// later tear down.
//
// Props are read through their own-property descriptors exactly as in
// `jsx-runtime.js`: a getter is the reactive input and reaches the host
// (or the per-activation branch) unevaluated, a data property is a constant
// or the render function itself. Reading `props.when` once would snapshot
// `get when() { return flag() }` into a boolean the host could never update.

import { getHost, isSignal } from "./host.js";
import { createScope, getOwner, isMemo, runWithOwner } from "./signals.js";
import { hasSpread, lazyProp, propValue, reactiveInput } from "./jsx-runtime.js";

function branch(render) {
  const owner = getOwner();
  return (...args) =>
    runWithOwner(owner, () =>
      createScope((dispose) => ({ handle: render(...args), dispose })),
    );
}

// One level of thunk resolution for element children: `{() => <Text />}` in
// a position that needs a concrete element evaluates once. Signals and memos
// are never invoked — they are values, not thunks.
function resolveElement(value) {
  return typeof value === "function" && !isSignal(value) && !isMemo(value)
    ? value()
    : value;
}

function isFunctionValue(value) {
  return typeof value === "function" && !isSignal(value) && !isMemo(value);
}

/**
 * `<Show when={…} fallback={…}>…</Show>` — presents its children while `when`
 * is truthy, the fallback otherwise. A function child is a render prop that
 * receives an accessor of the current truthy value. The non-function form
 * re-reads the child on every activation so a disposed branch never resurfaces.
 */
export function Show(props) {
  const host = getHost();
  const children = propValue(props, "children");
  const render = isFunctionValue(children)
    ? branch((item) => children(item))
    : branch(() => resolveElement(lazyProp(props, "children")()));
  const fallback =
    propValue(props, "fallback") === undefined
      ? undefined
      : branch(() => resolveElement(lazyProp(props, "fallback")()));
  return host.show(reactiveInput(props, "when"), render, fallback);
}

/**
 * `<For each={…} by={…}>{(item, index) => …}</For>` — keyed reconciliation.
 * `by` defaults to referential identity. The render function receives the
 * item and an accessor of its current index.
 */
export function For(props) {
  const host = getHost();
  const renderFn = propValue(props, "children");
  if (!isFunctionValue(renderFn)) {
    throw new TypeError(
      "<For> requires a function child: <For each={items}>{(item, index) => …}</For>",
    );
  }
  return host.each(
    reactiveInput(props, "each"),
    branch((item, index) => renderFn(item, index)),
    propValue(props, "by"),
  );
}

/** `<Suspense fallback={…}>…</Suspense>` — host-driven pending state. */
export function Suspense(props) {
  const host = getHost();
  const fallback =
    propValue(props, "fallback") === undefined
      ? undefined
      : branch(() => resolveElement(lazyProp(props, "fallback")()));
  return host.suspense(
    branch(() => resolveElement(lazyProp(props, "children")())),
    fallback,
  );
}

/**
 * `<Box>…</Box>` — a zero-cost modifier scope. It creates no node: its
 * modifier attributes apply to its single child in written order. Nesting
 * `<Box>`es is how a modifier is spelled twice, which JSX's
 * duplicate-attribute rule otherwise forbids. `children` is singular — more
 * than one child is an error, and so is any child that is not a host element.
 */
export function Box(props) {
  const host = getHost();
  const child = resolveElement(propValue(props, "children"));
  if (child === undefined || child === null) {
    throw new Error("<Box> requires exactly one child element, got none");
  }
  if (Array.isArray(child)) {
    throw new Error(`<Box> accepts exactly one child element, got ${child.length}`);
  }
  if (typeof child !== "object") {
    throw new TypeError(
      `<Box> requires a host element as its child, got ${typeof child === "function" ? "a reactive child" : JSON.stringify(child)}`,
    );
  }
  const spread = hasSpread(props);
  let handle = child;
  for (const name of Object.keys(props)) {
    if (name === "children") {
      continue;
    }
    if (!host.modifiers.has(name)) {
      throw new Error(
        `<Box> takes only modifier attributes and a single child; got "${name}"`,
      );
    }
    if (spread) {
      throw new Error(
        `<Box> received modifier attribute "${name}" through a spread. ` +
          "Modifier attributes apply in written order, so they must be written literally on the element.",
      );
    }
    handle = host.modify(handle, name, reactiveInput(props, name));
  }
  return handle;
}
