#!/usr/bin/env python3
"""Generate the deterministic Cherenkov sbix test font."""

from __future__ import annotations

import binascii
import io
import struct
import zlib
from pathlib import Path

from fontTools.ttLib import TTFont, newTable
from fontTools.ttLib.tables.sbixStrike import Glyph, Strike
from PIL import Image


HERE = Path(__file__).resolve().parents[1]
SOURCE = HERE / "NotoColorEmojiSubset.ttf"
OUTPUT = HERE / "CherenkovSbixTest.ttf"
FIXED_TIMESTAMP = 3_155_328_000

# (IHDR colour type, bit depth) for every PNG form the glyph decoders
# accept: RGBA, RGB, greyscale and greyscale+alpha at 8 and 16 bits.
# Assigned round-robin across the strike glyphs (5 glyphs x 2 strikes
# covers all 8) so the generated font exercises each decode path.
FORMATS = (
    (6, 8),
    (6, 16),
    (2, 8),
    (2, 16),
    (0, 8),
    (0, 16),
    (4, 8),
    (4, 16),
)

# Pillow mode holding the channels each PNG colour type needs.
_MODES = {0: "L", 2: "RGB", 4: "LA", 6: "RGBA"}


def _png_chunk(tag: bytes, payload: bytes) -> bytes:
    crc = binascii.crc32(tag + payload) & 0xFFFFFFFF
    return struct.pack(">I", len(payload)) + tag + payload + struct.pack(">I", crc)


def _paeth(a: int, b: int, c: int) -> int:
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    return a if pa <= pb and pa <= pc else (b if pb <= pc else c)


def encode_png(image, color_type: int, bit_depth: int) -> bytes:
    """Encode an image to a PNG of the given colour type and bit depth,
    filtering and deflating through the standard library rather than
    Pillow's encoder: macOS Pillow wheels bundle zlib-ng while Linux
    wheels use the system zlib, so Pillow writes different deflate
    streams on different hosts for identical input.
    """
    mode = _MODES[color_type]
    px = image.convert(mode).tobytes()
    channels = len(mode)
    if bit_depth == 16:
        # 16-bit big-endian samples: v -> (v << 8) | v keeps the 8-bit
        # values exact after the decoders strip to 8 bits.
        px = bytes(channel for byte in px for channel in (byte, byte))
    bpp = channels * (2 if bit_depth == 16 else 1)
    stride = image.width * bpp
    raw = bytearray()
    prev = bytes(stride)
    for y in range(image.height):
        row = px[y * stride : (y + 1) * stride]
        best, best_row = 0, row
        best_sum = sum(v if v < 128 else 256 - v for v in row)
        if best_sum > 0:
            filtered = bytes((row[x] - prev[x]) & 0xFF for x in range(stride))
            s = sum(v if v < 128 else 256 - v for v in filtered)
            if s < best_sum:
                best, best_sum, best_row = 2, s, filtered
        if best_sum > 0:
            filtered = bytes(
                (row[x] - (row[x - bpp] if x >= bpp else 0)) & 0xFF
                for x in range(stride)
            )
            s = sum(v if v < 128 else 256 - v for v in filtered)
            if s < best_sum:
                best, best_sum, best_row = 1, s, filtered
        if best_sum > 0:
            filtered = bytes(
                (
                    row[x]
                    - _paeth(
                        row[x - bpp] if x >= bpp else 0,
                        prev[x],
                        prev[x - bpp] if x >= bpp else 0,
                    )
                )
                & 0xFF
                for x in range(stride)
            )
            s = sum(v if v < 128 else 256 - v for v in filtered)
            if s < best_sum:
                best, best_sum, best_row = 4, s, filtered
        raw.append(best)
        raw += best_row
        prev = row
    comp = zlib.compressobj(-1, zlib.DEFLATED, 15, 9, zlib.Z_FILTERED)
    idat = comp.compress(bytes(raw)) + comp.flush()
    ihdr = struct.pack(
        ">IIBBBBB", image.width, image.height, bit_depth, color_type, 0, 0, 0
    )
    return (
        b"\x89PNG\r\n\x1a\n"
        + _png_chunk(b"IHDR", ihdr)
        + _png_chunk(b"IDAT", idat)
        + _png_chunk(b"IEND", b"")
    )


def png_at_size(png: bytes, ppem: int, color_type: int, bit_depth: int) -> bytes:
    with Image.open(io.BytesIO(png)) as image:
        width = round(image.width * ppem / 109)
        height = round(image.height * ppem / 109)
        resized = image.convert("RGBA").resize(
            (width, height), Image.Resampling.LANCZOS
        )
        return encode_png(resized, color_type, bit_depth)


def main() -> None:
    font = TTFont(SOURCE, recalcTimestamp=False)
    cbdt_glyphs = font["CBDT"].strikeData[0]
    del font["CBDT"]
    del font["CBLC"]

    for record in font["name"].names:
        if record.nameID in (1, 16):
            value = "Cherenkov Sbix Test"
        elif record.nameID == 4:
            value = "Cherenkov Sbix Test"
        elif record.nameID == 6:
            value = "CherenkovSbixTest-Regular"
        elif record.nameID == 3:
            value = "Cherenkov Sbix Test Regular"
        else:
            continue
        record.string = value.encode(record.getEncoding())

    for glyph_name, (advance, _lsb) in font["hmtx"].metrics.items():
        font["hmtx"].metrics[glyph_name] = (advance, 0)

    font["head"].created = FIXED_TIMESTAMP
    font["head"].modified = FIXED_TIMESTAMP
    font.recalcTimestamp = False

    sbix = newTable("sbix")
    sbix.version = 1
    sbix.flags = 1
    sbix.strikes = {}
    slot = 0
    for ppem in (32, 96):
        strike = Strike(ppem=ppem, resolution=72)
        for glyph_name in sorted(cbdt_glyphs):
            color_type, bit_depth = FORMATS[slot % len(FORMATS)]
            slot += 1
            strike.glyphs[glyph_name] = Glyph(
                glyphName=glyph_name,
                graphicType="png ",
                originOffsetX=0,
                originOffsetY=round(-27 * ppem / 109),
                imageData=png_at_size(
                    cbdt_glyphs[glyph_name].imageData, ppem, color_type, bit_depth
                ),
            )
        sbix.strikes[ppem] = strike
    font["sbix"] = sbix
    font.save(OUTPUT, reorderTables=True)


if __name__ == "__main__":
    main()
