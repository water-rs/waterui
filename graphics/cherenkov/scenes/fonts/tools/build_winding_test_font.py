"""Generated test font builder for the Cherenkov corpus.

Produces ``scenes/fonts/CherenkovWindingTest.ttf``: a tiny outline test
font whose PUA glyphs carry overlapping contours — one composite glyph
made of two overlapping circles and one self-crossing bowtie contour.
Both rasterize to the same union shape on every backend once the glyph
atlas resolves winding like the path rasterizer does.

This file is generated; do not edit the TTF by hand. Run
``python3 scenes/fonts/tools/build_winding_test_font.py`` to regenerate.
"""

import math
import pathlib

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

UPEM = 1000
OUT = pathlib.Path(__file__).resolve().parent.parent / "CherenkovWindingTest.ttf"


def circle(pen, cx, cy, r):
    # TrueType curves are quadratic: four 90-degree arcs whose off-curve
    # control sits at the intersection of the endpoint tangents.
    rc = r / math.cos(math.pi / 4)
    pen.moveTo((cx + r, cy))
    for quarter in range(4):
        mid = (quarter * 90 + 45) * math.pi / 180
        end = (quarter + 1) * 90 * math.pi / 180
        pen.qCurveTo(
            (cx + rc * math.cos(mid), cy + rc * math.sin(mid)),
            (cx + r * math.cos(end), cy + r * math.sin(end)),
        )
    pen.closePath()


def build():
    glyphs = {}
    glyphs[".notdef"] = TTGlyphPen(None).glyph()

    pen = TTGlyphPen(None)
    circle(pen, 350, 450, 320)
    glyphs["discL"] = pen.glyph()

    pen = TTGlyphPen(None)
    circle(pen, 650, 450, 320)
    glyphs["discR"] = pen.glyph()

    # U+E200: a composite of two overlapping same-direction circles —
    # component outlines overlap, so the resolved-winding union is the
    # only correct non-zero fill.
    pen = TTGlyphPen(glyphs)
    pen.addComponent("discL", (1.0, 0.0, 0.0, 1.0, 0.0, 0.0))
    pen.addComponent("discR", (1.0, 0.0, 0.0, 1.0, 0.0, 0.0))
    glyphs["overlapComposite"] = pen.glyph()

    # U+E201: one self-crossing contour — a bowtie keeps both lobes.
    pen = TTGlyphPen(None)
    pen.moveTo((100, 700))
    pen.lineTo((900, 200))
    pen.lineTo((900, 700))
    pen.lineTo((100, 200))
    pen.closePath()
    glyphs["bowtie"] = pen.glyph()

    # U+E202: two same-direction nested squares; the inner one is a
    # duplicate winding deposit, not a hole, so the union is the outer
    # square filled solid.
    pen = TTGlyphPen(None)
    for x0, y0, x1, y1 in [(100.0, 100.0, 900.0, 900.0), (300.0, 300.0, 700.0, 700.0)]:
        pen.moveTo((x0, y0))
        pen.lineTo((x1, y0))
        pen.lineTo((x1, y1))
        pen.lineTo((x0, y1))
        pen.closePath()
    glyphs["nested"] = pen.glyph()

    glyphs["space"] = TTGlyphPen(None).glyph()

    order = [".notdef", "discL", "discR", "overlapComposite", "bowtie", "nested", "space"]
    cmap = {
        0x0020: "space",
        0xE200: "overlapComposite",
        0xE201: "bowtie",
        0xE202: "nested",
    }

    fb = FontBuilder(UPEM, isTTF=True)
    # Pin the head timestamps so the output is reproducible byte-for-byte.
    fb.font["head"].created = 3873465583
    fb.font["head"].modified = 3873465583
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap(cmap)
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics(
        {name: (250 if name == "space" else 1000, 0) for name in order}
    )
    fb.setupHorizontalHeader(ascent=900, descent=-100)
    fb.setupOS2(
        sTypoAscender=900,
        sTypoDescender=-100,
        usWinAscent=900,
        usWinDescent=100,
    )
    fb.setupNameTable({"familyName": "Cherenkov Winding Test", "styleName": "Regular"})
    fb.setupPost()
    fb.setupMaxp()
    fb.save(OUT)
    print("wrote", OUT)


if __name__ == "__main__":
    build()
