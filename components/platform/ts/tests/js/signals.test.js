// The signal library's contract with the transform and the host: Solid
// semantics — automatic tracking, glitch-free propagation, owner-scoped
// lifetimes — in a package small enough to inject into every TS bundle.

import { describe, expect, test } from "bun:test";
import {
  batch,
  createContext,
  createEffect,
  createMemo,
  createRoot,
  createScope,
  createSignal,
  createStore,
  getOwner,
  isAccessor,
  isSignal,
  onCleanup,
  runWithOwner,
  untrack,
  useContext,
} from "../../src/js/signals.js";

describe("signal shape", () => {
  test("is callable and iterable at once", () => {
    const count = createSignal(0);
    expect(count()).toBe(0);
    count.set(1);
    expect(count()).toBe(1);
    count.update((v) => v + 1);
    expect(count()).toBe(2);

    const [read, write] = createSignal(10);
    expect(read()).toBe(10);
    write(20);
    write((v) => v + 1);
    expect(read()).toBe(21);
    expect(isSignal(count)).toBe(true);
    expect(isAccessor(count)).toBe(true);
  });

  test("map derives a memo", () => {
    const count = createSignal(2);
    const doubled = count.map((v) => v * 2);
    expect(doubled()).toBe(4);
    count.set(5);
    expect(doubled()).toBe(10);
  });

  test("honours equals: a comparator, and false for always-notify", () => {
    const list = createSignal([1], { equals: (a, b) => a[0] === b[0] });
    let runs = 0;
    createRoot((dispose) => {
      createEffect(() => {
        list();
        runs += 1;
      });
      list.set([1]);
      expect(runs).toBe(1);
      list.set([2]);
      expect(runs).toBe(2);
      dispose();
    });

    const noisy = createSignal(0, { equals: false });
    let noisyRuns = 0;
    createRoot((dispose) => {
      createEffect(() => {
        noisy();
        noisyRuns += 1;
      });
      noisy.set(0);
      expect(noisyRuns).toBe(2);
      dispose();
    });
  });
});

describe("propagation", () => {
  test("a diamond dependency evaluates its sink once per change", () => {
    const a = createSignal(1);
    const b = createMemo(() => a() * 2);
    const c = createMemo(() => a() + 10);
    let runs = 0;
    let observed;
    createRoot((dispose) => {
      createEffect(() => {
        observed = [b(), c()];
        runs += 1;
      });
      a.set(2);
      expect(runs).toBe(2);
      expect(observed).toEqual([4, 12]);
      a.set(3);
      expect(runs).toBe(3);
      expect(observed).toEqual([6, 13]);
      dispose();
    });
  });

  test("a memo that recomputes to an equal value does not disturb its observers", () => {
    const source = createSignal(0);
    const parity = createMemo(() => source() % 2);
    let runs = 0;
    createRoot((dispose) => {
      createEffect(() => {
        parity();
        runs += 1;
      });
      source.set(2);
      expect(runs).toBe(1);
      source.set(3);
      expect(runs).toBe(2);
      dispose();
    });
  });

  test("batch coalesces writes into one propagation", () => {
    const a = createSignal(0);
    const b = createSignal(0);
    let runs = 0;
    createRoot((dispose) => {
      createEffect(() => {
        a();
        b();
        runs += 1;
      });
      batch(() => {
        a.set(1);
        b.set(1);
      });
      expect(runs).toBe(2);
      dispose();
    });
  });

  test("untrack reads without subscribing", () => {
    const tracked = createSignal(0);
    const untracked = createSignal(0);
    let runs = 0;
    let last;
    createRoot((dispose) => {
      createEffect(() => {
        last = [tracked(), untrack(() => untracked())];
        runs += 1;
      });
      untracked.set(1);
      expect(runs).toBe(1);
      tracked.set(1);
      expect(runs).toBe(2);
      expect(last).toEqual([1, 1]);
      dispose();
    });
  });

  test("an effect that stops reading a source stops depending on it", () => {
    const flag = createSignal(true);
    const value = createSignal(0);
    let runs = 0;
    createRoot((dispose) => {
      createEffect(() => {
        if (flag()) {
          value();
        }
        runs += 1;
      });
      flag.set(false);
      const afterSwitch = runs;
      value.set(1);
      expect(runs).toBe(afterSwitch);
      dispose();
    });
  });
});

