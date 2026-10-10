#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["av==19.0.1", "imageio-ffmpeg==0.6.0"]
# ///
"""Regenerate `tests/fixtures/live-photo-motion.mp4`, the live photo's motion
clip (`tests/media.rs`): 96x64, 9 frames at 12fps, AV1 in MP4.

    uv run backends/hydrolysis/tests/fixtures/generate-live-photo-motion.py

Encoding runs through a pinned, hash-verified static `ffmpeg`
(`imageio-ffmpeg`'s bundled binary — libaom-av1 inside, no system ffmpeg
dependency, identical on every CI platform), and the output is verified by
decoding it back through dav1d and counting frames: the test accepts the
fixture by reading it, so a fixture that does not decode to the documented
geometry is a hard failure here rather than a skipped assertion there.
The generated file stays out of the repository (see the repository's
no-binary-assets rule); `live-photo-motion.mp4` is gitignored.
"""

from __future__ import annotations

import hashlib
import subprocess
import sys
from pathlib import Path

import av
import imageio_ffmpeg

OUT = Path(__file__).resolve().parent / "live-photo-motion.mp4"

WIDTH, HEIGHT, FRAMES, RATE = 96, 64, 9, 12


def verify() -> None:
    """Read-back gate: exactly FRAMES frames of WIDTHxHEIGHT through dav1d."""
    try:
        decoded = 0
        with av.open(str(OUT)) as container:
            stream = container.streams.video[0]
            assert stream.codec_context.name == "libdav1d", stream.codec_context.name
            for frame in container.decode(video=0):
                assert (frame.width, frame.height) == (WIDTH, HEIGHT), (
                    frame.width,
                    frame.height,
                )
                decoded += 1
    except av.error.FFmpegError as error:
        raise SystemExit(f"{OUT.name}: not a readable AV1 clip: {error}") from error
    if decoded != FRAMES:
        raise SystemExit(f"decoded {decoded} frames, expected {FRAMES}")


def main() -> None:
    if OUT.is_file():
        verify()
        print(
            f"{OUT.name} already generated ({OUT.stat().st_size} bytes, "
            f"sha256 {hashlib.sha256(OUT.read_bytes()).hexdigest()[:12]}…, "
            f"{FRAMES} frames verified)"
        )
        return

    ffmpeg = imageio_ffmpeg.get_ffmpeg_exe()
    # Deterministic moving content: `testsrc2` is a synthetic source, so the
    # nine frames carry real inter-frame change for the decoder to deliver.
    subprocess.run(
        [
            ffmpeg,
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            f"testsrc2=size={WIDTH}x{HEIGHT}:rate={RATE}",
            "-frames:v",
            str(FRAMES),
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "libaom-av1",
            "-cpu-used",
            "8",
            "-crf",
            "40",
            str(OUT),
        ],
        check=True,
    )

    verify()
    print(
        f"wrote {OUT.name} ({OUT.stat().st_size} bytes, "
        f"sha256 {hashlib.sha256(OUT.read_bytes()).hexdigest()[:12]}…, "
        f"{FRAMES} frames verified)"
    )


if __name__ == "__main__":
    sys.exit(main())
