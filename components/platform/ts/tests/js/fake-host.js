// A reference host for the unit suite.
//
// It implements the HOST.md contract in plain JS over the real signal
// library: `show` and `each` do the same branch/identity bookkeeping the Rust
// host table must do, `suspense` presents the fallback while `setPending`
// holds the pending state, and every entry point records its calls so tests
// can assert on order, reuse, and disposal. `modifiers` is the fake's own
// catalog subset — the real set comes from the generated component catalog.

import { createEffect, createRoot, createSignal } from "../../src/js/signals.js";
import { toSignal } from "../../src/js/host.js";

export function createFakeHost(environment = {}) {
  const calls = [];
  let nextId = 1;
  const pending = createSignal(false);

  const host = {
    calls,

    modifiers: new Set(["padding", "background", "foreground", "frame", "font"]),

    // Test control, not part of the Host contract: drives `suspense`.
    setPending(value) {
      pending.set(value);
    },

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
      const keyOf = by ?? ((item) => item);
      slot.dispose = createRoot((dispose) => {
        const list = toSignal(each);
        createEffect(() => {
          const items = list() ?? [];
          const previous = slot.entries;
          const byKey = new Map(previous.map((entry) => [entry.key, entry]));
          const next = items.map((item, index) => {
            const key = keyOf(item);
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

    suspense(children, fallback) {
      const slot = { id: nextId, type: "suspense", branch: null };
      nextId += 1;
      slot.dispose = createRoot((dispose) => {
        let side;
        createEffect(() => {
          const isPending = pending();
          if (isPending === side) {
            return;
          }
          side = isPending;
          slot.branch?.dispose();
          slot.branch = isPending ? (fallback ? fallback() : null) : children();
        });
        return dispose;
      });
      return slot;
    },

    environment() {
      return environment;
    },
  };

  return host;
}
