#!/usr/bin/env python3
"""Generate the deterministic Cherenkov static test font.

Produces ``scenes/fonts/CherenkovStaticSans.ttf``: the corpus Noto Sans
subset pinned to its regular instance (``wght`` 400, ``wdth`` 100) and
renamed, so it is a single regular face with no variation axes. A bold
request against it is what makes a layout engine synthesize the weight
(fontique's ``embolden``), which the synthetic-bold scenes exercise.

The family is renamed so it never joins the variable ``Noto Sans``
family when both are registered in one font collection. The font stays
under the SIL Open Font License of its source (``scenes/fonts/OFL.txt``).

Run after ``prepare-fonts``, which writes the ``NotoSans.ttf`` subset.
"""

from __future__ import annotations

from pathlib import Path

from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

HERE = Path(__file__).resolve().parents[1]
SOURCE = HERE / "NotoSans.ttf"
OUTPUT = HERE / "CherenkovStaticSans.ttf"
FIXED_TIMESTAMP = 3_155_328_000
FAMILY = "Cherenkov Static Sans"


def main() -> None:
    font = TTFont(SOURCE, recalcTimestamp=False)
    static = instancer.instantiateVariableFont(
        font, {"wght": 400, "wdth": 100}, updateFontNames=False
    )

    names = static["name"]
    for name_id in (16, 17, 21, 22, 25):
        names.removeNames(nameID=name_id)
    for record in names.names:
        if record.nameID == 1:
            value = FAMILY
        elif record.nameID == 2:
            value = "Regular"
        elif record.nameID == 3:
            value = f"{FAMILY} Regular"
        elif record.nameID == 4:
            value = f"{FAMILY} Regular"
        elif record.nameID == 6:
            value = "CherenkovStaticSans-Regular"
        else:
            continue
        record.string = value.encode(record.getEncoding())

    static["head"].created = FIXED_TIMESTAMP
    static["head"].modified = FIXED_TIMESTAMP
    static.recalcTimestamp = False
    static.save(OUTPUT, reorderTables=True)


if __name__ == "__main__":
    main()
