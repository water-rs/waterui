#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["av==19.0.1", "imageio-ffmpeg==0.6.0"]
# ///
"""Generate a live photo's motion clip — 96x64, 9 frames at 12fps, AV1 in
MP4 — at the output path given on the command line. Hydrolysis's
`tests/media.rs` and waterui-media's `tests/e2e_semantics.rs` each read
the clip from their own `tests/fixtures/live-photo-motion.mp4`, which the
nextest setup scripts in `.config/nextest.toml` produce through this one
generator.

    uv run backends/hydrolysis/tests/fixtures/generate-live-photo-motion.py <output.mp4>

Encoding runs through a pinned, hash-verified static `ffmpeg`
(`imageio-ffmpeg`'s bundled binary — libaom-av1 inside, no system ffmpeg
dependency, identical on every CI platform), and the output is verified by
decoding it back through dav1d and counting frames: the test accepts the
fixture by reading it, so a fixture that does not decode to the documented
geometry is a hard failure here rather than a skipped assertion there.
The generated file stays out of the repository (see the repository's
no-binary-assets rule); each `live-photo-motion.mp4` is gitignored next to
the suite that reads it.
"""

from __future__ import annotations

import hashlib
import subprocess
import sys
from pathlib import Path

import av
import imageio_ffmpeg

WIDTH, HEIGHT, FRAMES, RATE = 96, 64, 9, 12


def verify(out: Path) -> None:
    """Read-back gate: exactly FRAMES frames of WIDTHxHEIGHT through dav1d."""
    try:
        decoded = 0
        with av.open(str(out)) as container:
            stream = container.streams.video[0]
            assert stream.codec_context.name == "libdav1d", stream.codec_context.name
            for frame in container.decode(video=0):
                assert (frame.width, frame.height) == (WIDTH, HEIGHT), (
                    frame.width,
                    frame.height,
                )
                decoded += 1
    except av.error.FFmpegError as error:
        raise SystemExit(f"{out.name}: not a readable AV1 clip: {error}") from error
    if decoded != FRAMES:
        raise SystemExit(f"decoded {decoded} frames, expected {FRAMES}")


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(sys.argv[0]).name} <output.mp4>")
    out = Path(sys.argv[1]).resolve()

    if out.is_file():
        verify(out)
        print(
            f"{out.name} already generated ({out.stat().st_size} bytes, "
            f"sha256 {hashlib.sha256(out.read_bytes()).hexdigest()[:12]}…, "
            f"{FRAMES} frames verified)"
        )
        return

    out.parent.mkdir(parents=True, exist_ok=True)

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
            str(out),
        ],
        check=True,
    )

    verify(out)
    print(
        f"wrote {out.name} ({out.stat().st_size} bytes, "
        f"sha256 {hashlib.sha256(out.read_bytes()).hexdigest()[:12]}…, "
        f"{FRAMES} frames verified)"
    )


if __name__ == "__main__":
    sys.exit(main())
