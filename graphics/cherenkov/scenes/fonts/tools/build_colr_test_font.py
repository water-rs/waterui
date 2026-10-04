"""Generated test font builder for the Cherenkov corpus.

Produces ``scenes/fonts/CherenkovColrTest.ttf``: a small COLRv0+v1 test
font covering transformed brushes (rotate/skew/scale on linear, radial and
sweep gradients), every COLR composite mode, clip boxes, foreground
(0xFFFF) brushes, a nested PaintColrGlyph and one COLRv0 layered glyph.

This file is generated; do not edit the TTF by hand. Run
``python3 scenes/fonts/tools/build_colr_test_font.py`` to regenerate.
"""

import pathlib

from fontTools.fontBuilder import FontBuilder
from fontTools.colorLib.builder import buildCOLR, buildCPAL
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib.tables import otTables as ot

UPEM = 1000
OUT = pathlib.Path(__file__).resolve().parent.parent / "CherenkovColrTest.ttf"

# CPAL palette 0.
RED, BLUE, YELLOW, GREEN, MAGENTA, BLACK, WHITE = range(7)
PALETTE = [
    (230 / 255, 26 / 255, 26 / 255, 1.0),
    (26 / 255, 51 / 255, 230 / 255, 1.0),
    (255 / 255, 217 / 255, 26 / 255, 1.0),
    (26 / 255, 179 / 255, 77 / 255, 1.0),
    (204 / 255, 26 / 255, 179 / 255, 1.0),
    (0.0, 0.0, 0.0, 1.0),
    (1.0, 1.0, 1.0, 1.0),
]

# The shared gradient: red -> yellow -> blue.
G_STOPS = [(0.0, RED), (0.5, YELLOW), (1.0, BLUE)]
CENTER = (500, 450)


def rect(pen, x0, y0, x1, y1):
    pen.moveTo((x0, y0))
    pen.lineTo((x1, y0))
    pen.lineTo((x1, y1))
    pen.lineTo((x0, y1))
    pen.closePath()


def circle(pen, cx, cy, r):
    # TrueType curves are quadratic: four 90° arcs whose off-curve control
    # sits at the intersection of the endpoint tangents.
    import math

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


def outlines():
    glyphs = {}
    pen = TTGlyphPen(None)
    glyphs[".notdef"] = pen.glyph()

    glyphs["space"] = TTGlyphPen(None).glyph()

    pen = TTGlyphPen(None)
    rect(pen, 50, 0, 950, 900)
    glyphs["box"] = pen.glyph()

    pen = TTGlyphPen(None)
    circle(pen, 500, 450, 400)
    glyphs["disc"] = pen.glyph()

    pen = TTGlyphPen(None)
    circle(pen, 350, 450, 300)
    glyphs["discL"] = pen.glyph()

    pen = TTGlyphPen(None)
    circle(pen, 650, 450, 300)
    glyphs["discR"] = pen.glyph()

    # A plus shape inside the box: two crossing bars, 200 wide.
    pen = TTGlyphPen(None)
    rect(pen, 400, 0, 600, 900)
    rect(pen, 50, 350, 950, 550)
    glyphs["cross"] = pen.glyph()
    return glyphs


def colorline(stops, extend="pad"):
    # Each stop is (offset, palette_index[, alpha]); 0xFFFF is the
    # COLR foreground.
    return {
        "Extend": extend,
        "ColorStop": [
            {"StopOffset": s[0], "PaletteIndex": s[1], "Alpha": s[2] if len(s) > 2 else 1.0}
            for s in stops
        ],
    }


def solid(index, alpha=1.0):
    return {
        "Format": int(ot.PaintFormat.PaintSolid),
        "PaletteIndex": index,
        "Alpha": alpha,
    }


