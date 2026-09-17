// A reference host for the unit suite.
//
// It implements the HOST.md contract in plain JS over the real signal
// library: `show` and `each` do the same branch/identity bookkeeping the Rust
// host table must do, `suspense` presents the children branch the way the Rust
// table does, and every entry point records its calls so tests can assert on
// order, reuse, and disposal. `modifiers` is the fake's own catalog subset —
// the real set comes from the generated component catalog.
//
// It is a reference host, so where it differs from the Rust table it is wrong:
// a test that passes here and fails against the real one has proved nothing.
// That is why `each` demands a `by` for a non-primitive item and `suspense`
// has no pending state — see both below.

import { createEffect, createRoot, createSignal } from "../../src/js/signals.js";
import { toSignal } from "../../src/js/host.js";

// 2^63: a whole number strictly inside ±2^63 is exactly an i64, and one
// outside it is not. `ItemKey::of` keys the first kind as an integer and the
// second by its bits, so 1e21 and 2e21 are two keys rather than one saturated
// integer — and 1 and 1n are one key, because both are that same integer.
const INTEGER_BOUND = 9223372036854775808;
const INTEGER_BOUND_BIG = 9223372036854775808n;

// One scratch view, because reading a float's bits is how two numbers that are
// not the same number are told apart — including two NaNs, which `===` and a
// `Set` both call equal.
const FLOAT_BITS = new DataView(new ArrayBuffer(8));

// `waterui_ts::kind_of`, for the refusals below.
function kindOf(value) {
  if (value === null) {
    return "null";
  }
  switch (typeof value) {
    case "undefined":
      return "undefined";
    case "boolean":
      return "a boolean";
    case "number":
      return "a number";
    case "bigint":
      return "a bigint";
    case "string":
      return "a string";
    case "function":
      return "a function";
    default:
      return Array.isArray(value) ? "an array" : "an object";
  }
}

// The key one item carries, in the domain the native side reconciles in.
//
// This is `ItemKey::of` (src/ts/components/collection.rs) value for value, and
// it is spelled out rather than left to a `Set` because the two domains
// disagree in both directions: SameValueZero tells `1` from `1n`, which are
// one `Integer(1)` there, and calls two NaNs equal, which are two `Number`
// keys there whenever their payloads differ. A fake that reconciled by the raw
// value would accept lists the real host refuses, and refuse lists it accepts.
//
// `keyed` says whether the value came from `by`, which is the only thing that
// changes: what a `by` answered has to be a key, while an item that is not a
// primitive is asked for a `by` instead.
function itemKey(value, keyed) {
  switch (typeof value) {
    case "boolean":
      return `bool:${value}`;
    case "string":
      return `text:${value}`;
    case "bigint":
      if (value < -INTEGER_BOUND_BIG || value >= INTEGER_BOUND_BIG) {
        throw new TypeError("a key past the range of a 64-bit integer");
      }
      return `int:${value}`;
    case "number":
      if (Number.isInteger(value) && value >= -INTEGER_BOUND && value < INTEGER_BOUND) {
        return `int:${BigInt(value)}`;
      }
      FLOAT_BITS.setFloat64(0, value);
      return `number:${FLOAT_BITS.getBigUint64(0).toString(16)}`;
    default:
      throw new TypeError(
        keyed
          ? `<For by={…}> answered ${kindOf(value)}, which cannot be a key: return a string, ` +
            "a number or a boolean"
          : `<For each={…}> was given items of ${kindOf(value)}, whose identity cannot cross ` +
            "into the native side: an object crosses as data, so two references Rust sees are " +
            "two values. Give <For> a `by` that answers a stable key",
      );
  }
}

// A key as the duplicate-row error names it, which is `ItemKey`'s own Display:
// a string in backticks, anything else as it was written.
function displayKey(value) {
  return typeof value === "string" ? `\`${value}\`` : String(value);
}

