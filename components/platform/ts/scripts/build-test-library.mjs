// Bundles the JavaScript library into the classic script the Rust tests load.
//
// The engine evaluates a bundle as a classic script: an ES module's imports
// are unreachable from it, so the library has to be flattened into one IIFE
// first. That is exactly what the CLI's bundler does for an application, and
// this is the smallest version of it — one entry, no transform, no
// application modules — so a Rust test can mount the real library rather than
// a fixture that imitates it.
//
// Run it through `scripts/build-test-library.sh`, which writes
// `tests/fixtures/library.js`; `tests/js/library.test.js` fails when the
// committed file is not what this produces.

import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");

/** The bundle's text, as it is written to `tests/fixtures/library.js`. */
export async function buildLibrary() {
  // The bundler writes each module's path into the output as a comment,
  // relative to the working directory. A committed artifact that depended on
  // where it was built would be stale the moment someone ran the script from
  // the repository root instead of from this package, so the build happens
  // from this package either way.
  const cwd = process.cwd();
  process.chdir(root);
  try {
    const result = await Bun.build({
      entrypoints: [resolve(root, "tests/fixtures/library.entry.js")],
      format: "iife",
      target: "browser",
      minify: false,
    });
    if (!result.success) {
      throw new AggregateError(result.logs, "bundling the test library failed");
    }
    const [artifact] = result.outputs;
    return HEADER + (await artifact.text());
  } finally {
    process.chdir(cwd);
  }
}

/** The header that says where the file comes from. */
const HEADER = `// GENERATED — do not edit. Rebuild with:
//     bun run components/platform/ts/scripts/build-test-library.mjs
// from the repository root, or run the script through
// scripts/build-test-library.sh. tests/js/library.test.js fails when this
// file is not what the current src/js produces.
//
// The WaterUI JavaScript library, flattened into one classic script and
// published on globalThis.waterui, which is how the Rust tests reach it: the
// engine evaluates a bundle as a script, so nothing an ES module declares
// would otherwise be reachable.
`;

if (import.meta.main) {
  const path = resolve(root, "tests/fixtures/library.js");
  await Bun.write(path, await buildLibrary());
  console.log(`wrote ${path}`);
}
