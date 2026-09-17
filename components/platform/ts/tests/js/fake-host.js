// A reference host for the unit suite.
//
// It implements the HOST.md contract in plain JS over the real signal
// library: `show` and `each` do the same branch/identity bookkeeping the Rust
// host table must do, `suspense` presents the children branch the way the Rust
// table does, and every entry point records its calls so tests can assert on
// order, reuse, and disposal. `modifiers` is the fake's own catalog subset —
// the real set comes from the generated component catalog.
//
// It is a reference host, so where it differs from the Rust table it is wrong:
// a test that passes here and fails against the real one has proved nothing.
// That is why `each` demands a `by` for a non-primitive item and `suspense`
// has no pending state — see both below.

import { createEffect, createRoot, createSignal } from "../../src/js/signals.js";
import { toSignal } from "../../src/js/host.js";

export function createFakeHost(environment = {}) {
  const calls = [];
  const invocations = [];
  let nextId = 1;

  const host = {
    calls,

    modifiers: new Set(["padding", "background", "foreground", "frame", "font"]),

    create(component, config, children) {
      const handle = { id: nextId, type: "view", component, config, children };
      nextId += 1;
      calls.push(["create", component, config, children]);
      return handle;
    },

    modify(handle, name, value) {
      handle.modifiers = handle.modifiers ?? [];
      handle.modifiers.push([name, value]);
      calls.push(["modify", handle.id, name, value]);
      return handle;
    },

    text(content) {
      const handle = { id: nextId, type: "text", content };
      nextId += 1;
      calls.push(["text", content]);
      return handle;
    },

    show(when, render, fallback) {
      const slot = { id: nextId, type: "show", branch: null };
      nextId += 1;
      slot.dispose = createRoot((dispose) => {
        const condition = toSignal(when);
        let side;
        createEffect(() => {
          const truthy = Boolean(condition());
          if (truthy === side) {
            return;
          }
          side = truthy;
          slot.branch?.dispose();
          slot.branch = truthy ? render(condition) : fallback ? fallback() : null;
        });
        return dispose;
      });
      return slot;
    },

    each(each, render, by) {
      const slot = { id: nextId, type: "each", entries: [] };
      nextId += 1;
      // An item identifies itself only when it is a primitive. Across the
      // engine seam an object arrives as a copy, so the Rust table has no
      // referential identity to reconcile by and demands `by`; a fake that
      // reconciled objects would make a test pass that the real host refuses.
      const keyOf =
        by ??
        ((item) => {
          if (item !== null && (typeof item === "object" || typeof item === "function")) {
            throw new TypeError(
              "<For> over objects needs `by`: an object crosses to the host as a copy, so its " +
                "referential identity is gone by the time rows are reconciled",
            );
          }
          return item;
        });
      slot.dispose = createRoot((dispose) => {
        const list = toSignal(each);
        createEffect(() => {
          const items = list() ?? [];
          const previous = slot.entries;
          const byKey = new Map(previous.map((entry) => [entry.key, entry]));
          const seen = new Set();
          const next = items.map((item, index) => {
            const key = keyOf(item);
            // Two rows sharing one key is a list that cannot be reconciled, and
            // the Rust table refuses it by name. A fake that accepted it would
            // silently collapse the two and make a test pass that the real host
            // fails.
            if (seen.has(key)) {
              throw new TypeError(
                `two items of this <For> have the key ${JSON.stringify(key)}: give \`by\` ` +
                  "something unique, or make the items themselves distinct",
              );
            }
            seen.add(key);
            const kept = byKey.get(key);
            if (kept !== undefined && !kept.used) {
              kept.used = true;
              kept.index.set(index);
              return kept;
            }
            const position = createSignal(index);
            const inner = render(item, position);
            return {
              key,
              item,
              index: position,
              handle: inner.handle,
              dispose: () => {
                inner.dispose();
                calls.push(["dispose-item", key]);
              },
            };
          });
          for (const entry of previous) {
            if (!entry.used) {
              entry.dispose();
            }
          }
          for (const entry of next) {
            delete entry.used;
          }
          slot.entries = next;
        });
        return dispose;
      });
      return slot;
    },

    // The boundary presents its children. There is no resource primitive in
    // the JavaScript runtime, so nothing a `children()` branch builds out of
    // JSX alone can be pending and the fallback is never built — which is
    // exactly what the Rust table does.
    suspense(children, _fallback) {
      const slot = { id: nextId, type: "suspense", branch: null };
      nextId += 1;
      slot.dispose = createRoot((dispose) => {
        slot.branch = children();
        return dispose;
      });
      return slot;
    },

    environment() {
      return environment;
    },

    // The bridge's own entry: `makeCallback` wraps this one. Recorded rather
    // than dispatched, since there is no Rust registry behind a fake host.
    invoke(id, ...args) {
      invocations.push([id, ...args]);
      calls.push(["invoke", id, ...args]);
      return "invoked";
    },

    invocations,
  };

  return host;
}
