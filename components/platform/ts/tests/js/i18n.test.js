import { describe, expect, test } from "bun:test";
import { t } from "../../src/js/index.js";

describe("t", () => {
  test("tagged template without holes produces { key }", () => {
    expect(t`Settings`).toEqual({ key: "Settings" });
  });

  test("tagged template with holes produces indexed slots and preserves values", () => {
    const orderId = () => 42;
    const result = t`Order ${orderId} of ${100}`;
    expect(result.key).toBe("Order {0} of {1}");
    expect(result.args["0"]).toBe(orderId);
    expect(result.args["1"]).toBe(100);
  });

  test("function form produces { key } or { key, args } with named arguments", () => {
    expect(t("greeting")).toEqual({ key: "greeting" });
    const name = () => "Ada";
    const result = t("Hello, {name}", { name });
    expect(result.key).toBe("Hello, {name}");
    expect(result.args.name).toBe(name);
  });

  test("rejects malformed arguments", () => {
    expect(() => t(123)).toThrow(TypeError);
    expect(() => t([])).toThrow(TypeError);
    expect(() => t("greeting", null)).toThrow(TypeError);
    expect(() => t("greeting", ["a"])).toThrow(TypeError);
    expect(() => t("greeting", {}, {})).toThrow(TypeError);
  });
});
