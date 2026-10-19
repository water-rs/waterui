# /// script
# requires-python = ">=3.11"
# ///
"""Digest of `cherenkov-scene`'s dependency closure in `Cargo.lock`.

The generated Cherenkov font subsets (`graphics/cherenkov/scenes/fonts/*.ttf`)
are a compile-time input (`include_bytes!`) produced by the `prepare-fonts`
binary of `cherenkov-scene`, so their bytes can change only when a package in
that crate's lockfile closure does. The font-subset cache key in
`.github/actions/generate-scene-assets` uses this digest in place of the whole
`Cargo.lock`: a dependency bump anywhere else in the workspace then leaves the
key untouched instead of forcing a cold font build.

The closure follows every `dependencies` edge transitively, optional
dependencies included — a superset is correct, a subset is not. The digest is
the sha256 of the sorted `name version source checksum` line of every
reachable package. A workspace path package other than `cherenkov-scene` in
the closure is an error: the lockfile carries no checksum for it, so its
sources would have to be named in the key too.

Run: `uv run .github/scripts/font_lock_digest.py [Cargo.lock]`. Writes
`digest=<sha256>` to `$GITHUB_OUTPUT` when the variable is set, and always
prints the digest and the closure's package count. Python >= 3.11 for
`tomllib`.
"""

from __future__ import annotations

import hashlib
import os
import sys
import tomllib
from pathlib import Path
from typing import NoReturn

ROOT_PACKAGE = "cherenkov-scene"


def fail(message: str) -> NoReturn:
    print(f"font-lock-digest: {message}", file=sys.stderr)
    sys.exit(1)


def parse_dependency(spec: str) -> tuple[str, str | None, str | None]:
    """Split a `dependencies` entry into `(name, version, source)`.

    The lockfile writes `name` when the name alone is unique, `name version`
    to disambiguate versions, and `name version (source)` when the same
    version comes from more than one source.
    """
    name, _, rest = spec.partition(" ")
    version, _, source = rest.partition(" ")
    source = source.strip()
    if source:
        if not (source.startswith("(") and source.endswith(")")):
            fail(f"unparseable dependency entry: {spec!r}")
        source = source[1:-1]
    return name, version or None, source or None


def resolve(
    index: dict[str, list[dict]], spec: str, owner: str
) -> dict:
    """The one lockfile package a `dependencies` entry of `owner` names."""
    name, version, source = parse_dependency(spec)
    candidates = index.get(name)
    if not candidates:
        fail(f"{owner} depends on {spec!r}, which the lockfile does not contain")
    if version is not None:
        candidates = [p for p in candidates if p["version"] == version]
    if source is not None:
        candidates = [p for p in candidates if p.get("source") == source]
    if not candidates:
        fail(f"{owner} depends on {spec!r}, which the lockfile does not contain")
    if len(candidates) > 1:
        fail(f"{owner}'s dependency {spec!r} matches {len(candidates)} packages")
    return candidates[0]


def closure(packages: list[dict]) -> list[dict]:
    """Every package reachable from `cherenkov-scene`, itself included."""
    index: dict[str, list[dict]] = {}
    for package in packages:
        index.setdefault(package["name"], []).append(package)

    roots = index.get(ROOT_PACKAGE, [])
    if len(roots) != 1:
        fail(f"lockfile holds {len(roots)} packages named {ROOT_PACKAGE!r}")

    seen: dict[tuple[str, str, str], dict] = {}
    queue = [roots[0]]
    while queue:
        package = queue.pop()
        key = (
            package["name"],
            package["version"],
            package.get("source", ""),
        )
        if key in seen:
            continue
        if "source" not in package and package["name"] != ROOT_PACKAGE:
            fail(
                f"{package['name']} is a workspace path package in "
                f"{ROOT_PACKAGE}'s closure; its sources are not in the key"
            )
        seen[key] = package
        for spec in package.get("dependencies", []):
            queue.append(resolve(index, spec, package["name"]))
    return list(seen.values())


def main() -> int:
    lock_path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("Cargo.lock")
    with lock_path.open("rb") as handle:
        packages = tomllib.load(handle)["package"]

    reachable = closure(packages)
    lines = sorted(
        "{} {} {} {}".format(
            p["name"],
            p["version"],
            p.get("source", ""),
            p.get("checksum", ""),
        )
        for p in reachable
    )
    digest = hashlib.sha256(("\n".join(lines) + "\n").encode()).hexdigest()

    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with Path(output).open("a") as handle:
            handle.write(f"digest={digest}\n")
    print(f"digest={digest} packages={len(reachable)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
