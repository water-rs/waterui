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
        if machine not in ("arm64", "aarch64"):
            raise RuntimeError(
                f"unsupported target: Intel macOS "
                f"({machine}-apple-darwin) is retired — Apple support "
                "is ARM64-only")
        return "aarch64-apple-darwin"
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


# Generated-but-uncommitted RN app files: the community CLI `init` template
# files the committed app does not keep. The sha256 over
# relpath+content-sha256 of exactly this set is the template lock —
# re-resolution to different content fails loudly (M4).
_RN_TEMPLATE_SKIP_DIRS = {".git", "android", "node_modules"}
_RN_TEMPLATE_SKIP_FILES = {"App.tsx", "package.json", "package-lock.json"}


def rn_template_digest(gen: Path) -> tuple[str, list[str]]:
    """sha256 over relpath+content of the generated RN template files the
    app dir materializes — root files AND the ios/ subtree (the init
    template's own files), minus the authored set. Committed authored
    files always win at materialization, so the digest may cover files
    the checkout keeps (it only pins what the generator emits)."""
    gen = Path(gen)
    files = []
    digest = hashlib.sha256()
    for f in sorted(gen.rglob("*")):
        rel = f.relative_to(gen)
        if rel.parts[0] in _RN_TEMPLATE_SKIP_DIRS \
                or str(rel) in _RN_TEMPLATE_SKIP_FILES or not f.is_file():
            continue
        files.append(str(rel))
        digest.update(str(rel).encode() + b"\0"
                      + hashlib.sha256(f.read_bytes()).digest())
    return digest.hexdigest(), files


def generate_rn_template(cli_version: str, rn_version: str,
                         work_dir: Path, env: dict | None = None,
                         run=None) -> Path:
    """Run the pinned `@react-native-community/cli init` in `work_dir` and
    return the generated app dir."""
    r = (run or _run)(
        ["npx", f"@react-native-community/cli@{cli_version}", "init",
         "RnBench", "--version", rn_version, "--skip-install",
         "--directory", str(Path(work_dir) / "RnBench")], cwd=work_dir,
        env=env)
    if r.returncode != 0:
        raise RuntimeError(
            f"react-native init failed ({r.returncode}): "
            f"{(r.stderr or r.stdout).strip()[-500:]}")
    return Path(work_dir) / "RnBench"


def ensure_rn_template(app_dir: Path, cli_version: str, rn_version: str,
                       template_sha256: str, env: dict | None = None,
                       run=None):
    """Materialize the RN `init` template files the committed app does
    not keep (index.js, app.json, babel/metro/jest config, Gemfile…).

    cli_version + rn_version + `template_sha256` pin the generator and
    its output: a re-resolution that yields different content fails
    loudly instead of silently changing the app. Committed authored
    files always win — generated files only fill paths the app does not
    already have. `.bench-generator` records the tag and the generated
    file list; a stale tag deletes them and regenerates.
    """
    import tempfile
    app_dir = Path(app_dir)
    stamp = app_dir / ".bench-generator"
    tag = f"rn-template {cli_version} {rn_version} {template_sha256}"
    if stamp.exists() and stamp.read_text().splitlines()[0] == tag:
        return
    if stamp.exists():
        for rel in stamp.read_text().splitlines()[1:]:
            (app_dir / rel).unlink(missing_ok=True)
        stamp.unlink()
    with tempfile.TemporaryDirectory() as td:
        gen = generate_rn_template(cli_version, rn_version, Path(td),
                                   env=env, run=run)
        digest, files = rn_template_digest(gen)
        if digest != template_sha256:
            raise RuntimeError(
                f"RN template digest {digest} != declared "
                f"{template_sha256} (cli {cli_version}, rn {rn_version}): "
                "the generator re-resolved to different content")
        written = []
        for rel in files:
            dst = app_dir / rel
            if dst.exists():
                continue  # committed authored file wins
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(gen / rel, dst)
            written.append(rel)
        stamp.write_text(tag + "\n" + "\n".join(written) + "\n")


