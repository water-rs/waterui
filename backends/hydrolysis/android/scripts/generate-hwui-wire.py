#!/usr/bin/env python3
"""Materialize the HWUI protocol reported by a normal Cargo build."""

import json
from pathlib import Path
import shutil
import subprocess
import sys


def main() -> None:
    output = subprocess.run(
        ["cargo", "check", "--locked", "-p", "hydrolysis", "--features", "hwui",
         "--message-format=json-render-diagnostics"],
        stdout=subprocess.PIPE,
        check=True,
        text=True,
    )
    directories = {
        value
        for line in output.stdout.splitlines()
        for message in [json.loads(line)]
        if message.get("reason") == "build-script-executed"
        for key, value in message["env"]
        if key == "WATERUI_HWUI_KOTLIN_DIR"
    }
    if len(directories) != 1:
        raise RuntimeError(f"expected one HWUI Kotlin output from Cargo, got {directories}")
    source = Path(directories.pop())
    destination = Path(sys.argv[1])
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(source, destination)


if __name__ == "__main__":
    main()
