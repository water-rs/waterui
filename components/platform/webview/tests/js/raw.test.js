// The raw evaluation wrapper every backend's `run_javascript` evaluates.
//
// `src/js/raw.js` is a function expression `script::raw_evaluation` inlines
// around the caller's expression as `(<wrapper>)((\n<expression>\n))`; `evaluate`
// below runs exactly that program, so these cases pin what a real engine answers
// before any backend touches the reply.

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const WRAPPER = readFileSync(new URL("../../src/js/raw.js", import.meta.url), "utf8");

/// Evaluates `expression` the way a backend does, in a fresh global scope.
const evaluate = (expression) =>
  vm.runInNewContext(`(${WRAPPER})((\n${expression}\n))`);

describe("raw evaluation", () => {
  test("answers the JSON encoding of the value as a string", () => {
    expect(evaluate("'waterui'")).toBe('"waterui"');
    expect(evaluate("40 + 2")).toBe("42");
    expect(evaluate("true")).toBe("true");
    expect(evaluate("null")).toBe("null");
    expect(JSON.parse(evaluate("({name: 'waterui', items: [1, 2]})"))).toEqual({
      name: "waterui",
      items: [1, 2],
    });
  });

  test("answers null for the values JSON has no spelling for", () => {
    expect(evaluate("undefined")).toBe("null");
    expect(evaluate("() => 1")).toBe("null");
    expect(evaluate("Symbol('s')")).toBe("null");
    expect(evaluate("NaN")).toBe("null");
  });

  test("keeps a comma expression one value and a trailing comment out of the call", () => {
    expect(evaluate("1, 2")).toBe("2");
    expect(evaluate("'a' // a note")).toBe('"a"');
  });

  test("does not await a promise", () => {
    expect(evaluate("Promise.resolve(1)")).toBe("{}");
  });

  test("lets the expression's exception propagate", () => {
    expect(() => evaluate("(() => { throw new Error('boom'); })()")).toThrow("boom");
  });

  test("throws on a value JSON.stringify rejects", () => {
    expect(() => evaluate("(() => { const a = {}; a.self = a; return a; })()")).toThrow();
    expect(() => evaluate("10n")).toThrow();
  });

  test("refuses a statement, which is not an expression", () => {
    expect(() => evaluate("let x = 1")).toThrow(SyntaxError);
    expect(() => evaluate("document.title;")).toThrow(SyntaxError);
  });
});