def require_version(name: str, cmd: list[str],
                    expect: str | list[str],
                    env: dict | None = None) -> str:
    """Run a tool's version probe; fail unless the reported tokens
    satisfy the declared `expect` — a single string matches one token,
    a list requires EVERY element to match some token (e.g. both
    'Flutter 3.35.4' and 'Dart 3.9.2' in `flutter --version`). Matching
    is EXACT over the declared components (major-only '21' requires a
    report whose major is 21; '21.0.12' requires all three). Never a
    substring match."""
    out = _run(cmd, env=env)
    if out.returncode != 0:
        raise RuntimeError(
            f"{name} version probe failed ({out.returncode}): "
            f"{(out.stderr or out.stdout).strip()[:300]}")
    text = (out.stdout or "") + "\n" + (out.stderr or "")
    wants = expect if isinstance(expect, list) else [expect]
    reported = [tuple(int(p) for p in m.group(1).split("."))
                for m in _VERSION_TOKEN.finditer(text)]
    for e in wants:
        want = _parse_version_tuple(e)
        if want is None:
            raise RuntimeError(
                f"manifest {name} version pin is not a version: {e!r}")
        if not any(len(got) >= len(want) and got[: len(want)] == want
                   for got in reported):
            raise RuntimeError(
                f"{name} reports {reported or text.strip().splitlines()[0]!r} — "
                f"manifest declares {e!r}. Fix the tool or the pin.")
    return text.strip()


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


