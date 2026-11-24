// The committed bundles are what the current sources produce.
//
// The Rust tests drive the real JavaScript library, which reaches them as
// `tests/fixtures/library.js` and `tests/fixtures/mount.js` — committed
// artifacts, because a Rust test must not run a bundler. A committed artifact
// rots, so this is the check that it cannot: it builds each one again and
// compares.

import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

import { buildLibrary, buildMount } from "../../scripts/build-test-library.mjs";
import * as authoring from "../../src/js/index.js";
import * as runtime from "../../src/js/runtime.js";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

const RUNTIME_ONLY_EXPORTS = [
  "getHost",
  "installHost",
  "installRuntimeGlobal",
  "makeCallback",
  "mount",
  "read",
  "subscribe",
  "toAccessor",
  "toSignal",
  "uninstallHost",
  "write",
];

test("the waterui module exports only the authoring API", () => {
  for (const name of RUNTIME_ONLY_EXPORTS) {
    expect(name in authoring).toBe(false);
    expect(typeof runtime[name]).toBe("function");
  }
});

test("the committed test library is the current src/js", async () => {
  const committed = readFileSync(resolve(root, "tests/fixtures/library.js"), "utf8");
  expect(await buildLibrary()).toBe(committed);
});

test("the committed mount bundle is the current entry and src/js", async () => {
  const committed = readFileSync(resolve(root, "tests/fixtures/mount.js"), "utf8");
  expect(await buildMount()).toBe(committed);
});