def linear(p0, p1, p2, stops=G_STOPS, extend="pad"):
    return {
        "Format": int(ot.PaintFormat.PaintLinearGradient),
        "ColorLine": colorline(stops, extend),
        "x0": p0[0],
        "y0": p0[1],
        "x1": p1[0],
        "y1": p1[1],
        "x2": p2[0],
        "y2": p2[1],
    }


def radial(c0, r0, c1, r1, stops=G_STOPS, extend="pad"):
    return {
        "Format": int(ot.PaintFormat.PaintRadialGradient),
        "ColorLine": colorline(stops, extend),
        "x0": c0[0],
        "y0": c0[1],
        "r0": r0,
        "x1": c1[0],
        "y1": c1[1],
        "r1": r1,
    }


def sweep(center, start, end, stops=G_STOPS, extend="pad"):
    return {
        "Format": int(ot.PaintFormat.PaintSweepGradient),
        "ColorLine": colorline(stops, extend),
        "centerX": center[0],
        "centerY": center[1],
        "startAngle": start,
        "endAngle": end,
    }


def glyph(glyph, paint):
    return {
        "Format": int(ot.PaintFormat.PaintGlyph),
        "Paint": paint,
        "Glyph": glyph,
    }


def around(fmt, paint, *params):
    keys = {
        int(ot.PaintFormat.PaintRotateAroundCenter): ("angle", "centerX", "centerY"),
        int(ot.PaintFormat.PaintSkewAroundCenter): (
            "xSkewAngle",
            "ySkewAngle",
            "centerX",
            "centerY",
        ),
        int(ot.PaintFormat.PaintScaleAroundCenter): (
            "scaleX",
            "scaleY",
            "centerX",
            "centerY",
        ),
    }[int(fmt)]
    p = {"Format": int(fmt), "Paint": paint}
    p.update(dict(zip(keys, params)))
    return p


def transform(xx, yx, xy, yy, dx, dy, paint):
    return {
        "Format": int(ot.PaintFormat.PaintTransform),
        "Paint": paint,
        "Transform": {"xx": xx, "yx": yx, "xy": xy, "yy": yy, "dx": dx, "dy": dy},
    }


def composite(source, mode, backdrop):
    return {
        "Format": int(ot.PaintFormat.PaintComposite),
        "SourcePaint": source,
        "CompositeMode": mode,
        "BackdropPaint": backdrop,
    }