describe("ownership", () => {
  test("cleanups run before re-evaluation and on dispose, children first", () => {
    const source = createSignal(0);
    const log = [];
    createRoot((dispose) => {
      createEffect(() => {
        const value = source();
        log.push(`run ${value}`);
        createScope((innerDispose) => {
          onCleanup(() => log.push(`inner cleanup ${value}`));
          return innerDispose;
        });
        onCleanup(() => log.push(`cleanup ${value}`));
      });
      source.set(1);
      dispose();
      expect(log).toEqual([
        "run 0",
        "inner cleanup 0",
        "cleanup 0",
        "run 1",
        "inner cleanup 1",
        "cleanup 1",
      ]);
    });
  });

  test("dispose tears the scope's effects down", () => {
    const source = createSignal(0);
    let runs = 0;
    const dispose = createRoot((d) => {
      createEffect(() => {
        source();
        runs += 1;
      });
      return d;
    });
    source.set(1);
    expect(runs).toBe(2);
    dispose();
    source.set(2);
    expect(runs).toBe(2);
  });

  test("onCleanup outside an owner fails loudly", () => {
    expect(() => onCleanup(() => {})).toThrow(/reactive owner/);
  });

  test("runWithOwner restores the owner for the duration of the call", () => {
    const context = createContext("fallback");
    createRoot(() => {
      const owner = getOwner();
      owner.contexts = new Map([[context.id, "provided"]]);
      const seen = runWithOwner(owner, () => useContext(context));
      expect(seen).toBe("provided");
    });
    expect(useContext(context)).toBe("fallback");
  });
});

describe("contexts", () => {
  test("resolve through the owner chain and fall back to the default", () => {
    const context = createContext("default");
    createRoot((dispose) => {
      expect(useContext(context)).toBe("default");
      context.Provider({
        value: "outer",
        children: () => {
          expect(useContext(context)).toBe("outer");
          context.Provider({
            value: "inner",
            children: () => {
              expect(useContext(context)).toBe("inner");
              return null;
            },
          });
          expect(useContext(context)).toBe("outer");
          return null;
        },
      });
      dispose();
    });
  });
});

describe("stores", () => {
  test("track each property independently", () => {
    const [store, setStore] = createStore({ a: 1, b: 2 });
    let aRuns = 0;
    let bRuns = 0;
    createRoot((dispose) => {
      createEffect(() => {
        store.a;
        aRuns += 1;
      });
      createEffect(() => {
        store.b;
        bRuns += 1;
      });
      setStore("b", 20);
      expect(aRuns).toBe(1);
      expect(bRuns).toBe(2);
      setStore("a", 10);
      expect(aRuns).toBe(2);
      expect(bRuns).toBe(2);
      dispose();
    });
  });

  test("support path setters, updaters, and producers", () => {
    const [store, setStore] = createStore({ user: { name: "a", age: 1 }, items: [1] });
    setStore("user", "name", "b");
    expect(store.user.name).toBe("b");
    setStore("user", "age", (n) => n + 1);
    expect(store.user.age).toBe(2);
    setStore("items", (items) => {
      items.push(2);
    });
    expect([...store.items]).toEqual([1, 2]);
    setStore((root) => {
      root.user.name = "c";
    });
    expect(store.user.name).toBe("c");
  });

  test("nested writes only invalidate readers of that path", () => {
    const [store, setStore] = createStore({ user: { name: "a" }, other: 0 });
    let nameRuns = 0;
    let userRuns = 0;
    createRoot((dispose) => {
      createEffect(() => {
        store.user.name;
        nameRuns += 1;
      });
      createEffect(() => {
        store.other;
        userRuns += 1;
      });
      setStore("user", "name", "b");
      expect(nameRuns).toBe(2);
      expect(userRuns).toBe(1);
      dispose();
    });
  });

  test("key-set reads react to added and removed properties", () => {
    const [store, setStore] = createStore({ a: 1 });
    let seen;
    createRoot((dispose) => {
      createEffect(() => {
        seen = Object.keys(store).length;
      });
      setStore("b", 2);
      expect(seen).toBe(2);
      dispose();
    });
  });
});
