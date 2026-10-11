// The seam the Rust bridge reads: the runtime global a bundle publishes, and
// the host table it installs through it.

import { afterEach, describe, expect, test } from "bun:test";
import { installRuntimeGlobal, makeCallback } from "../../src/js/runtime-global.js";
import { getHost, installHost, read, uninstallHost, write } from "../../src/js/host.js";
import { createEffect, createRoot, createSignal } from "../../src/js/signals.js";
import { createFakeHost } from "./fake-host.js";

const REQUIRED_FUNCTIONS = [
  "installHost",
  "uninstallHost",
  "mount",
  "isSignal",
  "isAccessor",
  "read",
  "write",
  "subscribe",
  "toSignal",
  "toAccessor",
  "createSignal",
  "createMemo",
  "makeCallback",
];

afterEach(() => {
  uninstallHost();
  delete globalThis.__waterui_runtime;
  delete globalThis.__waterui_host;
});

describe("installRuntimeGlobal", () => {
  test("publishes every entry the bridge reads", () => {
    const modules = { "src/promo.tsx": () => null };
    const runtime = installRuntimeGlobal(modules);

    expect(globalThis.__waterui_runtime).toBe(runtime);
    for (const name of REQUIRED_FUNCTIONS) {
      expect(typeof runtime[name]).toBe("function");
    }
    expect(runtime.modules).toBe(modules);
  });

  test("rejects a module table that is not an object", () => {
    expect(() => installRuntimeGlobal(null)).toThrow(/module id/);
    expect(() => installRuntimeGlobal(undefined)).toThrow(/module id/);
  });

  test("exposes the library's own helpers, not copies", async () => {
    const host = await import("../../src/js/host.js");
    const signals = await import("../../src/js/signals.js");
    const runtime = installRuntimeGlobal({});

    expect(runtime.installHost).toBe(host.installHost);
    expect(runtime.subscribe).toBe(host.subscribe);
    expect(runtime.createSignal).toBe(signals.createSignal);
    expect(runtime.createMemo).toBe(signals.createMemo);
  });

  test("a second install replaces the table wholesale", () => {
    const first = installRuntimeGlobal({ a: 1 });
    const second = installRuntimeGlobal({ b: 2 });

    expect(globalThis.__waterui_runtime).toBe(second);
    expect(second).not.toBe(first);
  });
});

describe("makeCallback", () => {
  test("routes the call to the installed host's invoke, with the id first", () => {
    const table = createFakeHost();
    installHost(table);

    const callback = makeCallback(7);
    expect(callback(1, "two")).toBe("invoked");
    expect(table.invocations).toEqual([[7, 1, "two"]]);
  });

  test("says so when no host is installed", () => {
    expect(() => makeCallback(1)).toThrow(/no host installed/);
  });

  test("refuses an id that is not the number the bridge assigns", () => {
    expect(() => makeCallback("1")).toThrow(/numeric id/);
  });

  test("dispatches through the installed invoke, not the mutable global", () => {
    const table = createFakeHost();
    installHost(table);
    const callback = makeCallback(3);

    // A bundle reassigning the global — the entry the bridge registered is
    // what was installed, and what every wrapper must keep calling.
    globalThis.__waterui_host = {
      invoke() {
        throw new Error("the wrapper read the global");
      },
    };

    expect(callback("value")).toBe("invoked");
    expect(table.invocations).toEqual([[3, "value"]]);
  });
});

describe("installHost", () => {
  test("accepts the modifier names as an array and builds the set once", () => {
    const table = createFakeHost();
    table.modifiers = ["padding", "background"];
    installHost(table);

    const installed = getHost();
    expect(installed.modifiers).toBeInstanceOf(Set);
    expect(installed.modifiers.has("padding")).toBe(true);
    expect(installed.modifiers.has("frame")).toBe(false);
    expect(installed.create).toBe(table.create);
  });

  test("keeps a table that already carries a set", () => {
    const table = createFakeHost();
    installHost(table);
    expect(getHost()).toBe(table);
  });

  test("names the missing entry when modifiers are neither", () => {
    const table = createFakeHost();
    table.modifiers = "padding background";
    expect(() => installHost(table)).toThrow(/modifiers/);
  });
});

describe("write", () => {
  test("answers that the value stood", () => {
    const count = createSignal(1);
    expect(write(count, 4)).toBe(true);
    expect(read(count)).toBe(4);
  });

  test("answers that an effect changed it while the write settled", () => {
    createRoot(() => {
      const count = createSignal(1);
      createEffect(() => {
        if (count() > 10) {
          count.set(10);
        }
      });

      expect(write(count, 40)).toBe(false);
      expect(read(count)).toBe(10);
      expect(write(count, 5)).toBe(true);
    });
  });

  test("answers on identity, so a value carrying a signal is recognised", () => {
    const inner = createSignal(0);
    const box = createSignal({ inner });
    const next = { inner };

    expect(write(box, next)).toBe(true);
    expect(write(box, { inner })).toBe(true);
    expect(read(box).inner).toBe(inner);
  });

  test("refuses a read-only input rather than dropping the write", () => {
    const count = createSignal(1);
    expect(() => write(() => count(), 2)).toThrow(/signal/);
  });
});
