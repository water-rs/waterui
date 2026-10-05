r"""Shared declared-toolchain resolution for the competitive suite runners.

Each leg's runner resolves its toolchain ONLY from the manifest's declared
pins — never from a probe order over candidate paths on the host. The water
CLI is different in kind from the other tools: it is a workspace member of
the same checkout (`cli/`), so its identity is the checkout's HEAD sha — the
same commit that pins the framework crates, the Apple backend, Hydrolysis,
and `android-backend-revision`:

- `checkout_head` / `require_clean_checkout` establish and record that
  identity; a dirty tracked tree refuses to provision.
- `provision_water_cli` runs `cargo install --locked --path cli --root
  <runner-owned>` into the suite-shared `.cache/toolchain/` once per
  checkout-sha + host target, under a file lock so concurrently running
  legs cannot race the same build.
- `android_backend_revision` reads the `android-backend-revision` pin from
  the root Cargo.toml — the Kotlin runtime identity when the Android
  `android` backend is used.
- `require_version` runs a tool's version probe and compares PARSED version
  tuples exactly — never substring matching: each `\d+(\.\d+)*` token in the
  tool's output is parsed; the check passes iff some token's leading
  components equal the declared tuple component-wise.
- `require_dir`/`require_file` resolve declared `~`-paths and fail when
  they do not exist.
- `fetch_verified` downloads a pinned artifact only after SHA256 proof.

`--self-test` exercises the failure directions without network or a real
toolchain.
"""

from __future__ import annotations

import contextlib
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path


# One toolchain cache for the whole suite — a CLI at checkout sha H for
# target T is built once here, never rebuilt per leg.
SHARED_CACHE = Path(__file__).resolve().parent.parent / ".cache" / "toolchain"


def repo_root() -> Path:
    """The waterui checkout this suite lives in (benchmarks/competitive/lib)."""
    return Path(__file__).resolve().parents[3]


def host_target() -> str:
    """Rust target triple of this host — cache key component."""
    machine = platform.machine().lower()
    if sys.platform.startswith("win"):
        return f"{machine.replace('amd64', 'x86_64')}-pc-windows-msvc"
    if sys.platform == "darwin":
        arch = "aarch64" if machine in ("arm64", "aarch64") else "x86_64"
        return f"{arch}-apple-darwin"
    return f"{machine.replace('amd64', 'x86_64')}-unknown-linux-gnu"


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def require_dir(name: str, declared: str) -> Path:
    """Resolve a manifest-declared `~`-path; it must exist."""
    p = Path(os.path.expanduser(declared))
    if not p.is_dir():
        raise RuntimeError(
            f"manifest-declared {name} does not exist: {p} "
            "(fix the pin or provision the tool)")
    return p


def require_file(name: str, path: Path) -> Path:
    if not path.is_file():
        raise RuntimeError(f"manifest-declared {name} missing: {path}")
    return path


def _run(cmd: list[str], env: dict | None = None,
         cwd: Path | None = None) -> subprocess.CompletedProcess:
    return subprocess.run([str(c) for c in cmd], capture_output=True,
                          text=True, env=env, cwd=cwd)


_VERSION_TOKEN = re.compile(r"(?<![\d.])(\d+(?:\.\d+)*)(?![\d.])")


def _parse_version_tuple(text: str) -> tuple[int, ...] | None:
    """First standalone dotted version token in `text`, as an int tuple."""
    m = _VERSION_TOKEN.search(text)
    return tuple(int(p) for p in m.group(1).split(".")) if m else None


def require_version(name: str, cmd: list[str], expect: str,
                    env: dict | None = None) -> str:
    """Run a tool's version probe; fail unless a reported version token
    matches the declared `expect` tuple EXACTLY over the declared
    components (major-only '21' requires a report whose major is 21;
    '21.0.12' requires all three). Never a substring match."""
    out = _run(cmd, env=env)
    if out.returncode != 0:
        raise RuntimeError(
            f"{name} version probe failed ({out.returncode}): "
            f"{(out.stderr or out.stdout).strip()[:300]}")
    text = (out.stdout or "") + "\n" + (out.stderr or "")
    want = _parse_version_tuple(expect)
    if want is None:
        raise RuntimeError(f"manifest {name} version pin is not a version: "
                           f"{expect!r}")
    reported = [tuple(int(p) for p in m.group(1).split("."))
                for m in _VERSION_TOKEN.finditer(text)]
    for got in reported:
        if len(got) >= len(want) and got[: len(want)] == want:
            return text.strip()
    raise RuntimeError(
        f"{name} reports {reported or text.strip().splitlines()[0]!r} — "
        f"manifest declares {expect!r}. Fix the tool or the pin.")


