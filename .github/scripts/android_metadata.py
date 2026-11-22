# /// script
# requires-python = ">=3.11"
# ///
"""Select Android packaging when parsed scaffold metadata changes."""

import argparse
import json
import os
import subprocess
import tomllib
from pathlib import Path


def android_values(source: str, table: str) -> dict:
    metadata = tomllib.loads(source)["package"].get("metadata", {}).get(table, {})
    return {
        key: value
        for key, value in metadata.items()
        if key.startswith(("android-", "hydrolysis-android-"))
        # Android resolves its Rust backend through this shared path too.
        or key == "hydrolysis-path"
    }


def metadata_at(revision: str) -> dict:
    return {
        path: android_values(
            subprocess.check_output(["git", "show", f"{revision}:{path}"], text=True),
            table,
        )
        for path, table in (
            ("Cargo.toml", "waterui"),
            ("cli/Cargo.toml", "waterui-scaffold"),
        )
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", default="HEAD")
    args = parser.parse_args()
    changed = metadata_at(args.base) != metadata_at(args.head)
    print(json.dumps({"changed": changed}))
    if output := os.environ.get("GITHUB_OUTPUT"):
        with Path(output).open("a") as handle:
            handle.write(f"changed={str(changed).lower()}\n")


if __name__ == "__main__":
    main()
