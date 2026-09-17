// Control-flow components, implemented on the host primitives of HOST.md.
//
// Each render callback handed to the host is wrapped in a *branch*: it runs
// under a fresh reactive scope that inherits the owner active when this
// element was created — so contexts resolve and `onCleanup`s die with the
// branch — and returns `{ handle, dispose }` for the host to present and
// later tear down.

import { getHost, isSignal } from "./host.js";
import { createScope, getOwner, isMemo, runWithOwner } from "./signals.js";
import { MODIFIER_NAMES } from "./modifiers.js";

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

/**
 * `<Show when={…} fallback={…}>…</Show>` — presents its children while `when`
 * is truthy, the fallback otherwise. A function child is a render prop that
 * receives an accessor of the current truthy value.
 */
export function Show(props) {
  const host = getHost();
  const when = props.when;
  const children = props.children;
  const render =
    typeof children === "function" && !isSignal(children) && !isMemo(children)
      ? branch((item) => children(item))
      : branch(() => children);
  const fallback =
    props.fallback === undefined
      ? undefined
      : branch(() => resolveElement(props.fallback));
  return host.show(when, render, fallback);
}

/**
 * `<For each={…} by={…}>{(item, index) => …}</For>` — keyed reconciliation.
 * `by` defaults to referential identity. The render function receives the
 * item and an accessor of its current index.
 */
export function For(props) {
  const host = getHost();
  const renderFn = props.children;
  if (typeof renderFn !== "function" || isSignal(renderFn) || isMemo(renderFn)) {
    throw new TypeError(
      "<For> requires a function child: <For each={items}>{(item, index) => …}</For>",
    );
  }
  return host.each(
    props.each,
    branch((item, index) => renderFn(item, index)),
    props.by,
  );
}

/** `<Suspense fallback={…}>…</Suspense>` — host-driven pending state. */
export function Suspense(props) {
  const host = getHost();
  const fallback =
    props.fallback === undefined
      ? undefined
      : branch(() => resolveElement(props.fallback));
  return host.suspense(branch(() => resolveElement(props.children)), fallback);
}

/**
 * `<Box>…</Box>` — a zero-cost modifier scope. It creates no node: its
 * modifier attributes apply to its single child in written order. Nesting
 * `<Box>`es is how a modifier is spelled twice, which JSX's
 * duplicate-attribute rule otherwise forbids. `children` is singular — more
 * than one child is an error.
 */
export function Box(props) {
  const host = getHost();
  const child = resolveElement(props.children);
  if (child === undefined || child === null) {
    throw new Error("<Box> requires exactly one child element, got none");
  }
  if (Array.isArray(child)) {
    throw new Error(`<Box> accepts exactly one child element, got ${child.length}`);
  }
  if (typeof child === "function") {
    throw new TypeError("<Box> requires a resolved element, not a reactive child");
  }
  let handle = child;
  for (const name of Object.keys(props)) {
    if (name === "children") {
      continue;
    }
    if (!MODIFIER_NAMES.has(name)) {
      throw new Error(
        `<Box> takes only modifier attributes and a single child; got "${name}"`,
      );
    }
    handle = host.modify(handle, name, props[name]);
  }
  return handle;
}
