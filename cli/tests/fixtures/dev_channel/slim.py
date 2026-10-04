#!/usr/bin/env python3
"""Cut a real dev-channel capture down to the reviewable subgraph (#203).

Run it in a directory holding the full captures of one `water create` +
`water build` on the dev channel:

    python3 slim.py --capture <dir> --out <fixtures-dir>

<dir> must contain:

    Water.toml                          the project's manifest (its
                                        [framework] table is used verbatim)
    Water.lock                          the channel's certified lock
    Cargo.lock                          the project's lock
    hydrolysis-backend.lock             the generated hydrolysis backend's
                                        Cargo.lock (in the project's managed
                                        build cache) after the build resolved it
    hydrolysis-backend.metadata.json    `cargo metadata --format-version 1`
                                        on that backend's Cargo.toml

It writes the slimmed fixtures — Water.toml (with `lock_sha256` recomputed
for the slim Water.lock), Water.lock, Cargo.lock, hydrolysis-backend.lock,
and hydrolysis-backend.metadata.json — keeping only:

- the accesskit family (`accesskit`, `accesskit_winit`, `winit` — the
  package whose edge pulls the additive `objc2` line),
- `objc2` at both resolved versions (one canonical pin, one addition) with
  its `objc2-encode` edge,
- `hydrolysis` and `waterui`, the extracted crates at their sanctioned git
  sources, and
- the two path packages (the app and the generated backend).

Dependency edges are trimmed to targets inside the kept set; a dep entry
that carries an explicit version is only kept when that version's package
is kept, so every surviving edge still lands on a package the lock records.
"""

import hashlib
import json
import re
import sys
from pathlib import Path

REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"

# Names kept from every capture. Path packages (no `source`) — the app and
# the generated backend crate — are always kept.
CANONICAL_NAMES = {"accesskit", "objc2", "objc2-encode", "hydrolysis", "waterui"}
PROJECT_NAMES = {"waterui"}
RESOLVED_NAMES = CANONICAL_NAMES | {"accesskit_winit", "winit"}


def split_lock(text):
    """(header, [(name, version, source, block)]) for a Cargo.lock."""
    parts = re.split(r"(?=\[\[package\]\]\n)", text)
    packages = []
    for block in parts[1:]:
        name = re.search(r'^name = "(.+)"', block, re.M).group(1)
        version = re.search(r'^version = "(.+)"', block, re.M).group(1)
        source = re.search(r'^source = "(.+)"', block, re.M)
        packages.append((name, version, source and source.group(1), block))
    return parts[0], packages


def trim_deps(block, kept):
    """Drop dependency edges whose target is not in `kept` ((name, version) or name)."""
    m = re.search(r'^dependencies = \[\n(.*)^\]\n', block, re.M | re.S)
    if not m:
        return block
    entries = []
    for line in m.group(1).splitlines():
        entry = re.match(r'\s*"(.+?)",?\s*$', line)
        if not entry:
            continue
        spec = entry.group(1)
        dep = spec.split(" ")
        if dep[0] in kept["names"] and (
            len(dep) == 1 or (dep[0], dep[1]) in kept["exact"]
        ):
            entries.append(f' "{spec}",\n')
    deps = 'dependencies = [\n' + "".join(sorted(entries)) + "]\n" if entries else ""
    return block[: m.start()] + deps + block[m.end() :] if deps else re.sub(
        r'dependencies = \[\n.*?\]\n', "", block, flags=re.S
    )


def slim_lock(text, names, keep_path=True):
    header, packages = split_lock(text)
    selected = [
        (n, v, s, b)
        for n, v, s, b in packages
        if n in names or (keep_path and s is None)
    ]
    kept = {
        "names": {n for n, _, _, _ in selected},
        "exact": {(n, v) for n, v, _, _ in selected},
    }
    return header + "".join(trim_deps(b, kept) for _, _, _, b in selected)


def main():
    capture = Path(sys.argv[sys.argv.index("--capture") + 1])
    out = Path(sys.argv[sys.argv.index("--out") + 1])
    out.mkdir(parents=True, exist_ok=True)

    # The channel's certified lock and the project's own.
    water_lock = slim_lock(
        (capture / "Water.lock").read_text(), CANONICAL_NAMES, keep_path=False
    )
    (out / "Water.lock").write_text(water_lock)
    (out / "Cargo.lock").write_text(
        slim_lock((capture / "Cargo.lock").read_text(), PROJECT_NAMES)
    )
    backend_lock = slim_lock(
        (capture / "hydrolysis-backend.lock").read_text(), RESOLVED_NAMES
    )
    (out / "hydrolysis-backend.lock").write_text(backend_lock)

    # The project's manifest, with the slim lock's checksum re-certified.
    water_toml = re.sub(
        r'^(lock_sha256 = ")[0-9a-f]+"',
        rf'\g<1>{hashlib.sha256(water_lock.encode()).hexdigest()}"',
        (capture / "Water.toml").read_text(),
        flags=re.M,
    )
    (out / "Water.toml").write_text(water_toml)

    # The resolved `cargo metadata` graph, trimmed to the same packages.
    metadata = json.loads((capture / "hydrolysis-backend.metadata.json").read_text())
    packages = [
        {
            key: (
                [] if key in ("dependencies", "targets") else
                {} if key == "features" else
                package.get(key)
            )
            for key in (
                "name", "version", "id", "source", "dependencies",
                "targets", "features", "manifest_path", "edition",
            )
        }
        for package in metadata["packages"]
        if package["name"] in RESOLVED_NAMES or package["source"] is None
    ]
    kept_ids = {package["id"] for package in packages}
    resolve = metadata["resolve"]
    nodes = [
        {
            "id": node["id"],
            "dependencies": [
                dep for dep in node["dependencies"] if dep in kept_ids
            ],
            "deps": [],
        }
        for node in resolve["nodes"]
        if node["id"] in kept_ids
    ]
    slimmed = {
        **metadata,
        "packages": packages,
        "resolve": {**resolve, "nodes": nodes},
        "workspace_members": [resolve["root"]],
    }
    (out / "hydrolysis-backend.metadata.json").write_text(
        json.dumps(slimmed, indent=1)
    )

    print(
        f"kept {len(packages)} packages; Water.lock sha256 "
        f"{hashlib.sha256(water_lock.encode()).hexdigest()}"
    )


if __name__ == "__main__":
    main()
