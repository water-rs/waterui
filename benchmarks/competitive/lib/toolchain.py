r"""Shared declared-toolchain resolution for the competitive suite runners.

Each leg's runner resolves its toolchain ONLY from the manifest's declared
pins — never from a probe order over candidate paths on the host. The water
CLI is different in kind from the other tools: it is a workspace member of
the same checkout (`cli/`), so its identity is the checkout's HEAD sha — the
same commit that pins the framework crates, the Apple backend, Hydrolysis,
and `android-backend-revision`:

- `checkout_head` / `require_clean_checkout` establish and record that
  identity; a dirty tracked tree refuses to provision.
- `tracked_tree_unchanged` wraps every build/bootstrap step of every leg:
  the paths the step may write (the leg's own directory and the apps it
  builds) are clean before it runs and still clean after it, or the step
  fails naming the tracked files it rewrote. A guarded WaterUI project
  (a directory with a Water.toml) brings the framework checkout its
  `waterui_path` names into the guard, so the framework source the
  contestant compiles is held as clean as the app. Paths outside them
  belong to other legs and edits elsewhere; the framework source the CLI
  builds from is held clean by `provision_water_cli`.
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

if sys.version_info < (3, 10):
    raise SystemExit(
        "benchmarks/competitive requires Python >= 3.10 "
        f"(this interpreter is {sys.version.split()[0]}); every leg "
        "declares its version in pyproject.toml + .python-version and "
        "runs under the uv-managed interpreter (`uv run`)")


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


def _generated_current(dest_dir: Path, tag: str) -> bool:
    """True when `dest_dir/.bench-generator` records `tag`. A stamp with
    another tag deletes the files it lists and itself, so the caller
    regenerates from scratch."""
    stamp = dest_dir / ".bench-generator"
    if not stamp.exists():
        return False
    lines = stamp.read_text().splitlines()
    if lines[0] == tag:
        return True
    for rel in lines[1:]:
        (dest_dir / rel).unlink(missing_ok=True)
    stamp.unlink()
    return False


def _write_generated(dest_dir: Path, tag: str, gen: Path,
                     files: list[str]) -> None:
    """Copy each generated file in `files` (relative to `gen`) into
    `dest_dir` unless the checkout already has that path — committed
    authored files always win — and stamp the tag and the written list."""
    written = []
    for rel in files:
        dst = dest_dir / rel
        if dst.exists():
            continue  # committed authored file wins
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(gen / rel, dst)
        written.append(rel)
    (dest_dir / ".bench-generator").write_text(
        tag + "\n" + "\n".join(written) + "\n")


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
    tag = f"rn-template {cli_version} {rn_version} {template_sha256}"
    if _generated_current(app_dir, tag):
        return
    with tempfile.TemporaryDirectory() as td:
        gen = generate_rn_template(cli_version, rn_version, Path(td),
                                   env=env, run=run)
        digest, files = rn_template_digest(gen)
        if digest != template_sha256:
            raise RuntimeError(
                f"RN template digest {digest} != declared "
                f"{template_sha256} (cli {cli_version}, rn {rn_version}): "
                "the generator re-resolved to different content")
        _write_generated(app_dir, tag, gen, files)


# The installed react-native-macos package's own generator entry point
# (the one its `react-native-macos-init` drives):
# generateMacOS(projectDir, name, options).
_RN_MACOS_GENERATE = (
    "require(process.argv[1])(process.argv[2], process.argv[3], "
    "{overwrite: true})")


def ensure_rn_macos_template(app_dir: Path, rn_macos_version: str,
                             env: dict | None = None, run=None):
    """Materialize the react-native-macos template files the committed
    `macos/` project does not keep (AppDelegate.h, main.m, storyboard,
    asset catalogs, entitlements, shared scheme, .gitignore).

    Runs after `npm ci`: the generator is the installed package's
    `local-cli/generate-macos.js`, whose bytes the lockfile's integrity
    hash pins, and the installed version must equal the declared
    `rn_macos_version`. The whole template is generated into a scratch
    dir under the app's package.json name; committed authored files
    (AppDelegate.mm, Info.plist, Podfile, the project) win, the rest
    fill `macos/`, and `macos/.bench-generator` records the tag and the
    written list exactly as `ensure_rn_template` does for the root and
    ios/ files.
    """
    import tempfile
    app_dir = Path(app_dir)
    pkg = app_dir / "node_modules" / "react-native-macos"
    if not (pkg / "package.json").is_file():
        raise RuntimeError(
            f"react-native-macos is not installed under {pkg} — "
            "`npm ci` must run before the macOS template step")
    installed = json.loads((pkg / "package.json").read_text())["version"]
    if installed != rn_macos_version:
        raise RuntimeError(
            f"installed react-native-macos is {installed}, the manifest "
            f"declares {rn_macos_version} — package-lock.json and the "
            "manifest pin disagree")
    name = json.loads((app_dir / "package.json").read_text())["name"]
    dest = app_dir / "macos"
    tag = f"rn-macos-template {rn_macos_version} {name}"
    if _generated_current(dest, tag):
        return
    with tempfile.TemporaryDirectory() as td:
        r = (run or _run)(
            ["node", "-e", _RN_MACOS_GENERATE,
             str(pkg / "local-cli" / "generate-macos.js"), td, name],
            env=env)
        if r.returncode != 0:
            raise RuntimeError(
                f"react-native-macos template generation failed "
                f"({r.returncode}): {(r.stderr or r.stdout).strip()[-500:]}")
        gen = Path(td) / "macos"
        files = sorted(str(f.relative_to(gen)) for f in gen.rglob("*")
                       if f.is_file())
        if not files:
            raise RuntimeError(
                f"react-native-macos generator produced no files in {gen}")
        _write_generated(dest, tag, gen, files)


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


def _dirty_tracked(root: Path, run=None,
                   paths: list[Path] | None = None) -> list[str]:
    """`git status --porcelain` lines for uncommitted tracked changes —
    over the whole checkout, or only under `paths` when given."""
    cmd = ["git", "-C", str(root), "status", "--porcelain",
           "--untracked-files=no"]
    if paths is not None:
        cmd += ["--", *(str(Path(p).resolve()) for p in paths)]
    out = (run or _run)(cmd)
    if out.returncode != 0:
        raise RuntimeError(
            f"cannot check checkout cleanliness at {root}: "
            f"{(out.stderr or '').strip()[:200]}")
    return [ln for ln in out.stdout.splitlines() if ln.strip()]


def require_clean_checkout(root: Path | None = None, run=None) -> Path:
    """Refuse to build the contestant/toolchain from a checkout with
    uncommitted tracked changes — HEAD sha would then mislabel the actual
    source. Untracked files do not affect built artifacts and are allowed.
    """
    root = Path(root or repo_root()).resolve()
    dirty = _dirty_tracked(root, run)
    if dirty:
        shown = "\n  ".join(dirty[:20])
        raise RuntimeError(
            f"checkout has {len(dirty)} uncommitted tracked change(s); the "
            f"HEAD-sha identity would mislabel the source — commit or "
            f"stash first:\n  {shown}")
    return root


def waterui_framework_root(project: Path, root: Path) -> Path | None:
    """The framework checkout the WaterUI project at `project` builds
    against — its Water.toml's `waterui_path`, resolved against the project
    — or None when `project` is not a WaterUI project. The suite labels
    WaterUI results with this checkout's HEAD, so a project that builds
    against anything else (no `waterui_path`: published crates; a path
    outside `root`: another checkout) fails."""
    import tomllib
    water_toml = project / "Water.toml"
    if not water_toml.is_file():
        return None
    rel = tomllib.loads(water_toml.read_text()).get("waterui_path")
    if not isinstance(rel, str):
        raise RuntimeError(
            f"{water_toml} declares no waterui_path — the WaterUI "
            f"contestant must build against the checkout at {root}, whose "
            "HEAD labels its results")
    framework = (project / rel).resolve()
    if framework != root and root not in framework.parents:
        raise RuntimeError(
            f"{water_toml}: waterui_path {rel!r} resolves to {framework}, "
            f"outside the checkout at {root} whose HEAD labels the results")
    return framework


def guarded_paths(paths: list[Path], root: Path) -> list[Path]:
    """`paths` plus the framework root every WaterUI project among them
    builds against: an uncommitted tracked edit to framework source enters
    that contestant's build exactly as an edit to the app does."""
    out: list[Path] = []
    for p in paths:
        p = Path(p).resolve()
        for q in (p, waterui_framework_root(p, root)):
            if q is not None and q not in out:
                out.append(q)
    return out


