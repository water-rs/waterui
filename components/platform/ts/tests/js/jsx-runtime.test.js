// The element contract: evaluate once, route through the host, apply
// modifiers in written order, and refuse spread-carried modifiers.

import { beforeEach, describe, expect, test } from "bun:test";
import { installHost, uninstallHost } from "../../src/js/host.js";
import { Fragment, jsx, spreadProps } from "../../src/js/jsx-runtime.js";
import { createSignal } from "../../src/js/signals.js";
import { createFakeHost } from "./fake-host.js";

let host;
beforeEach(() => {
  uninstallHost();
  host = createFakeHost();
  installHost(host);
});

describe("jsx", () => {
  test("a function tag is invoked once with getter-props", () => {
    const count = createSignal(0);
    let invocations = 0;
    const props = {};
    Object.defineProperty(props, "title", {
      enumerable: true,
      get: () => count(),
    });
    const Component = (p) => {
      invocations += 1;
      return p;
    };
    const result = jsx(Component, props);
    expect(invocations).toBe(1);
    expect(result.title).toBe(0);
    count.set(1);
    // The getter stays live: props are read lazily, not copied.
    expect(result.title).toBe(1);
    expect(invocations).toBe(1);
  });

  test("a string tag creates a native view through the host", () => {
    const handle = jsx("VStack", { gap: 8 });
    expect(handle.component).toBe("VStack");
    expect(handle.config.gap).toBe(8);
    expect(host.calls[0][0]).toBe("create");
  });

  test("children normalize: nested arrays flatten, empties drop", () => {
    const inner = jsx("Text", { children: "leaf" });
    const handle = jsx("VStack", {
      children: [inner, null, false, ["a", ["b"]], undefined],
    });
    expect(handle.children).toEqual([inner, "a", "b"]);
  });

  test("a function child stays a reactive slot for the host", () => {
    const count = createSignal(0);
    const handle = jsx("Text", { children: () => `Count: ${count()}` });
    expect(typeof handle.children[0]).toBe("function");
  });

  test("label is configuration, not a modifier", () => {
    const label = jsx("Label", {});
    const handle = jsx("Button", { label, children: "Save" });
    expect(handle.config.label).toBe(label);
    expect(handle.children).toEqual(["Save"]);
    expect(handle.modifiers ?? []).toEqual([]);
  });

  test("modifier attributes reach host.modify in written order", () => {
    const handle = jsx("Text", {
      children: "hi",
      padding: 12,
      background: "red",
      foreground: "white",
    });
    const order = host.calls
      .filter(([kind]) => kind === "modify")
      .map(([, , name]) => name);
    expect(order).toEqual(["padding", "background", "foreground"]);
    expect(handle.modifiers).toEqual([
      ["padding", 12],
      ["background", "red"],
      ["foreground", "white"],
    ]);
  });

  test("modifier order changes the host calls, as documented", () => {
    jsx("Text", { padding: 12, background: "red" });
    jsx("Text", { background: "red", padding: 12 });
    const orders = host.calls
      .filter(([kind]) => kind === "modify")
      .map(([, , name]) => name);
    expect(orders).toEqual(["padding", "background", "background", "padding"]);
  });

  test("a spread carrying a modifier throws, naming attribute and element", () => {
    expect(() => jsx("Text", spreadProps({ padding: 12, title: "x" }))).toThrow(
      /"padding".*<Text>|<Text>.*"padding"/,
    );
  });

  test("a spread of pure configuration is fine", () => {
    const handle = jsx("VStack", spreadProps({ gap: 4 }));
    expect(handle.config.gap).toBe(4);
  });

  test("Fragment evaluates to its children", () => {
    const a = jsx("Text", {});
    expect(jsx(Fragment, { children: [a, "x"] })).toEqual([a, "x"]);
  });

  test("reactive values pass through to the host untouched", () => {
    const width = createSignal(10);
    const handle = jsx("Text", { children: "x", frame: () => width() });
    expect(handle.modifiers[0][0]).toBe("frame");
    expect(typeof handle.modifiers[0][1]).toBe("function");
  });
});
