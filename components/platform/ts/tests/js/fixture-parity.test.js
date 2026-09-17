// The Rust tests drive a hand-written stand-in for the runtime global, because
// a Rust test cannot bundle: `tests/fixtures/runtime.js` publishes the same
// shape over a tiny push-based signal implementation.
//
// A stand-in that behaves differently from the library is worse than no test
// at all: it makes the Rust suite green over defects that only the real
// runtime would show — a `set` with no updater overload hid a write that
// called the value it was meant to store, and a `===` comparison hid the two
// values where `===` and SameValue disagree.
//
// So the two are pinned together here. Each scenario runs against the fixture
// and against the real `signals.js` / `host.js`, and the observations must
// match exactly.

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { createSignal } from "../../src/js/signals.js";
import { read, subscribe, write } from "../../src/js/host.js";

const FIXTURE = readFileSync(new URL("../fixtures/runtime.js", import.meta.url), "utf8");

/** Evaluates the fixture as the classic script the engine evaluates. */
function loadFixture() {
  const indirect = eval;
  indirect(FIXTURE);
  return globalThis.__waterui_runtime;
}

/** The two implementations of the four entries these scenarios use. */
const RUNTIMES = [
  ["the fixture", loadFixture()],
  ["the library", { createSignal, read, write, subscribe }],
];

/** Runs `scenario` against both and returns what each observed. */
function observations(scenario) {
  return RUNTIMES.map(([name, runtime]) => [name, scenario(runtime)]);
}

/** Asserts both implementations observed the same thing, and returns it. */
function agreed(scenario) {
  const [[, fixture], [, library]] = observations(scenario);
  expect(fixture).toEqual(library);
  return library;
}

describe("the Rust fixture and the library agree", () => {
  test("a write stores a function instead of calling it", () => {
    const seen = agreed((runtime) => {
      const calls = [];
      const probe = (...args) => {
        calls.push(args);
        return "called";
      };
      const signal = runtime.createSignal(1);
      const stood = runtime.write(signal, probe);
      return {
        stood,
        storedTheFunction: runtime.read(signal) === probe,
        calls: calls.length,
      };
    });

    expect(seen).toEqual({ stood: true, storedTheFunction: true, calls: 0 });
  });

  test("NaN written twice settles and propagates once", () => {
    const seen = agreed((runtime) => {
      const signal = runtime.createSignal(0);
      let notified = 0;
      runtime.subscribe(signal, () => {
        notified += 1;
      });
      const first = runtime.write(signal, Number.NaN);
      const second = runtime.write(signal, Number.NaN);
      return {
        first,
        second,
        isNaN: Number.isNaN(runtime.read(signal)),
        notified,
      };
    });

    expect(seen).toEqual({ first: true, second: true, isNaN: true, notified: 1 });
  });

  test("a negative zero written over a positive zero is a real change", () => {
    const seen = agreed((runtime) => {
      const signal = runtime.createSignal(0);
      let notified = 0;
      runtime.subscribe(signal, () => {
        notified += 1;
      });
      const stood = runtime.write(signal, -0);
      return { stood, isNegativeZero: Object.is(runtime.read(signal), -0), notified };
    });

    expect(seen).toEqual({ stood: true, isNegativeZero: true, notified: 1 });
  });

  test("an ordinary change propagates once and stands", () => {
    const seen = agreed((runtime) => {
      const signal = runtime.createSignal("start");
      const values = [];
      runtime.subscribe(signal, (value) => {
        values.push(value);
      });
      const stood = runtime.write(signal, "next");
      const again = runtime.write(signal, "next");
      return { stood, again, values, current: runtime.read(signal) };
    });

    expect(seen).toEqual({
      stood: true,
      again: true,
      values: ["next"],
      current: "next",
    });
  });
});
