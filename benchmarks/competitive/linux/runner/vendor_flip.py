#!/usr/bin/env python3
"""Flip the waterui-hydrolysis contestant between its committed remote pins and
vendored bare mirrors under vendor/ for measuring unpushed commits.

Usage: vendor_flip.py on|off
  on  — rewrite git URLs to file:///bench/vendor/<repo>.git and the revs that
        the r9 branches carry (vello 97532103, hydrolysis 6ece906,
        waterui c9e935f0, map-gpu b3eb359e). Any entry whose URL is
        water-rs/waterui rides the waterui mirror (workspace members).
        Both manifests are rewritten: the app crate's Cargo.toml and the
        managed backend crate's backends/hydrolysis/Cargo.toml (backed up to
        Cargo.toml.flip-orig). The backend workspace lock is regenerated in
        the bench container and a .vendored sentinel is written so the
        runner builds the backend crate with cargo directly — `water build`
        certifies dev-channel pins and cannot resolve unpushed revisions.
  off — restore the committed manifest and the managed crate backup.

The vendored mirrors contain the exact commits; everything else (nami,
waterkit, hydrolysis-m3, barcode, canvas, chart, dew, block) stays on its
committed git pin. The second vello rev a01e5039 (vello_hybrid/common/
sparse_shaders/glifo) keeps its rev on the vendored mirror.
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOML = ROOT / "contestants/waterui-hydrolysis/Cargo.toml"
BACKEND_DIR = ROOT / "contestants/waterui-hydrolysis/backends/hydrolysis"
BACKEND_TOML = BACKEND_DIR / "Cargo.toml"
BACKEND_ORIG = BACKEND_DIR / "Cargo.toml.flip-orig"
SENTINEL = BACKEND_DIR / ".vendored"

URL_TO_REPO = {
    "https://github.com/water-rs/waterui": ("waterui", "c9e935f05b5add88d5968e9271ed614164ba3b25"),
    "https://github.com/water-rs/hydrolysis": ("hydrolysis", "6ece906832d77d344f1f4cb78a216a2fa659fca0"),
    "https://github.com/water-rs/map-gpu": ("map-gpu", "b3eb359e5313ec5152658fb35f95064a11400714"),
    "https://github.com/lexoliu/vello": ("vello", None),  # rev: per-section
}
VELLO_PRIMARY = "975321037d357bbf6676d51db8467b33f0b4d22a"


def flip_on(text: str) -> str:
    out = []
    section = ""
    pending_repo = None
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("["):
            section = s
            pending_repo = None
            out.append(line)
            continue
        m = re.match(r'git\s*=\s*"([^"]+)"', s)
        hit = m is not None and m.group(1) in URL_TO_REPO and m.group(1) or None
        if hit is None and m:
            # Idempotent re-flip: an already-vendored URL names its repo too.
            v = re.fullmatch(r'file:///bench/vendor/(\w[\w-]*)\.git', m.group(1))
            if v and v.group(1) in URL_TO_REPO_INV:
                hit = m.group(1)
        if hit is not None:
            repo, _ = URL_TO_REPO.get(hit, (None, None))
            if repo is None:
                repo = re.fullmatch(r'file:///bench/vendor/(\w[\w-]*)\.git', hit).group(1)
            pending_repo = repo
            out.append(f'git = "file:///bench/vendor/{repo}.git"')
            continue
        if re.match(r'rev\s*=\s*"', s) and pending_repo:
            _, rev = URL_TO_REPO_INV[pending_repo]
            if pending_repo == "vello" and section == "[patch.crates-io.vello]":
                rev = VELLO_PRIMARY
            if rev:
                out.append(f'rev = "{rev}"')
                continue
        out.append(line)
    return "\n".join(out) + "\n"


URL_TO_REPO_INV = {repo: (url, rev) for url, (repo, rev) in URL_TO_REPO.items()}


def docker_bash(script: str) -> None:
    subprocess.run(
        ["docker", "run", "--rm",
         "-v", f"{ROOT}:/bench",
         "-v", f"{ROOT}/.cache/cargo:/cargo-home",
         "-e", "HOME=/root", "-e", "CARGO_HOME=/cargo-home",
         "waterui-bench-linux", "bash", "-c", script],
        check=True)


def check_flipped(text: str) -> None:
    n_v = text.count('git = "file:///bench/vendor/vello.git"')
    n_w = text.count('git = "file:///bench/vendor/waterui.git"')
    assert text.count(f'rev = "{VELLO_PRIMARY}"') == 1
    assert n_v >= 4 and n_w >= 20, (n_v, n_w)
    for repo, (_, rev) in URL_TO_REPO_INV.items():
        if rev:
            assert f'rev = "{rev}"' in text, repo
    assert 'rev = "7f5e1ca' not in text and 'rev = "06003fd' not in text
    assert 'rev = "8ad539f8' not in text and 'rev = "74d7dcd' not in text
    # A previous round's revs must not survive a re-flip.
    assert '64254396' not in text and '3daba12d' not in text
    assert '0cc47429' not in text and '892746e3' not in text
    assert 'a51c4058' not in text and 'd3c8b239' not in text
    assert '50d7411' not in text and 'e812f0f7' not in text
    assert '8e584495' not in text and '93a676de' not in text
    assert 'd0f1af22' not in text
    assert '546841a3' not in text and 'ed884c0a' not in text
    assert '5d561681' not in text and 'def05de9' not in text
    assert '76724fcf' not in text and '341a286f' not in text
    assert '68ec29a5' not in text and '6415cedf' not in text
    print(f"flipped: {n_w} waterui entries, {n_v} vello entries")


def main() -> int:
    if sys.argv[1] == "off":
        subprocess.run(["git", "checkout", "--",
                        "contestants/waterui-hydrolysis/Cargo.toml"],
                       cwd=ROOT, check=True)
        if BACKEND_ORIG.exists():
            docker_bash("cd /bench/contestants/waterui-hydrolysis/backends/hydrolysis"
                        " && cp Cargo.toml.flip-orig Cargo.toml"
                        " && rm -f Cargo.toml.flip-orig .vendored Cargo.lock")
        # The app-level Cargo.lock was regenerated against vendored sources;
        # water build --locked would reject it.
        (ROOT / "contestants/waterui-hydrolysis/Cargo.lock").unlink(missing_ok=True)
        print("restored committed manifest")
        return 0
    if sys.argv[1] == "on":
        subprocess.run(["git", "checkout", "--",
                        "contestants/waterui-hydrolysis/Cargo.toml"],
                       cwd=ROOT, check=True)
        TOML.write_text(flip_on(TOML.read_text()))
        check_flipped(TOML.read_text())
        # The managed backend dir is root-owned (written by the container);
        # stage the flipped manifest host-side, then move it in as root.
        staged = ROOT / "contestants/waterui-hydrolysis/Cargo.toml.vendored"
        staged.write_text(flip_on(BACKEND_TOML.read_text()))
        docker_bash(
            "cd /bench/contestants/waterui-hydrolysis"
            " && D=backends/hydrolysis"
            " && [ -f $D/Cargo.toml.flip-orig ] || cp $D/Cargo.toml $D/Cargo.toml.flip-orig"
            " && cp Cargo.toml.vendored $D/Cargo.toml"
            " && rm -f Cargo.toml.vendored $D/Cargo.lock"
            " && touch $D/.vendored")
        return 0
    print("usage: vendor_flip.py on|off", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
