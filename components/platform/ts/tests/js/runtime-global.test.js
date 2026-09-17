// The seam the Rust bridge reads: the runtime global a bundle publishes, and
// the host table it installs through it.

import { afterEach, describe, expect, test } from "bun:test";
import { installRuntimeGlobal, makeCallback } from "../../src/js/runtime-global.js";
import { getHost, installHost, uninstallHost } from "../../src/js/host.js";
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
  test("routes the call to __waterui_host.invoke with the id first", () => {
    const seen = [];
    globalThis.__waterui_host = {
      invoke(...args) {
        seen.push(args);
        return "answered";
      },
    };

    const callback = makeCallback(7);
    expect(callback(1, "two")).toBe("answered");
    expect(seen).toEqual([[7, 1, "two"]]);
  });

  test("names the missing entry when the bridge registered nothing", () => {
    const callback = makeCallback(1);
    expect(() => callback()).toThrow(/__waterui_host\.invoke/);
  });

  test("refuses an id that is not the number the bridge assigns", () => {
    expect(() => makeCallback("1")).toThrow(/numeric id/);
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
