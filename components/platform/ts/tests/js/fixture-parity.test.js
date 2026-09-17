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
import { createSignal, isSignal } from "../../src/js/signals.js";
import {
  installHost,
  read,
  subscribe,
  toSignal,
  uninstallHost,
  write,
} from "../../src/js/host.js";
import { mount, useLocale, useTheme } from "../../src/js/contexts.js";
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
 * `useTheme` / `useLocale` are not runtime-global entries — a module imports
 * them from the library and the bridge never calls them — so the fixture's
 * stand-ins are read off `globalThis.fixture` and put beside the rest here.
 */
const FIXTURE_RUNTIME = loadFixture();
const RUNTIMES = [
  [
    "the fixture",
    {
      ...FIXTURE_RUNTIME,
      useTheme: globalThis.fixture.useTheme,
      useLocale: globalThis.fixture.useLocale,
    },
  ],
  [
    "the library",
    {
      createSignal,
      isSignal,
      read,
      write,
      subscribe,
      toSignal,
      installHost,
      uninstallHost,
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
});