def build():
    glyphs = outlines()

    # One empty base glyph per colour-glyph codepoint.
    colr_names = []
    for cp in (
        list(range(0xE000, 0xE009))
        + list(range(0xE100, 0xE11C))
        + [0xE200, 0xE201]
        + list(range(0xE300, 0xE304))
        + [0xE400, 0xE500]
    ):
        name = f"g{cp:04X}"
        colr_names.append(name)
        glyphs[name] = TTGlyphPen(None).glyph()

    # `space` comes last so every existing glyph id is unchanged.
    order = [".notdef", "box", "disc", "discL", "discR", "cross"] + colr_names + ["space"]
    cmap = {0x0020: "space"}
    cmap.update({cp: f"g{cp:04X}" for cp in (
        list(range(0xE000, 0xE009))
        + list(range(0xE100, 0xE11C))
        + [0xE200, 0xE201]
        + list(range(0xE300, 0xE304))
        + [0xE400, 0xE500]
    )})
    cmap[0xE600] = "cross"

    fb = FontBuilder(UPEM, isTTF=True)
    # Pin the head timestamps so the output is reproducible byte-for-byte.
    fb.font["head"].created = 3873408498
    fb.font["head"].modified = 3873408498
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
    fb.setupNameTable({"familyName": "Cherenkov COLR Test", "styleName": "Regular"})
    fb.setupPost()
    fb.setupMaxp()

    cx, cy = CENTER
    colr = {
        # Transformed linear gradients (the gradient wraps inside the
        # PaintGlyph so only the brush is transformed).
        "gE000": glyph("box", around(ot.PaintFormat.PaintRotateAroundCenter,
                                    linear((100, 450), (900, 450), (100, 1450)),
                                    30, cx, cy)),
        "gE001": glyph("box", around(ot.PaintFormat.PaintSkewAroundCenter,
                                    linear((100, 450), (900, 450), (100, 1450)),
                                    25, 0, cx, cy)),
        "gE002": glyph("box", around(ot.PaintFormat.PaintScaleAroundCenter,
                                    linear((100, 450), (500, 450), (100, 850),
                                           extend="repeat"),
                                    1.6, 0.5, cx, cy)),
        # Transformed radial gradients.
        "gE003": glyph("box", around(ot.PaintFormat.PaintRotateAroundCenter,
                                    radial((400, 450), 50, (500, 450), 400),
                                    30, cx, cy)),
        "gE004": glyph("box", around(ot.PaintFormat.PaintSkewAroundCenter,
                                    radial((400, 450), 50, (500, 450), 400),
                                    30, 0, cx, cy)),
        "gE005": glyph("box", around(ot.PaintFormat.PaintScaleAroundCenter,
                                    radial((500, 450), 0, (500, 450), 350,
                                           extend="reflect"),
                                    1.5, 0.6, cx, cy)),
        # Transformed sweep gradients.
        "gE006": glyph("box", around(ot.PaintFormat.PaintRotateAroundCenter,
                                    sweep((500, 450), 0, 270),
                                    45, cx, cy)),
        "gE007": glyph("box", around(ot.PaintFormat.PaintSkewAroundCenter,
                                    sweep((500, 450), 0, 270),
                                    30, 0, cx, cy)),
        "gE008": glyph("box", around(ot.PaintFormat.PaintScaleAroundCenter,
                                    sweep((500, 450), 0, 360),
                                    1.5, 0.6, cx, cy)),
        # Clip boxes (ClipList entries on the base glyph; a PaintGlyph
        # clip would produce the same `push_clip_box` callback).
        "gE200": glyph("box", radial((500, 450), 0, (500, 450), 500)),
        "gE201": linear((100, 50), (900, 850), (900, -750)),
        # Foreground brushes.
        "gE300": glyph("disc", solid(0xFFFF, 1.0)),
        "gE301": glyph("disc", solid(0xFFFF, 0.4)),
        "gE302": glyph("box", linear((50, 450), (950, 450), (50, 1450),
                                    stops=[(0.0, 0xFFFF, 1.0), (1.0, YELLOW)])),
        "gE303": glyph("box", linear((50, 450), (950, 450), (50, 1450),
                                    stops=[(0.0, 0xFFFF, 0.4), (1.0, YELLOW)])),
        # A nested colour glyph under a general affine.
        "gE400": transform(
            0.8, 0.2, -0.3, 0.9, 80, 40,
            {"Format": int(ot.PaintFormat.PaintColrGlyph), "Glyph": "gE000"},
        ),
    }
    # All 28 composite modes, E100..E11B in CompositeMode order.
    for m in range(28):
        colr[f"g{0xE100 + m:04X}"] = composite(
            glyph("discR", solid(BLUE, 0.8)),
            m,
            glyph("discL", solid(RED, 1.0)),
        )

    fb.font["COLR"] = buildCOLR(
        colr,
        version=1,
        glyphMap=fb.font.getReverseGlyphMap(),
        clipBoxes={"gE200": (200, 100, 800, 800), "gE201": (100, 50, 900, 850)},
    )
    # COLR v0 layered glyph on U+E500.
    v0 = {"gE500": [("disc", GREEN), ("cross", MAGENTA)]}
    from fontTools.colorLib.builder import populateCOLRv0

    populateCOLRv0(fb.font["COLR"].table, v0, fb.font.getReverseGlyphMap())
    fb.font["CPAL"] = buildCPAL([PALETTE])
    fb.save(OUT)
    print("wrote", OUT)


if __name__ == "__main__":
    build()