export function createFakeHost(environment = {}) {
  const calls = [];
  const invocations = [];
  let nextId = 1;

  const host = {
    calls,

    modifiers: new Set(["padding", "background", "foreground", "frame", "font"]),

    create(component, config, children) {
      const handle = { id: nextId, type: "view", component, config, children };
      nextId += 1;
      calls.push(["create", component, config, children]);
      return handle;
    },

    modify(handle, name, value) {
      handle.modifiers = handle.modifiers ?? [];
      handle.modifiers.push([name, value]);
      calls.push(["modify", handle.id, name, value]);
      return handle;
    },

    text(content) {
      const handle = { id: nextId, type: "text", content };
      nextId += 1;
      calls.push(["text", content]);
      return handle;
    },

    show(when, render, fallback) {
      const slot = { id: nextId, type: "show", branch: null };
      nextId += 1;
      slot.dispose = createRoot((dispose) => {
        const condition = toSignal(when);
        let side;
        createEffect(() => {
          const truthy = Boolean(condition());
          if (truthy === side) {
            return;
          }
          side = truthy;
          slot.branch?.dispose();
          slot.branch = truthy ? render(condition) : fallback ? fallback() : null;
        });
        return dispose;
      });
      return slot;
    },

    each(each, render, by) {
      const slot = { id: nextId, type: "each", entries: [] };
      nextId += 1;
      // An item identifies itself only when it is a primitive. Across the
      // engine seam an object arrives as a copy, so the Rust table has no
      // referential identity to reconcile by and demands `by`; a fake that
      // reconciled objects would make a test pass that the real host refuses.
      // What `by` answers is held to the same domain, because the key it
      // returns is the key that crosses.
      const keyed = by !== undefined && by !== null;
      const keyOf = keyed ? by : (item) => item;
      slot.dispose = createRoot((dispose) => {
        const list = toSignal(each);
        createEffect(() => {
          const items = list() ?? [];
          const previous = slot.entries;
          const byKey = new Map(previous.map((entry) => [entry.key, entry]));
          const seen = new Set();
          const next = items.map((item, index) => {
            const answered = keyOf(item);
            const key = itemKey(answered, keyed);
            // Two rows sharing one key is a list that cannot be reconciled, and
            // the Rust table refuses it by name. A fake that accepted it would
            // silently collapse the two and make a test pass that the real host
            // fails.
            if (seen.has(key)) {
              throw new TypeError(
                `two items of this <For> have the key ${displayKey(answered)}: give \`by\` ` +
                  "something unique, or make the items themselves distinct",
              );
            }
            seen.add(key);
            const kept = byKey.get(key);
            if (kept !== undefined && !kept.used) {
              kept.used = true;
              kept.index.set(index);
              return kept;
            }
            const position = createSignal(index);
            const inner = render(item, position);
            return {
              key,
              item,
              index: position,
              handle: inner.handle,
              dispose: () => {
                inner.dispose();
                // The key as the author wrote it, not as it is normalized:
                // what a test asserts on is the value it put in the list.
                calls.push(["dispose-item", answered]);
              },
            };
          });
          for (const entry of previous) {
            if (!entry.used) {
              entry.dispose();
            }
          }
          for (const entry of next) {
            delete entry.used;
          }
          slot.entries = next;
        });
        return dispose;
      });
      return slot;
    },

    // The boundary presents its children. There is no resource primitive in
    // the JavaScript runtime, so nothing a `children()` branch builds out of
    // JSX alone can be pending and the fallback is never built — which is
    // exactly what the Rust table does.
    suspense(children, _fallback) {
      const slot = { id: nextId, type: "suspense", branch: null };
      nextId += 1;
      slot.dispose = createRoot((dispose) => {
        slot.branch = children();
        return dispose;
      });
      return slot;
    },

    environment() {
      return environment;
    },

    // The bridge's own entry: `makeCallback` wraps this one. Recorded rather
    // than dispatched, since there is no Rust registry behind a fake host.
    invoke(id, ...args) {
      invocations.push([id, ...args]);
      calls.push(["invoke", id, ...args]);
      return "invoked";
    },

    invocations,
  };

  return host;
}