_GENERATED_CHURN = (
    # CocoaPods rewrites these on every `pod install` (lockfile checksum
    # normalization, project.pbxproj file-reference re-sort). They are
    # committed for reproducibility but that generated churn does not
    # change the source identity the checkout gate protects, so it does
    # not count as an uncommitted change. Regenerate and commit them on
    # macOS whenever the Podfile changes.
    "benchmarks/competitive/apps/react-native/ios/Podfile.lock",
    "benchmarks/competitive/apps/react-native/ios/RnBench.xcodeproj/"
    "project.pbxproj",
    "benchmarks/competitive/apps/react-native/macos/Podfile.lock",
    "benchmarks/competitive/apps/react-native/macos/RnBench.xcodeproj/"
    "project.pbxproj",
)


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
    dirty = [ln for ln in out.stdout.splitlines()
             if ln.strip() and ln[3:] not in _GENERATED_CHURN]
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
    locked = False
    try:
        if sys.platform.startswith("win"):
            import msvcrt
            # LK_LOCK gives up after ~10 s, far shorter than a CLI build;
            # poll the non-blocking form on a build-sized deadline instead.
            deadline = time.monotonic() + 900
            while True:
                try:
                    msvcrt.locking(fd, msvcrt.LK_NBLCK, 1)
                    locked = True
                    break
                except OSError:
                    if time.monotonic() >= deadline:
                        raise TimeoutError(
                            f"water-cli build lock still held after 900 s: "
                            f"{cache_dir}") from None
                    time.sleep(0.5)
        else:
            import fcntl
            fcntl.flock(fd, fcntl.LOCK_EX)
            locked = True
        yield
    finally:
        try:
            if locked:
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

    # require_version: exact parsed-tuple match, never substring.
    # The fake tool is a python script invoked via sys.executable —
    # runnable on Windows too (a #!/bin/sh script is not).
    fake = tempfile.NamedTemporaryFile("w", suffix=".py", delete=False)
    fake.write("print('water 0.4.4')\n")
    fake.close()
    fake_cmd = [sys.executable, fake.name, "--version"]
    assert "0.4.4" in require_version("water CLI", fake_cmd, "0.4.4")
    # major-only pin matches the same major, exactly
    assert require_version("java", [sys.executable, fake.name], "0")
    try:
        # 0.4.44 must NOT satisfy a 0.4.4 pin via substring
        require_version("water CLI", fake_cmd, "0.4.44")
    except RuntimeError as e:
        assert "manifest declares" in str(e)
    else:
        raise AssertionError("version mismatch accepted")
    try:
        require_version("water CLI", fake_cmd, "9.9.9")
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

    head = {"v": "b" * 40}
    calls = []

    def fake_run(cmd, env=None):
        calls.append(list(map(str, cmd)))
        if cmd[0] == "git" and "status" in cmd:
            return Out("")
        if cmd[0] == "git" and "rev-parse" in cmd:
            return Out(head["v"] + "\n")
        if cmd[0] == "cargo":
            cli_root = Path(str(cmd[cmd.index("--root") + 1]))
            (cli_root / "bin").mkdir(parents=True, exist_ok=True)
            shutil.copy(fake.name, cli_root / "bin" / "water")
            return Out()
        if cmd[0].endswith("water"):
            return Out("water 0.4.4\n")
        return Out()

    with tempfile.TemporaryDirectory() as td:
        root = (Path(td) / "checkout").resolve()
        (root / "cli").mkdir(parents=True)
        (root / "Cargo.toml").write_text('android-backend-revision = "%s"' % ("c" * 40))
        exe = provision_water_cli(root, Path(td) / "cache", run=fake_run)
        assert exe.name == "water" and exe.exists()
        cargo_cmd = next(c for c in calls if c[0] == "cargo")
        assert cargo_cmd[:3] == ["cargo", "install", "--locked"]
        assert str(root / "cli") in cargo_cmd
        # provenance stamp records head identity + binary hash
        prov = json.loads((exe.parent.parent / ".provenance").read_text())
        assert prov["head"] == head["v"] and prov["sha256"] == sha256_file(exe)
        # second call: stamp verifies -> no rebuild (git probes still run)
        cargo_calls = lambda: [c for c in calls if c[0] == "cargo"]
        n = len(cargo_calls())
        provision_water_cli(root, Path(td) / "cache", run=fake_run)
        assert len(cargo_calls()) == n
        # tampered binary: stamp hash fails -> rebuild
        exe.write_bytes(b"tampered")
        provision_water_cli(root, Path(td) / "cache", run=fake_run)
        assert len(cargo_calls()) == n + 1
        # a new checkout HEAD provisions a fresh root — the previous
        # build is never reused for a different framework identity
        head["v"] = "c" * 40
        exe2 = provision_water_cli(root, Path(td) / "cache", run=fake_run)
        assert exe2 != exe and exe2.exists()
        assert len(cargo_calls()) == n + 2
        assert "water-cli-cccccccccccc" in str(exe2)

    # the suite-shared build lock serializes concurrent legs: while one
    # holds it, another provision cannot enter the build section
    if not sys.platform.startswith("win"):
        import fcntl
        import threading
        with tempfile.TemporaryDirectory() as td:
            root = (Path(td) / "checkout").resolve()
            (root / "cli").mkdir(parents=True)
            (root / "Cargo.toml").write_text('x = 1')
            cache = Path(td) / "cache"
            cache.mkdir()
            lock_fd = os.open(cache / "water-cli.lock",
                              os.O_CREAT | os.O_RDWR)
            fcntl.flock(lock_fd, fcntl.LOCK_EX)
            done = []
            t = threading.Thread(
                target=lambda: done.append(
                    provision_water_cli(root, cache, run=fake_run)))
            t.start()
            t.join(0.5)
            assert not done, "provision ran without the build lock"
            fcntl.flock(lock_fd, fcntl.LOCK_UN)
            os.close(lock_fd)
            t.join(10)
            assert done and done[0].exists()

    # dirty checkout refused before any build
    def dirty_run(cmd, env=None):
        if cmd[0] == "git" and "status" in cmd:
            return Out(" M cli/main.rs\n")
        return fake_run(cmd, env)
    with tempfile.TemporaryDirectory() as td:
        root = (Path(td) / "checkout").resolve()
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
