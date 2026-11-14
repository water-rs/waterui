// Prunes a Cargo target directory down to the units one build used.
//
// usage: node prune.mjs <target-dir> <cargo-messages.jsonl>
//
// The messages file is the `--message-format=json-render-diagnostics`
// stdout of every cargo invocation of the run. Cargo reports a
// `compiler-artifact` for every unit it used — fresh ones included — and
// a `build-script-executed` (with its `out_dir`) for every build-script
// run. Each reported path names its unit by the 16-hex-digit hash Cargo
// puts in `deps/lib<name>-<hash>.rmeta`, `build/<name>-<hash>/` and
// `.fingerprint/<name>-<hash>/`; the hashes found in those paths are the
// units to keep.
//
// In every profile directory under the target (`target/<profile>` and
// `target/<triple>/<profile>`, recognised by its `.fingerprint`), each
// entry of `deps`, `build`, `examples` and `.fingerprint` whose name does
// not end in a kept hash is deleted — stale dependency versions, units of
// feature sets no longer built, and anything whose name carries no hash.
// Deleting a live unit costs a rebuild, never correctness: Cargo
// recompiles whatever is missing.

import { existsSync, readFileSync, readdirSync, rmSync, statSync } from 'node:fs';
import { join } from 'node:path';

const UNIT_DIRS = ['deps', 'build', 'examples', '.fingerprint'];
// `<name>-<hash>` followed by any number of extensions:
// `libfoo-<hash>.rmeta`, `foo-<hash>.d`, `foo-<hash>.dll.lib`, `foo-<hash>`.
const ENTRY_HASH = /-([0-9a-f]{16})(?:\.[A-Za-z0-9_]+)*$/;
const PATH_HASH = /-([0-9a-f]{16})(?=[./\\]|$)/g;

const [targetDir, messagesPath] = process.argv.slice(2);
if (!targetDir || !messagesPath) {
  throw new Error('usage: node prune.mjs <target-dir> <cargo-messages.jsonl>');
}

const keep = new Set();
const lines = readFileSync(messagesPath, 'utf8').split('\n');
lines.forEach((line, index) => {
  if (line.trim() === '') return;
  let message;
  try {
    message = JSON.parse(line);
  } catch (error) {
    throw new Error(`${messagesPath}:${index + 1} is not a Cargo JSON message: ${error.message}`);
  }
  const paths = [];
  if (message.reason === 'compiler-artifact') paths.push(...message.filenames);
  if (message.reason === 'build-script-executed') paths.push(message.out_dir);
  for (const path of paths) {
    for (const match of path.matchAll(PATH_HASH)) keep.add(match[1]);
  }
});
if (keep.size === 0) {
  throw new Error(`${messagesPath} reports no unit: the cargo steps did not record their messages`);
}

// `target/<profile>` and `target/<triple>/<profile>`.
const profileDirs = [];
const subdirs = (dir) => readdirSync(dir, { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => join(dir, entry.name));
for (const dir of subdirs(targetDir)) {
  if (existsSync(join(dir, '.fingerprint'))) {
    profileDirs.push(dir);
    continue;
  }
  for (const nested of subdirs(dir)) {
    if (existsSync(join(nested, '.fingerprint'))) profileDirs.push(nested);
  }
}
if (profileDirs.length === 0) {
  throw new Error(`${targetDir} holds no Cargo profile directory`);
}

for (const profileDir of profileDirs) {
  let kept = 0;
  let removed = 0;
  for (const unitDir of UNIT_DIRS.map((name) => join(profileDir, name))) {
    if (!existsSync(unitDir) || !statSync(unitDir).isDirectory()) continue;
    for (const name of readdirSync(unitDir)) {
      const hash = ENTRY_HASH.exec(name)?.[1];
      if (hash !== undefined && keep.has(hash)) {
        kept += 1;
      } else {
        rmSync(join(unitDir, name), { recursive: true, force: true });
        removed += 1;
      }
    }
  }
  process.stdout.write(`${profileDir}: kept ${kept} entries, removed ${removed}\n`);
}
