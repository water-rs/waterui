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

import { beforeEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { createMemo, createSignal, isSignal } from "../../src/js/signals.js";
import {
  getHost,
  installHost,
  isAccessor,
  read,
  subscribe,
  toAccessor,
  toSignal,
  uninstallHost,
  write,
} from "../../src/js/host.js";
import { mount, useLocale, useTheme } from "../../src/js/contexts.js";
import { makeCallback } from "../../src/js/runtime-global.js";
import { createFakeHost } from "./fake-host.js";

const FIXTURE = readFileSync(new URL("../fixtures/runtime.js", import.meta.url), "utf8");

/** Evaluates the fixture as the classic script the engine evaluates. */
function loadFixture() {
  const indirect = eval;
  indirect(FIXTURE);
  return globalThis.__waterui_runtime;
}

/**
 * The two implementations of the entries these scenarios use.
 *
 * `useTheme`, `useLocale` and `getHost` are not runtime-global entries — a
 * module imports the first two from the library and `getHost` is internal, so
 * the fixture's stand-ins are read off `globalThis.fixture` and put beside the
 * rest here.
 */
const FIXTURE_RUNTIME = loadFixture();
const RUNTIMES = [
  [
    "the fixture",
    {
      ...FIXTURE_RUNTIME,
      useTheme: globalThis.fixture.useTheme,
      useLocale: globalThis.fixture.useLocale,
      getHost: globalThis.fixture.getHost,
    },
  ],
  [
    "the library",
    {
      createSignal,
      createMemo,
      isSignal,
      isAccessor,
      read,
      write,
      subscribe,
      toSignal,
      toAccessor,
      installHost,
      uninstallHost,
      getHost,
      makeCallback,
      mount,
      useTheme,
      useLocale,
    },
  ],
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

// Both hosts are module state, and a sibling suite in the same bun process
// may have left one installed, so each scenario starts from nothing —
// the same discipline the other suites here follow.
beforeEach(() => {
  for (const [, runtime] of RUNTIMES) {
    runtime.uninstallHost();
  }
});

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

  test("a read-only host value becomes a signal that holds its reading", () => {
    const seen = agreed((runtime) => {
      const signal = runtime.toSignal({ read: () => 41 });
      return { value: runtime.read(signal), isSignal: runtime.isSignal(signal) };
    });

    expect(seen).toEqual({ value: 41, isSignal: true });
  });

  test("a plain thunk is followed without being told what it reads", () => {
    const seen = agreed((runtime) => {
      const source = runtime.createSignal(2);
      const doubled = () => source() * 2;
      const values = [];
      const dispose = runtime.subscribe(doubled, (value) => {
        values.push(value);
      });
      runtime.write(source, 3);
      dispose();
      runtime.write(source, 4);
      return { values, current: doubled() };
    });

    expect(seen).toEqual({ values: [6], current: 8 });
  });

  test("a thunk materialized as a signal keeps recomputing", () => {
    const seen = agreed((runtime) => {
      const source = runtime.createSignal(1);
      const derived = runtime.toSignal(() => source() + 1);
      const before = runtime.read(derived);
      runtime.write(source, 5);
      return { before, after: runtime.read(derived) };
    });

    expect(seen).toEqual({ before: 2, after: 6 });
  });

  test("a write settles against the target's own comparator", () => {
    const seen = agreed((runtime) => {
      const byId = (a, b) => a.id === b.id;
      const signal = runtime.createSignal({ id: 1, label: "a" }, { equals: byId });
      let notified = 0;
      runtime.subscribe(signal, () => {
        notified += 1;
      });
      // Structurally different, so the seam does write it; equal by the
      // signal's own comparator, so nothing changed and the write stood.
      const sameId = runtime.write(signal, { id: 1, label: "b" });
      const otherId = runtime.write(signal, { id: 2, label: "b" });
      return { sameId, otherId, notified, label: runtime.read(signal).label };
    });

    expect(seen).toEqual({ sameId: true, otherId: true, notified: 1, label: "b" });
  });

  test("mounting materializes the host environment and releases it on dispose", () => {
    const seen = agreed((runtime) => {
      let themeValue = "light";
      const subscribers = new Set();
      const theme = {
        read: () => themeValue,
        subscribe(callback) {
          subscribers.add(callback);
          return () => subscribers.delete(callback);
        },
      };
      const push = (value) => {
        themeValue = value;
        for (const subscriber of [...subscribers]) {
          subscriber(value);
        }
      };
      const host = createFakeHost();
      let environmentCalls = 0;
      host.environment = () => {
        environmentCalls += 1;
        return { theme, locale: "en-GB" };
      };
      runtime.installHost(host);
      let mountedTheme;
      let mountedLocale;
      const mounted = runtime.mount(() => {
        mountedTheme = runtime.useTheme();
        mountedLocale = runtime.useLocale();
        return "handle";
      });
      const before = runtime.read(mountedTheme);
      push("dark");
      const after = runtime.read(mountedTheme);
      const locale = runtime.read(mountedLocale);
      mounted.dispose();
      push("sepia");
      const afterDispose = runtime.read(mountedTheme);
      runtime.uninstallHost();
      return {
        environmentCalls,
        handle: mounted.handle,
        before,
        after,
        locale,
        afterDispose,
        subscribersLeft: subscribers.size,
      };
    });

    expect(seen).toEqual({
      environmentCalls: 1,
      handle: "handle",
      before: "light",
      after: "dark",
      locale: "en-GB",
      afterDispose: "dark",
      subscribersLeft: 0,
    });
  });

  test("a value equal to the one held is not written, a different one is", () => {
    const seen = agreed((runtime) => {
      const signal = runtime.createSignal({ items: [1, 2], label: "a" });
      let notified = 0;
      runtime.subscribe(signal, () => {
        notified += 1;
      });
      // A fresh object with the same contents: what every payload crossing
      // the seam looks like.
      const copy = runtime.write(signal, { items: [1, 2], label: "a" });
      const afterCopy = notified;
      const change = runtime.write(signal, { items: [1, 3], label: "a" });
      return { copy, change, afterCopy, notified, label: runtime.read(signal).label };
    });

    expect(seen).toEqual({
      copy: true,
      change: true,
      afterCopy: 0,
      notified: 1,
      label: "a",
    });
  });

  test("an array with a value where the held one has a hole is a change", () => {
    const seen = agreed((runtime) => {
      const sparse = [1, , 3];
      const signal = runtime.createSignal(sparse);
      let notified = 0;
      runtime.subscribe(signal, () => {
        notified += 1;
      });
      const stood = runtime.write(signal, [1, 9, 3]);
      const held = runtime.read(signal);
      return { stood, notified, filled: 1 in held, value: held[1] };
    });

    expect(seen).toEqual({ stood: true, notified: 1, filled: true, value: 9 });
  });

  test("the same entries in another order are a change", () => {
    const seen = agreed((runtime) => {
      const signal = runtime.createSignal({ a: 1, b: 2 });
      let notified = 0;
      runtime.subscribe(signal, () => {
        notified += 1;
      });
      const stood = runtime.write(signal, { b: 2, a: 1 });
      return { stood, notified, keys: Object.keys(runtime.read(signal)) };
    });

    expect(seen).toEqual({ stood: true, notified: 1, keys: ["b", "a"] });
  });

  test("a value written as a handle is compared by identity", () => {
    const seen = agreed((runtime) => {
      const held = { tag: 1 };
      const signal = runtime.createSignal(held);
      let notified = 0;
      runtime.subscribe(signal, () => {
        notified += 1;
      });
      const lookalike = { tag: 1 };
      const stood = runtime.write(signal, lookalike, true);
      const received = runtime.read(signal) === lookalike;
      const again = runtime.write(signal, lookalike, true);
      return { stood, again, received, notified };
    });

    expect(seen).toEqual({ stood: true, again: true, received: true, notified: 1 });
  });

  test("a difference deeper than the comparison looks is written", () => {
    const seen = agreed((runtime) => {
      const nest = (leaf) => {
        let value = leaf;
        for (let depth = 0; depth < 200; depth += 1) {
          value = { inner: value };
        }
        return value;
      };
      const signal = runtime.createSignal(nest(1));
      let notified = 0;
      runtime.subscribe(signal, () => {
        notified += 1;
      });
      const deeper = nest(1);
      const stood = runtime.write(signal, deeper);
      return { stood, notified, received: runtime.read(signal) === deeper };
    });

    // Past the bound the two are reported different and the write goes
    // through, which is the safe answer: JavaScript ends up holding what the
    // native side sent.
    expect(seen).toEqual({ stood: true, notified: 1, received: true });
  });

  test("a host table missing an entry is refused where it is installed", () => {
    const seen = agreed((runtime) => {
      const host = createFakeHost();
      const { invoke, ...withoutInvoke } = host;
      let message = null;
      try {
        runtime.installHost(withoutInvoke);
      } catch (error) {
        message = error.message;
      }
      const installed = (() => {
        try {
          runtime.getHost();
          return true;
        } catch {
          return false;
        }
      })();
      return { message, installed };
    });

    expect(seen.message).toMatch(/invoke/);
    expect(seen.installed).toBe(false);
  });

  test("the modifier names cross as an array and become a set", () => {
    const seen = agreed((runtime) => {
      const host = createFakeHost();
      runtime.installHost({ ...host, modifiers: ["padding", "background"] });
      const { modifiers } = runtime.getHost();
      const observed = {
        isSet: modifiers instanceof Set,
        has: modifiers.has("padding"),
        size: modifiers.size,
      };
      runtime.uninstallHost();
      return observed;
    });

    expect(seen).toEqual({ isSet: true, has: true, size: 2 });
  });

  test("a callback wrapper dispatches through the installed invoke", () => {
    const seen = agreed((runtime) => {
      const host = createFakeHost();
      runtime.installHost(host);
      const callback = runtime.makeCallback(7);
      const answer = callback("a", 2);
      // Reassigning the entry afterwards must not divert the wrapper: it
      // holds the function the table was installed with.
      host.invoke = () => "diverted";
      const again = callback("b");
      runtime.uninstallHost();
      return { answer, again, invocations: host.invocations };
    });

    expect(seen).toEqual({
      answer: "invoked",
      again: "invoked",
      invocations: [
        [7, "a", 2],
        [7, "b"],
      ],
    });
  });

  test("a host value that reads and writes is written through", () => {
    const seen = agreed((runtime) => {
      let held = 1;
      let writes = 0;
      const cell = {
        read: () => held,
        write(value) {
          writes += 1;
          held = value;
        },
      };
      const changed = runtime.write(cell, 2);
      const again = runtime.write(cell, 2);
      return { changed, again, held, writes, isAccessor: runtime.isAccessor(cell) };
    });

    expect(seen).toEqual({ changed: true, again: true, held: 2, writes: 1, isAccessor: true });
  });

  test("a host value that announces its changes is followed and released", () => {
    const seen = agreed((runtime) => {
      let held = "start";
      const subscribers = new Set();
      const source = {
        read: () => held,
        subscribe(callback) {
          subscribers.add(callback);
          return () => subscribers.delete(callback);
        },
      };
      const push = (value) => {
        held = value;
        for (const subscriber of [...subscribers]) {
          subscriber(value);
        }
      };
      const values = [];
      const dispose = runtime.subscribe(source, (value) => values.push(value));
      push("next");
      dispose();
      push("after");
      return { values, left: subscribers.size, read: runtime.read(source) };
    });

    expect(seen).toEqual({ values: ["next"], left: 0, read: "after" });
  });

  test("toAccessor lifts a constant, a thunk and a host value alike", () => {
    const seen = agreed((runtime) => {
      const constant = runtime.toAccessor(3);
      const thunk = runtime.toAccessor(() => 4);
      const hostValue = runtime.toAccessor({ read: () => 5 });
      return [constant(), thunk(), hostValue()];
    });

    expect(seen).toEqual([3, 4, 5]);
  });

  test("a memo evaluates once, and again only after what it read changed", () => {
    const seen = agreed((runtime) => {
      const source = runtime.createSignal(1);
      const unrelated = runtime.createSignal("x");
      let computations = 0;
      const memo = runtime.createMemo(() => {
        computations += 1;
        return source() + 1;
      });
      const first = memo();
      const second = memo();
      const afterCreation = computations;
      runtime.write(unrelated, "y");
      const afterUnrelated = memo();
      const untouched = computations;
      runtime.write(source, 5);
      const afterChange = memo();
      return {
        first,
        second,
        afterCreation,
        afterUnrelated,
        untouched,
        afterChange,
        computations,
      };
    });

    expect(seen).toEqual({
      first: 2,
      second: 2,
      afterCreation: 1,
      afterUnrelated: 2,
      untouched: 1,
      afterChange: 6,
      computations: 2,
    });
  });
});
