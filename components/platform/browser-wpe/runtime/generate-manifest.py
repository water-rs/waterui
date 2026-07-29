#!/usr/bin/env python3

import argparse
import json
import pathlib


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("metadata", nargs="+", type=pathlib.Path)
    arguments = parser.parse_args()

    artifacts = []
    identities = set()
    for path in sorted(arguments.metadata):
        artifact = json.loads(path.read_text())
        identity = (
            artifact["engine"],
            artifact["version"],
            artifact["platform"],
            artifact["architecture"],
        )
        if identity in identities:
            raise RuntimeError(f"duplicate browser runtime artifact {identity}")
        identities.add(identity)
        filename = artifact.pop("file")
        artifact["url"] = f"{arguments.base_url.rstrip('/')}/{filename}"
        artifacts.append(artifact)

    manifest = {"schema_version": 1, "artifacts": artifacts}
    arguments.output.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    )


if __name__ == "__main__":
    main()
