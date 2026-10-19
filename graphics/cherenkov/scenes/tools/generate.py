#!/usr/bin/env python3
"""Regenerate the ``scenes/`` tree.

Nothing under ``scenes/corpus``, ``scenes/perf`` or the generated files
in ``scenes/fonts`` is committed. By default this produces all of it:

    1. fetch   ``scenes/fonts/_full/``   pinned upstream files, hash-verified
    2. fonts   ``scenes/fonts/``         OFL subsets and the licence
    3. tools   ``scenes/fonts/*.ttf``    the authored and derived test fonts
    4. corpus  ``scenes/corpus/``        scene.json + resources per scene
    5. perf    ``scenes/perf/``          perf scenes, including a batched road stroke

``--fonts-only`` stops after the font tools. Compile-time readers
(``include_bytes!`` of ``scenes/fonts``) need nothing else.
``--corpus-only`` runs the corpus and perf steps from the fonts already
in ``scenes/fonts`` and does not rebuild them. The two flags are
mutually exclusive; omitting both is the full run.

The Python steps need ``scenes/fonts/tools/requirements.txt`` installed
(``python3 -m pip install -r scenes/fonts/tools/requirements.txt``).
Output is deterministic: the tree is byte-identical across machines, so
a checkout and CI render the same scenes.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCENES = ROOT / "scenes"
TOOLS = SCENES / "fonts" / "tools"

GENERATOR = ["cargo", "run", "-q", "--locked", "-p", "cherenkov-scene",
             "--features", "generator", "--bin"]

FONT_TOOLS = (
    "build_emoji_subset.py",
    "build_colr_test_font.py",
    "build_winding_test_font.py",
    "make_sbix_font.py",
    "build_static_test_font.py",
)


def run(cmd: list[str]) -> None:
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, cwd=ROOT, check=True)


def generate_fonts() -> None:
    run([sys.executable, str(TOOLS / "fetch-fonts.py")])
    run([*GENERATOR, "prepare-fonts"])
    for tool in FONT_TOOLS:
        run([sys.executable, str(TOOLS / tool)])


def generate_corpus() -> None:
    # The writer does not clean stale scenes; regenerate from scratch.
    for out in ("corpus", "perf"):
        shutil.rmtree(SCENES / out, ignore_errors=True)
    run([*GENERATOR, "generate-corpus"])


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Regenerate the Cherenkov scenes tree.",
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--fonts-only",
        action="store_true",
        help="Write only scenes/fonts (fetch, prepare-fonts, and the font "
        "tools). Leave scenes/corpus and scenes/perf untouched.",
    )
    mode.add_argument(
        "--corpus-only",
        action="store_true",
        help="Write only scenes/corpus and scenes/perf from the fonts "
        "already in scenes/fonts. Do not fetch or rebuild the fonts.",
    )
    args = parser.parse_args()
    if not args.corpus_only:
        generate_fonts()
    if not args.fonts_only:
        generate_corpus()
    return 0


if __name__ == "__main__":
    sys.exit(main())
