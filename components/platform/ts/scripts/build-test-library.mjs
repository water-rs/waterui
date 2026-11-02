// Bundles the JavaScript the Rust tests load into classic scripts.
//
// The engine evaluates a bundle as a classic script: an ES module's imports
// are unreachable from it, so everything has to be flattened into one IIFE
// first. That is exactly what the CLI's bundler does for an application, and
// this is the smallest version of it — no transform, no dependency graph
// beyond the library itself — so a Rust test can drive the real library
// rather than a fixture that imitates it.
//
// Two bundles come out of it:
//
//   * `tests/fixtures/library.js`, the library alone, published on
//     `globalThis.waterui`, which the equivalence tests append their own
//     module source to; and
//   * `tests/fixtures/mount.js`, a whole application bundle — the library
//     plus one module plus the `installRuntimeGlobal` call — which
//     `tests/mount.rs` loads to mount a module through `tsx!`.
//
// Run it through `scripts/build-test-library.sh`, which writes both;
// `tests/js/library.test.js` fails when a committed file is not what this
// produces.

import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");

/**
 * Bundles one entry into a classic script.
 *
 * @param {string} entry - Path of the entry, relative to this package.
 * @param {string} header - The generated-file header to put on top.
 */
async function build(entry, header) {
  // The bundler writes each module's path into the output as a comment,
  // relative to the working directory. A committed artifact that depended on
  // where it was built would be stale the moment someone ran the script from
  // the repository root instead of from this package, so the build happens
  // from this package either way.
  const cwd = process.cwd();
  process.chdir(root);
  try {
    const result = await Bun.build({
      entrypoints: [resolve(root, entry)],
      format: "iife",
      target: "browser",
      minify: false,
    });
    if (!result.success) {
      throw new AggregateError(result.logs, `bundling ${entry} failed`);
    }
    const [artifact] = result.outputs;
    return header + (await artifact.text());
  } finally {
    process.chdir(cwd);
  }
}

/** The bundle's text, as it is written to `tests/fixtures/library.js`. */
export function buildLibrary() {
  return build("tests/fixtures/library.entry.js", LIBRARY_HEADER);
}

/** The bundle's text, as it is written to `tests/fixtures/mount.js`. */
export function buildMount() {
  return build("tests/fixtures/mount.entry.js", MOUNT_HEADER);
}

/** What every generated bundle says about where it comes from. */
const GENERATED = `// GENERATED — do not edit. Rebuild with:
//     bun run components/platform/ts/scripts/build-test-library.mjs
// from the repository root, or run the script through
// scripts/build-test-library.sh. tests/js/library.test.js fails when this
// file is not what the current sources produce.
//`;

const LIBRARY_HEADER = `${GENERATED}
// The WaterUI JavaScript library, flattened into one classic script and
// published on globalThis.waterui, which is how the Rust tests reach it: the
// engine evaluates a bundle as a script, so nothing an ES module declares
// would otherwise be reachable.
`;

const MOUNT_HEADER = `${GENERATED}
// One whole application bundle: the WaterUI JavaScript library, the module
// tests/fixtures/mount.entry.js declares, and the installRuntimeGlobal call
// that publishes it — the shape the CLI's bundler produces for a real app.
`;

/** Every bundle this script writes, and where. */
const BUNDLES = [
  ["tests/fixtures/library.js", buildLibrary],
  ["tests/fixtures/mount.js", buildMount],
];

if (import.meta.main) {
  for (const [target, bundle] of BUNDLES) {
    const path = resolve(root, target);
    await Bun.write(path, await bundle());
    console.log(`wrote ${path}`);
  }
}
