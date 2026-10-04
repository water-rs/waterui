#!/usr/bin/env python3
"""Regenerate the whole ``scenes/`` tree in one command.

Nothing under ``scenes/corpus``, ``scenes/perf`` or ``scenes/fonts`` is
committed; this produces all of it:

    1. fetch   ``scenes/fonts/_full/``   pinned upstream files, hash-verified
    2. fonts   ``scenes/fonts/``         OFL subsets and the licence
    3. tools   ``scenes/fonts/*.ttf``    the authored and derived test fonts
    4. corpus  ``scenes/corpus/``        scene.json + resources per scene
    5. perf    ``scenes/perf/``          the five perf scenes

The Python steps need ``scenes/fonts/tools/requirements.txt`` installed
(``python3 -m pip install -r scenes/fonts/tools/requirements.txt``).
Output is deterministic: the tree is byte-identical across machines, so
a checkout and CI render the same scenes.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCENES = ROOT / "scenes"
TOOLS = SCENES / "fonts" / "tools"

GENERATOR = ["cargo", "run", "-q", "--locked", "-p", "cherenkov-scene",
             "--features", "generator", "--bin"]


def run(cmd: list[str]) -> None:
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, cwd=ROOT, check=True)


def main() -> int:
    run([sys.executable, str(TOOLS / "fetch-fonts.py")])
    run([*GENERATOR, "prepare-fonts"])
    for tool in (
        "build_emoji_subset.py",
        "build_colr_test_font.py",
        "build_winding_test_font.py",
        "make_sbix_font.py",
        "build_static_test_font.py",
    ):
        run([sys.executable, str(TOOLS / tool)])
    # The writer does not clean stale scenes; regenerate from scratch.
    for out in ("corpus", "perf"):
        shutil.rmtree(SCENES / out, ignore_errors=True)
    run([*GENERATOR, "generate-corpus"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
