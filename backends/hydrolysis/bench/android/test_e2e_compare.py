"""Color-managed PNG comparison tests for bench/android/scripts/e2e.py."""

import struct
import sys
import tempfile
import unittest
import zlib
from pathlib import Path

from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent / "scripts"))
import e2e  # noqa: E402


def _png_chunk(tag: bytes, payload: bytes) -> bytes:
    crc = zlib.crc32(tag + payload) & 0xFFFFFFFF
    return struct.pack(">I", len(payload)) + tag + payload + struct.pack(">I", crc)


def _with_srgb_chunk(png_bytes: bytes) -> bytes:
    """Insert an explicit PNG sRGB chunk (intent=0 perceptual) after IHDR."""
    ihdr_end = 8 + 4 + 4 + 13 + 4
    return png_bytes[:ihdr_end] + _png_chunk(b"sRGB", b"\x00") + png_bytes[ihdr_end:]


class CompareImagesColorManagementTests(unittest.TestCase):
    def test_srgb_chunk_and_display_p3_iccp_match_in_linear_display_p3(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            p3_path = root / "gpu_display_p3.png"
            srgb_path = root / "hwui_srgb.png"
            diff_path = root / "diff.png"

            # #E53935 encoded as Display P3 (211, 72, 62) with iCCP chunk vs sRGB (229, 57, 53) with sRGB chunk.
            Image.new("RGB", (64, 64), (211, 72, 62)).save(
                p3_path,
                icc_profile=e2e._display_p3_icc_bytes(),
            )
            Image.new("RGB", (64, 64), (229, 57, 53)).save(srgb_path)
            srgb_path.write_bytes(_with_srgb_chunk(srgb_path.read_bytes()))

            self.assertEqual(e2e.compare_images(p3_path, srgb_path, 8, 0.0, diff_path), 0)
            self.assertFalse(diff_path.exists())

    def test_genuine_color_mismatch_fails_and_writes_diff(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ref_path = root / "ref.png"
            act_path = root / "act.png"
            diff_path = root / "diff.png"

            Image.new("RGB", (64, 64), (211, 72, 62)).save(
                ref_path,
                icc_profile=e2e._display_p3_icc_bytes(),
            )
            Image.new("RGB", (64, 64), (40, 120, 220)).save(act_path)
            act_path.write_bytes(_with_srgb_chunk(act_path.read_bytes()))

            self.assertEqual(e2e.compare_images(ref_path, act_path, 8, 0.0, diff_path), 1)
            self.assertTrue(diff_path.exists())


if __name__ == "__main__":
    unittest.main()
