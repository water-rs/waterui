#!/usr/bin/env python3

from __future__ import annotations

import sys
from collections import defaultdict
from pathlib import Path


def heading_for_artifact(name: str) -> str:
    prefix = "test-snapshots-"
    if name.startswith(prefix):
        return name[len(prefix) :]
    return name


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: generate_test_summary.py <artifacts-dir>")

    root = Path(sys.argv[1])
    pngs = sorted(root.rglob("*.png"))

    print("## WaterUI Test Snapshots")
    print()

    if not pngs:
        print("No PNG test artifacts were downloaded.")
        return 0

    grouped: dict[str, dict[str, list[tuple[str, str]]]] = defaultdict(
        lambda: defaultdict(list)
    )

    for path in pngs:
        relative = path.relative_to(root)
        if len(relative.parts) < 4:
            raise RuntimeError(
                f"expected artifact path <artifact>/<suite>/<case>/<stage>.png, got {relative}"
            )
        artifact = relative.parts[0]
        suite = "/".join(relative.parts[1:-2])
        case = relative.parts[-2]
        stage = path.stem
        grouped[artifact][suite].append((case, stage))

    for artifact_name in sorted(grouped):
        print(f"### {heading_for_artifact(artifact_name)}")
        print()
        for suite in sorted(grouped[artifact_name]):
            print(f"#### `{suite}`")
            print()
            print("| Case | Stage | Snapshot |")
            print("| --- | --- | --- |")
            entries = sorted(grouped[artifact_name][suite])
            for case, stage in entries:
                relative_path = (
                    Path(artifact_name) / Path(suite) / case / f"{stage}.png"
                ).as_posix()
                print(
                    f"| `{case}` | `{stage}` | ![{case}-{stage}]({Path(sys.argv[1]).as_posix()}/{relative_path}) |"
                )
            print()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
