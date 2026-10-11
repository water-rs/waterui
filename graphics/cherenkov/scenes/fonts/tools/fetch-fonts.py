#!/usr/bin/env python3
"""Fetch the pinned upstream font inputs into ``scenes/fonts/_full/``.

Every file is a ``(url, sha256)`` pair. A file already present with a
matching hash is kept, so the directory doubles as the CI cache target
(keyed on this file). A hash mismatch fails the run — newer bytes are
never substituted for the pin.
"""

from __future__ import annotations

import hashlib
import shutil
import sys
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
FULL = HERE / "_full"

GOOGLE_FONTS = (
    "https://raw.githubusercontent.com/google/fonts"
    "/9710da1eacb3be272583c3224dcb70f9da6eadbb"
)
NOTO_EMOJI = (
    "https://raw.githubusercontent.com/googlefonts/noto-emoji"
    "/e20cbc2bbec1926686be9f9bee7d1d2cfa1fea0e"
)

# (file name under _full/, url, sha256)
FILES = [
    (
        "OFL.txt",
        f"{GOOGLE_FONTS}/ofl/notosans/OFL.txt",
        "cee9892f9f0cc8fe882c9e9537ee6a89621d86ee7ceaf70b02e2b2b1c25c061a",
    ),
    (
        "Nabla_EDPT_EHLT_.ttf",
        f"{GOOGLE_FONTS}/ofl/nabla/Nabla%5BEDPT,EHLT%5D.ttf",
        "e45cec60eb2099b4b4ffee8ebe005d1d3060771071ede1caeeeb12e37a2c00ae",
    ),
    (
        "NotoColorEmoji.ttf",
        f"{NOTO_EMOJI}/2D/fonts/NotoColorEmoji.ttf",
        "15671215ab769fdc7162a045d56fd7d7e477c51b04e6b3c761d914d8fdd6cc44",
    ),
    (
        "NotoEmoji_wght_.ttf",
        f"{GOOGLE_FONTS}/ofl/notoemoji/NotoEmoji%5Bwght%5D.ttf",
        "de6c18832938afc99caf132b39d6a30a19bac7f2e812e28db2535b4608d27551",
    ),
    (
        "NotoSansArabic_wdth_wght_.ttf",
        f"{GOOGLE_FONTS}/ofl/notosansarabic/NotoSansArabic%5Bwdth,wght%5D.ttf",
        "63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e",
    ),
    (
        "NotoSansDevanagari_wdth_wght_.ttf",
        f"{GOOGLE_FONTS}/ofl/notosansdevanagari/"
        "NotoSansDevanagari%5Bwdth,wght%5D.ttf",
        "14ec4af41f27482216d1c2229f417ff9b1425e1babb014e57d1d40d03229853e",
    ),
    (
        "NotoSansHebrew_wdth_wght_.ttf",
        f"{GOOGLE_FONTS}/ofl/notosanshebrew/NotoSansHebrew%5Bwdth,wght%5D.ttf",
        "7ef36a2c3593758cdb622e1bdef4f84523e92fbc3ccc667438dd80ff54c2de88",
    ),
    (
        "NotoSansSC_wght_.ttf",
        f"{GOOGLE_FONTS}/ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf",
        "a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da",
    ),
    (
        "NotoSansThai_wdth_wght_.ttf",
        f"{GOOGLE_FONTS}/ofl/notosansthai/NotoSansThai%5Bwdth,wght%5D.ttf",
        "5a1c559bb539583c8a1fd99d1c5b9491e5e14478c9cd2bd0970d5c3096cc9ef8",
    ),
    (
        "NotoSans_wdth_wght_.ttf",
        f"{GOOGLE_FONTS}/ofl/notosans/NotoSans%5Bwdth,wght%5D.ttf",
        "bfb7bb691513f12e734dc346c03a03f784912432d7e3fa8e56efcf906fe86b3d",
    ),
]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def fetch(name: str, url: str, want: str) -> None:
    dst = FULL / name
    if dst.exists():
        got = sha256(dst)
        if got == want:
            print(f"cached  {name}")
            return
        print(f"{name}: stale ({got[:12]}), refetching")
        dst.unlink()
    tmp = dst.with_name(dst.name + ".tmp")
    with urllib.request.urlopen(url) as response, tmp.open("wb") as out:
        shutil.copyfileobj(response, out)
    got = sha256(tmp)
    if got != want:
        tmp.unlink()
        raise SystemExit(
            f"{name}: sha256 mismatch\n  url:  {url}\n  want: {want}\n  got:  {got}"
        )
    tmp.rename(dst)
    print(f"fetched {name} ({got[:12]})")


def main() -> int:
    FULL.mkdir(exist_ok=True)
    for name, url, want in FILES:
        fetch(name, url, want)
    return 0


if __name__ == "__main__":
    sys.exit(main())
