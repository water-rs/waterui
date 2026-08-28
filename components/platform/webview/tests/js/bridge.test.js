// The half of the bridge that Rust tests cannot see.
//
// `bridge.js` and `state.js` are injected into every page by every backend, and
// nothing in the Rust suite executes them. Three separate total breaks shipped
// behind a green test run before this file existed:
//
//   * every reply crossed as base64, so `await waterui.invoke(...)` resolved to
//     a base64 string instead of the value the handler returned;
//   * `bridge.js` froze the `waterui` object and `state.js` then tried to add
//     `state` and `watch` to it, which throws — so mirrored state was
//     unreachable from a page for as long as the feature had existed;
//   * integers past 2^53 lost their low bits in both directions.
//
// Each of those is one assertion here. Run with `bun test` from the repository
// root; this is deliberately not wired into CI, so run it yourself when you
// touch anything under `src/js/`.

import { beforeEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";

const JS = new URL("../../src/js/", import.meta.url).pathname;
const source = (name) => readFileSync(JS + name, "utf8");

/// A page with the real scripts in it, and a spy on what reaches the native side.
function loadPage() {
  const sent = [];
  const context = {
    __wateruiSend: (envelope) => sent.push(JSON.parse(envelope)),
  };
  // The scripts install onto `globalThis`, so each page gets a fresh one.
  const run = new Function(
    "globalThis",
    `${source("bridge.js")}\n${source("state.js")}\nreturn globalThis;`,
  );
  const page = run(context);
  return { page, sent, waterui: page.waterui };
}

/// The reply a backend evaluates once a handler has answered. Its shape is
/// asserted on the Rust side in `bridge.rs`; this is the other end of it.
const resolve = (page, id, payload) => page.__wateruiResolve(id, true, payload);
const reject = (page, id, message) => page.__wateruiResolve(id, false, { message });

describe("invoke", () => {
  let page;
  let sent;
  let waterui;
  beforeEach(() => ({ page, sent, waterui } = loadPage()));

  test("resolves with the value a handler returned, not its base64", async () => {
    const reply = waterui.invoke("greet", { name: "Lexo" });
    resolve(page, sent[0].id, { json: { text: "Hi Lexo" } });
    expect(await reply).toEqual({ text: "Hi Lexo" });
  });

  test("resolves bytes as a Uint8Array", async () => {
    const reply = waterui.invoke("read", "/a.txt");
    resolve(page, sent[0].id, { b64: "AQID" });
    expect(await reply).toEqual(new Uint8Array([1, 2, 3]));
  });

  test("a handler with nothing to say resolves null, not an empty string", async () => {
    const reply = waterui.invoke("reset");
    resolve(page, sent[0].id, { json: null });
    expect(await reply).toBeNull();
  });

  test("a failure rejects with an Error carrying the message", async () => {
    const reply = waterui.invoke("save", {});
    reject(page, sent[0].id, "disk full");
    expect(reply).rejects.toThrow("disk full");
  });

  test("sends JSON payloads as JSON and binary ones as base64", () => {
    waterui.invoke("save", { a: 1 });
    expect(sent[0]).toMatchObject({ name: "save", json: { a: 1 } });
    expect(sent[0].b64).toBeUndefined();

    waterui.invoke("upload", new Uint8Array([0, 1, 2]));
    expect(sent[1].b64).toBe("AAEC");
    expect(sent[1].json).toBeUndefined();
  });

  test("gives each call its own id", () => {
    waterui.invoke("a");
    waterui.invoke("b");
    expect(sent[0].id).not.toBe(sent[1].id);
  });

  // Page script can reach `__wateruiResolve`, so a stray call must not surface
  // as an unhandled exception or settle somebody else's promise.
  test("ignores a reply for an id it is not waiting on", async () => {
    const reply = waterui.invoke("greet", {});
    expect(() => resolve(page, 9999, { json: 1 })).not.toThrow();
    resolve(page, sent[0].id, { json: "ok" });
    expect(await reply).toBe("ok");
  });

  test("ignores a second reply for the same call", async () => {
    const reply = waterui.invoke("greet", {});
    resolve(page, sent[0].id, { json: "first" });
    expect(() => resolve(page, sent[0].id, { json: "second" })).not.toThrow();
    expect(await reply).toBe("first");
  });
});

describe("mirrored state", () => {
  let page;
  let sent;
  let waterui;
  beforeEach(() => {
    ({ page, sent, waterui } = loadPage());
    page.__wateruiState.define("theme", "light", 0, true);
    page.__wateruiState.define("doubled", 4, 0, false);
  });

  // `bridge.js` used to freeze `waterui`, which made `state.js` throw while
  // installing these two — so both were absent from every page.
  test("is reachable at all", () => {
    expect(waterui.state).toBeDefined();
    expect(typeof waterui.watch).toBe("function");
  });

  test("reads locally without crossing the boundary", () => {
    expect(waterui.state.theme).toBe("light");
    expect(sent).toHaveLength(0);
  });

  test("a write updates locally and is sent on", () => {
    waterui.state.theme = "dark";
    expect(waterui.state.theme).toBe("dark");
    expect(sent[0]).toMatchObject({
      name: "__wateruiSetState",
      json: { key: "theme", value: "dark", epoch: 0 },
    });
  });

  test("a derived value refuses assignment", () => {
    expect(() => {
      waterui.state.doubled = 9;
    }).toThrow(TypeError);
  });

  // A typo that silently created a property is what the Proxy exists to catch.
  test("an unknown key is refused rather than invented", () => {
    expect(() => waterui.state.thmee).toThrow(ReferenceError);
    expect(() => {
      waterui.state.thmee = "dark";
    }).toThrow(ReferenceError);
    expect(() => waterui.watch("thmee", () => {})).toThrow(ReferenceError);
  });

  test("a patch from Rust updates the value and its epoch", () => {
    page.__wateruiState.apply({ theme: { v: "dark", e: 7 } });
    expect(waterui.state.theme).toBe("dark");
    waterui.state.theme = "sepia";
    expect(sent[0].json.epoch).toBe(7);
  });

  test("watchers fire on both a local write and a patch, until unsubscribed", () => {
    const seen = [];
    const stop = waterui.watch("theme", (value) => seen.push(value));
    waterui.state.theme = "dark";
    page.__wateruiState.apply({ theme: { v: "sepia", e: 1 } });
    stop();
    page.__wateruiState.apply({ theme: { v: "ignored", e: 2 } });
    expect(seen).toEqual(["dark", "sepia"]);
  });

  // A patch used to hand watchers the raw wire value while the mirror held the
  // revived one, so `waterui.state.id` was a BigInt and the watcher argument
  // was `{__wateruiBigInt: "…"}` — arithmetic in a watcher silently operated on
  // an object.
  test("a watcher receives the same revived value the mirror holds", () => {
    const BIG = "9007199254740993";
    page.__wateruiState.define("id", 1, 0, true);
    let observed;
    waterui.watch("id", (value) => {
      observed = value;
    });
    page.__wateruiState.apply({ id: { v: { __wateruiBigInt: BIG }, e: 1, w: true } });
    expect(observed).toBe(BigInt(BIG));
    expect(observed).toBe(waterui.state.id);
  });

  // A document that loaded while a navigation was already in flight can miss
  // the seed entirely. A patch has to be able to define the key, or every read
  // of that key throws for the life of the page.
  test("a patch defines a key the document never saw seeded", () => {
    expect(() => waterui.state.locale).toThrow(ReferenceError);
    page.__wateruiState.apply({ locale: { v: "en", e: 3, w: true } });
    expect(waterui.state.locale).toBe("en");

    // And the writability the patch carried is honoured.
    waterui.state.locale = "fr";
    expect(waterui.state.locale).toBe("fr");
  });

  test("a patch that defines a read-only key refuses assignment", () => {
    page.__wateruiState.apply({ derived: { v: 4, e: 1, w: false } });
    expect(waterui.state.derived).toBe(4);
    expect(() => {
      waterui.state.derived = 9;
    }).toThrow(TypeError);
  });
});

// A handler can still be awaiting when the page navigates, and its reply is
// then evaluated in the next document. Ids restarted from the same low numbers
// in every document, so such a reply could settle an unrelated call with
// another call's value.
describe("replies that arrive after a navigation", () => {
  test("a stale reply does not settle a call in the next document", async () => {
    const first = loadPage();
    const firstCall = first.waterui.invoke("slow", {});
    const staleId = first.sent[0].id;

    // The page navigates: a fresh document, a fresh bridge.
    const second = loadPage();
    let settled = false;
    const secondCall = second.waterui.invoke("other", {});
    secondCall.then(() => {
      settled = true;
    });

    // The handler from the first document finally answers.
    second.page.__wateruiResolve(staleId, true, { json: "from the old document" });
    await Promise.resolve();
    expect(settled).toBe(false);

    // The call this document actually made still settles normally.
    second.page.__wateruiResolve(second.sent[0].id, true, { json: "mine" });
    expect(await secondCall).toBe("mine");

    // Sanity: the ids really are drawn from different ranges.
    expect(second.sent[0].id).not.toBe(staleId);
    void firstCall;
  });
});

describe("integers a JavaScript number cannot hold", () => {
  const BIG = "9007199254740993";
  let page;
  let sent;
  let waterui;
  beforeEach(() => ({ page, sent, waterui } = loadPage()));

  test("survive being seeded, patched, and written back", () => {
    page.__wateruiState.define("id", { __wateruiBigInt: BIG }, 0, true);
    expect(waterui.state.id).toBe(BigInt(BIG));

    page.__wateruiState.apply({ id: { v: { __wateruiBigInt: "123456789012345678" }, e: 1 } });
    expect(waterui.state.id).toBe(123456789012345678n);

    waterui.state.id = BigInt(BIG);
    expect(sent[0].json.value).toEqual({ __wateruiBigInt: BIG });
  });

  test("survive a handler reply, nested anywhere in it", async () => {
    const reply = waterui.invoke("sizeOf", { path: "/a" });
    resolve(page, sent[0].id, { json: { sizes: [{ bytes: { __wateruiBigInt: BIG } }] } });
    expect((await reply).sizes[0].bytes).toBe(BigInt(BIG));
  });

  // Tagging follows the value, so ordinary numbers must stay ordinary or page
  // arithmetic breaks on values that were never at risk.
  test("leave ordinary numbers alone", async () => {
    page.__wateruiState.define("small", 42, 0, true);
    expect(waterui.state.small).toBe(42);

    const reply = waterui.invoke("count");
    resolve(page, sent[0].id, { json: { n: 7 } });
    expect((await reply).n).toBe(7);
  });
});
