#!/usr/bin/env python3
"""Build ``scenes/fonts/NotoColorEmojiSubset.ttf``.

The upstream ``_full/NotoColorEmoji.ttf`` subset down to the five emoji
``corpus::BITMAP_EMOJI`` uses; the CBDT strikes it carries feed both the
``text-cbdt*`` scenes and the sbix fixture.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
SOURCE = HERE / "_full" / "NotoColorEmoji.ttf"
OUTPUT = HERE / "NotoColorEmojiSubset.ttf"
EMOJI = "U+2615,U+26A0,U+26A1,U+2764,U+1F600"


def main() -> int:
    subprocess.run(
        [
            sys.executable,
            "-m",
            "fontTools.subset",
            str(SOURCE),
            f"--unicodes={EMOJI}",
            f"--output-file={OUTPUT}",
        ],
        check=True,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