# ---------------------------------------------------------------------------
# In-tree water CLI: identity is the checkout's HEAD sha
# ---------------------------------------------------------------------------

def checkout_head(root: Path | None = None, run=None) -> str:
    """HEAD sha of the waterui checkout — the framework+CLI+backend
    identity recorded for every run."""
    root = root or repo_root()
    out = (run or _run)(["git", "-C", str(root), "rev-parse", "HEAD"])
    if out.returncode != 0:
        raise RuntimeError(
            f"cannot resolve checkout HEAD at {root}: "
            f"{(out.stderr or '').strip()[:200]}")
    return out.stdout.strip()


def require_clean_checkout(root: Path | None = None, run=None) -> Path:
    """Refuse to build the contestant/toolchain from a checkout with
    uncommitted tracked changes — HEAD sha would then mislabel the actual
    source. Untracked files do not affect built artifacts and are allowed.
    """
    root = Path(root or repo_root()).resolve()
    out = (run or _run)(
        ["git", "-C", str(root), "status", "--porcelain",
         "--untracked-files=no"])
    if out.returncode != 0:
        raise RuntimeError(
            f"cannot check checkout cleanliness at {root}: "
            f"{(out.stderr or '').strip()[:200]}")
    dirty = [ln for ln in out.stdout.splitlines() if ln.strip()]
    if dirty:
        shown = "\n  ".join(dirty[:20])
        raise RuntimeError(
            f"checkout has {len(dirty)} uncommitted tracked change(s); the "
            f"HEAD-sha identity would mislabel the source — commit or "
            f"stash first:\n  {shown}")
    return root


def android_backend_revision(root: Path | None = None) -> str:
    """The `android-backend-revision` pin the root manifest declares —
    the Kotlin-runtime identity of the Android `android` backend. Read
    with tomllib from [package.metadata.waterui], never by regex."""
    import tomllib
    root = Path(root or repo_root())
    try:
        meta = tomllib.loads(
            (root / "Cargo.toml").read_text())["package"]["metadata"]["waterui"]
    except KeyError as exc:
        raise RuntimeError(
            f"no [package.metadata.waterui] table in {root / 'Cargo.toml'}") \
            from exc
    rev = meta.get("android-backend-revision")
    if not (isinstance(rev, str) and re.fullmatch(r"[0-9a-f]{40}", rev)):
        raise RuntimeError(
            f"no valid android-backend-revision pin in {root / 'Cargo.toml'}")
    return rev