@contextlib.contextmanager
def tracked_tree_unchanged(label: str, paths: list[Path],
                           root: Path | None = None, run=None):
    """Every build step that can write into the checkout — bootstrap
    (`pod install`, `flutter create`, `npm ci`, wrapper materialisation)
    and the contestant build itself — runs inside this block, in every
    leg, naming the `paths` it may write: its own leg directory and the
    app projects it builds.

    Those paths must be clean BEFORE the step runs, so a dirty file there
    is reported as the user's uncommitted change and never as the step's
    output; after the step they must still be clean. A step that rewrote
    tracked files fails naming them: the committed generated files are
    stale and are regenerated with the pinned tool and committed — the
    HEAD sha never labels a tree a bootstrap changed. A WaterUI project
    among `paths` also guards the framework checkout it builds against
    (guarded_paths). Tracked changes outside the guarded paths (another
    leg's concurrent bootstrap, an edit elsewhere) are not this step's and
    never fail it."""
    if not paths:
        raise RuntimeError(f"{label}: no paths declared for the guard")
    root = Path(root or repo_root()).resolve()
    paths = guarded_paths(paths, root)
    dirty = _dirty_tracked(root, run, paths)
    if dirty:
        shown = "\n  ".join(dirty[:20])
        raise RuntimeError(
            f"{label}: {len(dirty)} uncommitted tracked change(s) under the "
            f"paths it builds — commit or stash first:\n  {shown}")
    yield root
    dirty = _dirty_tracked(root, run, paths)
    if dirty:
        shown = "\n  ".join(dirty[:20])
        raise RuntimeError(
            f"{label} rewrote {len(dirty)} tracked file(s) — the committed "
            "generated files are stale; regenerate them with the pinned "
            f"tool and commit:\n  {shown}")


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
    """Serialize CLI builds across concurrently running legs: an exclusive
    lock on <cache>/water-cli.lock that blocks in the kernel until the
    holder releases it — flock(2) on POSIX, LockFileEx without
    LOCKFILE_FAIL_IMMEDIATELY on Windows. No retry loop, no deadline: the
    holder is a build that ends, and the lock dies with its process."""
    cache_dir.mkdir(parents=True, exist_ok=True)
    path = cache_dir / "water-cli.lock"
    if sys.platform.startswith("win"):
        import pywintypes
        import win32con
        import win32file
        handle = win32file.CreateFile(
            str(path), win32con.GENERIC_READ | win32con.GENERIC_WRITE,
            win32con.FILE_SHARE_READ | win32con.FILE_SHARE_WRITE, None,
            win32con.OPEN_ALWAYS, win32con.FILE_ATTRIBUTE_NORMAL, None)
        try:
            span = pywintypes.OVERLAPPED()  # offset 0
            win32file.LockFileEx(handle, win32con.LOCKFILE_EXCLUSIVE_LOCK,
                                 1, 0, span)
            try:
                yield
            finally:
                win32file.UnlockFileEx(handle, 1, 0, span)
        finally:
            handle.Close()
    else:
        import fcntl
        fd = os.open(path, os.O_CREAT | os.O_RDWR)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX)
            try:
                yield
            finally:
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

    # bootstrap guard: clean before, clean after, over the declared paths
    # only — a step that rewrites a tracked file there fails naming it; a
    # dirty path before the step fails before the step runs; a tracked
    # change outside the paths is another leg's and never fails it
    state = {"dirty": {}}
    guard_calls = []

    def guard_run(cmd, env=None):
        if cmd[0] == "git" and "status" in cmd:
            guard_calls.append(cmd)
            scope = cmd[cmd.index("--") + 1:] if "--" in cmd else None
            lines = [line for path, line in state["dirty"].items()
                     if scope is None
                     or any(path.startswith(s + "/") for s in scope)]
            return Out("".join(lines))
        return fake_run(cmd, env)
    with tempfile.TemporaryDirectory() as td:
        root = Path(td).resolve()
        leg, app = root / "bench" / "apple", root / "apps" / "rn"
        other = str(root / "bench" / "android" / "build.gradle")
        with tracked_tree_unchanged("noop", [leg, app], root,
                                    run=guard_run):
            pass
        assert guard_calls[-1][-2:] == [str(leg), str(app)], guard_calls
        # a concurrent leg dirtied its own tree: not this step's concern
        state["dirty"] = {other: " M bench/android/build.gradle\n"}
        with tracked_tree_unchanged("noop", [leg, app], root,
                                    run=guard_run):
            pass
        try:
            with tracked_tree_unchanged("pod install", [leg, app], root,
                                        run=guard_run):
                state["dirty"][str(app / "ios" / "Podfile.lock")] = \
                    " M apps/rn/ios/Podfile.lock\n"
        except RuntimeError as e:
            assert "pod install rewrote 1 tracked file" in str(e), e
            assert "Podfile.lock" in str(e)
        else:
            raise AssertionError("bootstrap rewrite accepted")
        ran = []
        try:
            with tracked_tree_unchanged("pod install", [leg, app], root,
                                        run=guard_run):
                ran.append(True)
        except RuntimeError as e:
            assert "uncommitted tracked change" in str(e)
        else:
            raise AssertionError("dirty path accepted before bootstrap")
        assert not ran, "bootstrap ran over a dirty path"
        # a WaterUI project guards the framework checkout its waterui_path
        # names: an uncommitted framework edit fails its build before it
        # runs, though it lies outside the declared leg and app paths
        state["dirty"] = {}
        wapp = root / "bench" / "apps" / "waterui"
        wapp.mkdir(parents=True)
        (wapp / "Water.toml").write_text('waterui_path = "../../.."\n')
        with tracked_tree_unchanged("water build", [leg, wapp], root,
                                    run=guard_run):
            pass
        assert guard_calls[-1][-3:] == [str(leg), str(wapp), str(root)], \
            guard_calls[-1]
        state["dirty"] = {str(root / "core" / "src" / "lib.rs"):
                          " M core/src/lib.rs\n"}
        try:
            with tracked_tree_unchanged("water build", [leg, wapp], root,
                                        run=guard_run):
                raise AssertionError("built over a dirty framework")
        except RuntimeError as e:
            assert "core/src/lib.rs" in str(e), e
        state["dirty"] = {}
        for bad, why in (('x = 1\n', "declares no waterui_path"),
                         ('waterui_path = "../../../.."\n',
                          "outside the checkout")):
            (wapp / "Water.toml").write_text(bad)
            try:
                with tracked_tree_unchanged("water build", [wapp], root,
                                            run=guard_run):
                    pass
            except RuntimeError as e:
                assert why in str(e), (why, e)
            else:
                raise AssertionError(f"accepted Water.toml {bad!r}")

    # react-native-macos template: the installed version must match the
    # pin; committed files win, absent ones are generated and stamped,
    # a current stamp skips the generator
    gen_calls = []

    def gen_run(cmd, env=None):
        gen_calls.append(cmd)
        out = Path(cmd[4]) / "macos" / f"{cmd[5]}-macOS"
        out.mkdir(parents=True)
        (out / "main.m").write_text("generated")
        (out / "AppDelegate.mm").write_text("generated")
        return Out()
    with tempfile.TemporaryDirectory() as td:
        app = Path(td)
        (app / "package.json").write_text('{"name": "RnBench"}')
        pkg = app / "node_modules" / "react-native-macos"
        pkg.mkdir(parents=True)
        (pkg / "package.json").write_text('{"version": "0.81.9"}')
        authored = app / "macos" / "RnBench-macOS" / "AppDelegate.mm"
        authored.parent.mkdir(parents=True)
        authored.write_text("authored")
        try:
            ensure_rn_macos_template(app, "0.81.8", run=gen_run)
        except RuntimeError as e:
            assert "manifest pin disagree" in str(e)
        else:
            raise AssertionError("react-native-macos version drift accepted")
        assert not gen_calls
        ensure_rn_macos_template(app, "0.81.9", run=gen_run)
        assert authored.read_text() == "authored"
        assert (app / "macos" / "RnBench-macOS" / "main.m").read_text() \
            == "generated"
        assert (app / "macos" / ".bench-generator").read_text().splitlines() \
            == ["rn-macos-template 0.81.9 RnBench", "RnBench-macOS/main.m"]
        ensure_rn_macos_template(app, "0.81.9", run=gen_run)
        assert len(gen_calls) == 1, "current stamp regenerated"

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
