// The committed test-library bundle is what the current library produces.
//
// The Rust equivalence tests mount the real JavaScript library, which reaches
// them as `tests/fixtures/library.js` — a committed artifact, because a Rust
// test must not run a bundler. A committed artifact rots, so this is the check
// that it cannot: it builds the bundle again and compares.

import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

import { buildLibrary } from "../../scripts/build-test-library.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

test("the committed test library is the current src/js", async () => {
  const committed = readFileSync(resolve(root, "tests/fixtures/library.js"), "utf8");
  expect(await buildLibrary()).toBe(committed);
});