@contextlib.contextmanager
def _build_lock(cache_dir: Path):
    """Serialize CLI builds across concurrently running legs."""
    cache_dir.mkdir(parents=True, exist_ok=True)
    fd = os.open(cache_dir / "water-cli.lock", os.O_CREAT | os.O_RDWR)
    try:
        if sys.platform.startswith("win"):
            import msvcrt
            msvcrt.locking(fd, msvcrt.LK_LOCK, 1)
        else:
            import fcntl
            fcntl.flock(fd, fcntl.LOCK_EX)
        yield
    finally:
        try:
            if sys.platform.startswith("win"):
                os.lseek(fd, 0, os.SEEK_SET)
                msvcrt.locking(fd, msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(fd, fcntl.LOCK_UN)
        finally:
            os.close(fd)


def provision_water_cli(root: Path | None = None,
                        cache_dir: Path | None = None,
                        run=None) -> Path:
    """Build the water CLI from THIS checkout once per host into a
    runner-owned root.

    Identity = checkout HEAD sha over a clean tracked tree. The binary is
    `cargo install --locked --path cli --root <cache>/water-cli-<head>-<tgt>`,
    taken under a suite-shared file lock so concurrent legs do not race a
    partial install. A `.provenance` stamp beside the binary records head,
    target, reported version, and the binary's sha256; a stamp that no
    longer matches the binary triggers a rebuild.

    `run` is injectable for --self-test: it replaces _run and must create
    the binary at root/bin/water.
    """
    root = require_clean_checkout(root, run=run)
    head = checkout_head(root, run=run)
    target = host_target()
    cache_dir = cache_dir or SHARED_CACHE
    exe_name = "water.exe" if "windows" in target else "water"
    cli_root = cache_dir / f"water-cli-{head[:12]}-{target}"
    exe = cli_root / "bin" / exe_name
    stamp = cli_root / ".provenance"
    run_fn = run or _run

    def verify_stamped() -> bool:
        if not (exe.exists() and stamp.exists()):
            return False
        try:
            prov = json.loads(stamp.read_text())
        except ValueError:
            return False
        return (prov.get("head") == head and prov.get("target") == target
                and prov.get("sha256") == sha256_file(exe))

    if not verify_stamped():
        with _build_lock(cache_dir):
            # Re-check under the lock: another leg may have built while
            # we waited.
            if not verify_stamped():
                if run is None and shutil.which("cargo") is None:
                    raise RuntimeError(
                        "cargo is required to build the water CLI from "
                        "this checkout (cli/ is a workspace member)")
                out = run_fn(["cargo", "install", "--locked",
                              "--path", str(root / "cli"),
                              "--root", str(cli_root)])
                if out.returncode != 0:
                    raise RuntimeError(
                        f"cargo install of the in-tree water CLI failed "
                        f"({out.returncode}):\n"
                        f"{(out.stderr or out.stdout)[-1200:]}")
                if not exe.exists():
                    raise RuntimeError(
                        f"cargo install completed but {exe} is missing")
                ver = run_fn([str(exe), "--version"])
                stamp.write_text(json.dumps({
                    "kind": "cargo-install --path",
                    "head": head,
                    "target": target,
                    "version": (ver.stdout or ver.stderr).strip(),
                    "sha256": sha256_file(exe)}))
    return exe


def fetch_verified(url: str, sha256: str, dest: Path,
                   fetch=None) -> Path:
    """Download `url` to `dest` only when the bytes hash to `sha256`.

    `fetch` is injectable for --self-test (receives url + tmp path).
    """
    dest.parent.mkdir(parents=True, exist_ok=True)
    if dest.exists() and sha256_file(dest) == sha256:
        return dest
    tmp = dest.with_suffix(dest.suffix + ".tmp")
    if fetch is None:
        out = _run(["curl", "-fsSL", "-o", str(tmp), url])
        if out.returncode != 0:
            tmp.unlink(missing_ok=True)
            raise RuntimeError(
                f"download failed ({out.returncode}) for {url}")
    else:
        fetch(url, tmp)
    got = sha256_file(tmp)
    if got != sha256:
        tmp.unlink(missing_ok=True)
        raise RuntimeError(
            f"SHA256 mismatch for {url}: got {got}, "
            f"manifest pins {sha256} — refusing to use the artifact")
    tmp.rename(dest)
    return dest


def unzip_verified(zip_path: Path, dest_dir: Path,
                   top: str | None = None) -> Path:
    """Unzip a verified distribution archive once; returns its root dir."""
    if top is None:
        with zipfile.ZipFile(zip_path) as z:
            top = z.namelist()[0].split("/")[0]
    root = dest_dir / top
    if not root.exists():
        with zipfile.ZipFile(zip_path) as z:
            z.extractall(dest_dir)
    return root


# ---------------------------------------------------------------------------
# self-test: failure directions only, no network/toolchain required
# ---------------------------------------------------------------------------

def _self_test() -> None:
    import tempfile

    # require_version: exact parsed-tuple match, never substring
    fake = tempfile.NamedTemporaryFile("w", suffix=".sh", delete=False)
    fake.write("#!/bin/sh\necho 'water 0.4.4'\n")
    fake.close()
    os.chmod(fake.name, 0o755)
    assert "0.4.4" in require_version(
        "water CLI", [fake.name, "--version"], "0.4.4")
    # major-only pin matches the same major, exactly
    assert require_version("java", [fake.name], "0")
    try:
        # 0.4.44 must NOT satisfy a 0.4.4 pin via substring
        require_version("water CLI", [fake.name], "0.4.44")
    except RuntimeError as e:
        assert "manifest declares" in str(e)
    else:
        raise AssertionError("version mismatch accepted")
    try:
        require_version("water CLI", [fake.name], "9.9.9")
    except RuntimeError:
        pass
    else:
        raise AssertionError("absent version accepted")

    # provision_water_cli with an injected runner; the fake git reports a
    # clean checkout at head H and fake cargo materializes the binary.
    class Out:
        def __init__(self, stdout="", returncode=0):
            self.returncode = returncode
            self.stdout = stdout
            self.stderr = ""

    head = "b" * 40
    calls = []

    def fake_run(cmd, env=None):
        calls.append(list(map(str, cmd)))
        if cmd[0] == "git" and "status" in cmd:
            return Out("")
        if cmd[0] == "git" and "rev-parse" in cmd:
            return Out(head + "\n")
        if cmd[0] == "cargo":
            cli_root = Path(str(cmd[cmd.index("--root") + 1]))
            (cli_root / "bin").mkdir(parents=True, exist_ok=True)
            shutil.copy(fake.name, cli_root / "bin" / "water")
            return Out()
        if cmd[0].endswith("water"):
            return Out("water 0.4.4\n")
        return Out()

    with tempfile.TemporaryDirectory() as td:
        root = Path(td) / "checkout"
        (root / "cli").mkdir(parents=True)
        (root / "Cargo.toml").write_text('android-backend-revision = "%s"' % ("c" * 40))
        exe = provision_water_cli(root, Path(td) / "cache", run=fake_run)
        assert exe.name == "water" and exe.exists()
        cargo_cmd = next(c for c in calls if c[0] == "cargo")
        assert cargo_cmd[:3] == ["cargo", "install", "--locked"]
        assert str(root / "cli") in cargo_cmd
        # provenance stamp records head identity + binary hash
        prov = json.loads((exe.parent.parent / ".provenance").read_text())
        assert prov["head"] == head and prov["sha256"] == sha256_file(exe)
        # second call: stamp verifies -> no rebuild (git probes still run)
        cargo_calls = lambda: [c for c in calls if c[0] == "cargo"]
        n = len(cargo_calls())
        provision_water_cli(root, Path(td) / "cache", run=fake_run)
        assert len(cargo_calls()) == n
        # tampered binary: stamp hash fails -> rebuild
        exe.write_bytes(b"tampered")
        provision_water_cli(root, Path(td) / "cache", run=fake_run)
        assert len(cargo_calls()) == n + 1

    # dirty checkout refused before any build
    def dirty_run(cmd, env=None):
        if cmd[0] == "git" and "status" in cmd:
            return Out(" M cli/main.rs\n")
        return fake_run(cmd, env)
    with tempfile.TemporaryDirectory() as td:
        root = Path(td) / "checkout"
        (root / "cli").mkdir(parents=True)
        try:
            provision_water_cli(root, Path(td) / "cache", run=dirty_run)
        except RuntimeError as e:
            assert "uncommitted tracked change" in str(e)
        else:
            raise AssertionError("dirty checkout provisioned")

    # android-backend-revision read from the root manifest
    with tempfile.TemporaryDirectory() as td:
        (Path(td) / "Cargo.toml").write_text(
            '[package.metadata.waterui]\n'
            'android-backend-revision = "%s"\n' % ("d" * 40))
        assert android_backend_revision(Path(td)) == "d" * 40
        # a pin outside the metadata table must not be picked up
        (Path(td) / "Cargo.toml").write_text(
            'android-backend-revision = "%s"\n' % ("e" * 40))
        try:
            android_backend_revision(Path(td))
        except RuntimeError:
            pass
        else:
            raise AssertionError("top-level pin accepted")

    # fetch_verified: wrong bytes rejected, right bytes accepted
    with tempfile.TemporaryDirectory() as td:
        dest = Path(td) / "x.jar"
        def bad_fetch(url, tmp):
            tmp.write_bytes(b"forged")
        try:
            fetch_verified("http://x", "0" * 64, dest, fetch=bad_fetch)
        except RuntimeError as e:
            assert "SHA256 mismatch" in str(e)
        else:
            raise AssertionError("forged artifact accepted")
        assert not dest.exists()
        payload = b"real bytes"
        want = hashlib.sha256(payload).hexdigest()
        def good_fetch(url, tmp):
            tmp.write_bytes(payload)
        assert fetch_verified("http://x", want, dest,
                              fetch=good_fetch).read_bytes() == payload

    print("toolchain self-test ok")


if __name__ == "__main__":
    if "--self-test" in sys.argv:
        _self_test()
    else:
        print(__doc__)
