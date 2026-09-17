// The control-flow components against the reference host: branch lifetimes,
// keyed reconciliation, and Box's zero-cost modifier scope.

import { beforeEach, describe, expect, test } from "bun:test";
import { installHost, uninstallHost } from "../../src/js/host.js";
import { Box, For, Show, Suspense } from "../../src/js/components.js";
import { jsx, spreadProps } from "../../src/js/jsx-runtime.js";
import { createRoot, createSignal, onCleanup } from "../../src/js/signals.js";
import { createFakeHost } from "./fake-host.js";

let host;
beforeEach(() => {
  uninstallHost();
  host = createFakeHost();
  installHost(host);
});

describe("Show", () => {
  test("presents the main branch while truthy, the fallback otherwise", () => {
    const when = createSignal(false);
    createRoot((dispose) => {
      const slot = jsx(Show, {
        when,
        fallback: () => jsx("Text", { children: "off" }),
        children: () => jsx("Text", { children: "on" }),
      });
      expect(slot.branch.handle.children).toEqual(["off"]);
      when.set(true);
      expect(slot.branch.handle.children).toEqual(["on"]);
      when.set(false);
      expect(slot.branch.handle.children).toEqual(["off"]);
      dispose();
    });
  });

  test("disposes the inactive branch on every flip", () => {
    const when = createSignal(false);
    const log = [];
    const dispose = createRoot((d) => {
      jsx(Show, {
        when,
        fallback: () => {
          onCleanup(() => log.push("fallback disposed"));
          return jsx("Text", { children: "off" });
        },
        children: () => {
          onCleanup(() => log.push("main disposed"));
          return jsx("Text", { children: "on" });
        },
      });
      return d;
    });
    when.set(true);
    when.set(false);
    expect(log).toEqual(["fallback disposed", "main disposed"]);
    dispose();
    expect(log).toEqual(["fallback disposed", "main disposed", "fallback disposed"]);
  });

  test("a function child receives an accessor of the truthy value", () => {
    const when = createSignal(0);
    let seen;
    createRoot((dispose) => {
      jsx(Show, {
        when,
        children: (item) => {
          seen = item;
          return jsx("Text", {});
        },
      });
      when.set(5);
      expect(seen()).toBe(5);
      when.set(7);
      // Same side, no recreation: the accessor carries the update.
      expect(seen()).toBe(7);
      dispose();
    });
  });

  test("a `when` getter reaches the host as an accessor and stays live", () => {
    const flag = createSignal(false);
    createRoot((dispose) => {
      const slot = jsx(Show, {
        get when() {
          return flag();
        },
        fallback: () => jsx("Text", { children: "off" }),
        children: () => jsx("Text", { children: "on" }),
      });
      expect(slot.branch.handle.children).toEqual(["off"]);
      flag.set(true);
      expect(slot.branch.handle.children).toEqual(["on"]);
      flag.set(false);
      expect(slot.branch.handle.children).toEqual(["off"]);
      dispose();
    });
  });
});

describe("For", () => {
  const row = (item) => jsx("Text", { children: `row ${item.id}` });

  test("reconciles by referential identity without `by`", () => {
    const a = { id: 1 };
    const b = { id: 2 };
    const c = { id: 3 };
    const d = { id: 4 };
    const items = createSignal([a, b, c]);
    createRoot((dispose) => {
      const slot = jsx(For, { each: items, children: (item) => row(item) });
      const before = [...slot.entries];
      items.set([c, a, d]);
      expect(slot.entries[0].handle).toBe(before[2].handle);
      expect(slot.entries[1].handle).toBe(before[0].handle);
      expect(slot.entries[2].item).toBe(d);
      expect(host.calls).toContainEqual(["dispose-item", b]);
      dispose();
    });
  });

  test("reconciles by `by` and keeps an index accessor current", () => {
    const a = { id: "a" };
    const b = { id: "b" };
    const items = createSignal([a, b]);
    createRoot((dispose) => {
      const slot = jsx(For, {
        each: items,
        by: (item) => item.id,
        children: (item) => row(item),
      });
      const before = [...slot.entries];
      items.set([{ id: "b" }, { id: "a" }]);
      // Keys retained across fresh item objects; only positions move.
      expect(slot.entries[0].handle).toBe(before[1].handle);
      expect(slot.entries[1].handle).toBe(before[0].handle);
      expect(slot.entries[0].index()).toBe(0);
      expect(slot.entries[1].index()).toBe(1);
      dispose();
    });
  });

  test("an `each` getter reaches the host as an accessor and stays live", () => {
    const items = createSignal([{ id: 1 }]);
    createRoot((dispose) => {
      const slot = jsx(For, {
        get each() {
          return items();
        },
        children: (item) => row(item),
      });
      expect(slot.entries).toHaveLength(1);
      items.set([{ id: 1 }, { id: 2 }]);
      expect(slot.entries).toHaveLength(2);
      dispose();
    });
  });

  test("a non-function child fails loudly", () => {
    expect(() =>
      createRoot(() => jsx(For, { each: [], children: "nope" })),
    ).toThrow(/function child/);
  });
});

describe("Suspense", () => {
  test("mounts its children branch", () => {
    createRoot((dispose) => {
      const slot = jsx(Suspense, {
        fallback: () => jsx("Text", { children: "loading" }),
        children: () => jsx("Text", { children: "ready" }),
      });
      expect(slot.branch.handle.children).toEqual(["ready"]);
      dispose();
    });
  });

  test("presents the fallback while pending, then the children", () => {
    const log = [];
    createRoot((dispose) => {
      host.setPending(true);
      const slot = jsx(Suspense, {
        fallback: () => {
          onCleanup(() => log.push("fallback disposed"));
          return jsx("Text", { children: "loading" });
        },
        children: () => jsx("Text", { children: "ready" }),
      });
      expect(slot.branch.handle.children).toEqual(["loading"]);
      host.setPending(false);
      expect(slot.branch.handle.children).toEqual(["ready"]);
      expect(log).toEqual(["fallback disposed"]);
      dispose();
    });
  });
});

describe("Box", () => {
  test("applies its modifiers to the single child and creates no node", () => {
    const child = jsx("Text", { children: "hi" });
    const before = host.calls.length;
    const result = createRoot(() =>
      jsx(Box, { children: child, padding: 8, background: "red" }),
    );
    expect(result).toBe(child);
    expect(child.modifiers).toEqual([
      ["padding", 8],
      ["background", "red"],
    ]);
    expect(host.calls.slice(before).every(([kind]) => kind === "modify")).toBe(true);
  });

  test("rejects more than one child", () => {
    const a = jsx("Text", {});
    const b = jsx("Text", {});
    expect(() => createRoot(() => jsx(Box, { children: [a, b] }))).toThrow(
      /exactly one child.*2/,
    );
  });

  test("rejects a missing child and non-modifier attributes", () => {
    expect(() => createRoot(() => jsx(Box, {}))).toThrow(/exactly one child/);
    const child = jsx("Text", {});
    expect(() =>
      createRoot(() => jsx(Box, { children: child, gap: 4 })),
    ).toThrow(/"gap"/);
  });

  test("rejects children that are not host elements", () => {
    for (const child of ["text", 0, false]) {
      expect(() =>
        createRoot(() => jsx(Box, { children: child })),
      ).toThrow(/<Box>/);
    }
  });

  test("a spread carrying a modifier throws, naming attribute and element", () => {
    const child = jsx("Text", {});
    expect(() =>
      createRoot(() => jsx(Box, spreadProps({ children: child, padding: 8 }))),
    ).toThrow(/"padding".*<Box>|<Box>.*"padding"/);
  });
});
