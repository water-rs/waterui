#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = ["perfetto>=0.58"]
# ///
"""Competitive benchmark runner — Android (water-rs/waterui#1262).

Single entry point, run with `uv run bench.py <command>`.

Commands:
  build      Build every contestant from manifest.toml into dist/
  collect    Static size metrics for the built artifacts (APK/AAB sizes,
             bundletool get-size total when a JDK + bundletool.jar exist)
  measure    Install/launch/drive/measure on an attached target
             (--serial <adb serial>; --locks-dir /tmp/device-locks enables
             physical-device mode: per-contestant flock, thermal cooldown,
             interleaved reps)
  report     Render a results JSON to markdown tables
  all        build + collect + measure + report (the one-shot VM path)

Device mode (Mac host, no toolchains):
  uv run bench.py measure --serial 4C081FDAP000U0 --locks-dir /tmp/device-locks
      --artifacts dist/
"""

from __future__ import annotations

import sys

if sys.version_info < (3, 10):
    raise SystemExit(
        "benchmarks/competitive requires Python >= 3.10 "
        f"(this interpreter is {sys.version.split()[0]}); every leg "
        "declares its version in pyproject.toml + .python-version and "
        "runs under the uv-managed interpreter (`uv run`)")

import argparse
import fcntl
import hashlib
import json
import math
import os
import platform
import re
import shlex
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time
import tomllib
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "lib"))
import toolchain  # benchmarks/competitive/lib/toolchain.py
import frame_stats as lib_frames

# backends/hydrolysis/bench/android metrics reuse assessment (reported
# to the coordinator): metrics/size.py crashes on any non-empty archive
# (it reads ZipInfo.name, which does not exist — the attribute is
# .filename), so its APK breakdown cannot run; metrics/meminfo.py's
# parser covers only the newer one-line `TOTAL PSS: x TOTAL RSS:`
# dumpsys format, and metrics/frames.py collectors are bound to the
# fixture-round serial model. The runner keeps its own size and
# meminfo implementations; the breakdown below mirrors size.py's
# categories in working code.

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "manifest.toml"
DIST = ROOT / "dist"
RESULTS = ROOT / "results.json"
# Runner-owned caches only — nothing is read from ad-hoc tool dirs.
# (the water CLI lives in the suite-shared benchmarks/competitive/.cache)
GRADLE_CACHE = ROOT / ".cache" / "gradle"
BUNDLETOOL_JAR = ROOT / ".cache" / "bundletool.jar"
# Every pulled trace of a run, kept so a failed capture can be analysed
# after the fact; one file per capture, never reused.
TRACE_DIR = ROOT / "traces"

# SHA256s of the official gradle-<v>-bin.zip published on
# services.gradle.org — the wrapper scripts/jar are materialised from
# these, never committed.
GRADLE_SUMS = {
    "9.6.1": "9c0f7faeeb306cb14e4279a3e084ca6b596894089a0638e68a07c945a32c9e14",
    "9.4.1": "2ab2958f2a1e51120c326cad6f385153bb11ee93b3c216c5fccebfdfbb7ec6cb",
    "8.14.1": "845952a9d6afa783db70bb3b0effaae45ae5542ca2bb7929619e8af49cb634cf",
    "8.14.3": "bd71102213493060956ec229d946beee57158dbd89d0e62b91bca0fa2c5f3531",
}
GRADLE_DIST = (
    "https://services.gradle.org/distributions/gradle-{v}-bin.zip")
BUNDLETOOL_URL = (
    "https://github.com/google/bundletool/releases/download/"
    "{v}/bundletool-all-{v}.jar")

# ---------------------------------------------------------------------------
# shell helpers
# ---------------------------------------------------------------------------


def sh(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def checked(cmd: list[str], **kw) -> str:
    r = sh(cmd, **kw)
    if r.returncode != 0:
        raise RuntimeError(
            f"{cmd[0]} failed ({r.returncode}):\n{r.stdout}\n{r.stderr}"
        )
    return r.stdout


def _adb_bin() -> str:
    """adb from the manifest-declared SDK — the one adb every host uses,
    build VM and device host alike. A host without the declared SDK fails
    here naming the pin; nothing is picked from PATH."""
    sdk = toolchain.require_dir(
        "android.sdk_root", FULL_MAN["toolchain"]["android"]["sdk_root"])
    return str(toolchain.require_file(
        "adb", sdk / "platform-tools" / "adb"))


def adb(serial: str, *args: str, timeout: int = 60) -> str:
    return checked([_adb_bin(), "-s", serial, *args],
                   timeout=timeout).strip()


def _java_bin(man: dict) -> str:
    """The manifest-declared JDK (toolchain.android.java_home), verified
    to report the declared major version — the one JDK every host uses;
    nothing is picked from $JAVA_HOME or PATH."""
    jh = toolchain.require_dir(
        "android.java_home", man["toolchain"]["android"]["java_home"])
    java = toolchain.require_file("java", jh / "bin" / "java")
    toolchain.require_version(
        "JDK", [str(java), "-version"],
        man["toolchain"]["android"]["jdk_version"])
    return str(java)


def adb_shell(serial: str, *args: str, timeout: int = 60) -> str:
    return adb(serial, "shell", *args, timeout=timeout)


# ---------------------------------------------------------------------------
# device locking / health (physical-device mode)
# ---------------------------------------------------------------------------


class DeviceLock:
    """Exclusive flock on <locks_dir>/<serial>.lock (+ mac.lock on the host)."""

    def __init__(self, locks_dir: Path, serial: str):
        self.serial = serial
        self.handles: list = []
        if locks_dir:
            locks_dir.mkdir(parents=True, exist_ok=True)
            for name in (serial, "mac"):
                fh = open(locks_dir / f"{name}.lock", "w")
                fcntl.flock(fh, fcntl.LOCK_EX)
                self.handles.append(fh)

    def release(self):
        for fh in self.handles:
            fcntl.flock(fh, fcntl.LOCK_UN)
            fh.close()
        self.handles = []

    __enter__ = lambda s: s
    __exit__ = lambda s, *a: s.release()


def thermal_status(serial: str) -> str:
    out = adb_shell(serial, "dumpsys thermalservice")
    m = re.search(r"mStatus=(\d+)", out)
    names = {0: "nominal", 1: "light", 2: "moderate", 3: "severe",
             4: "critical", 5: "emergency", 6: "shutdown"}
    return names.get(int(m.group(1)), f"unknown({m.group(1)})") if m else "n/a"


def refresh_hz(serial: str) -> float | None:
    out = adb_shell(serial, "dumpsys SurfaceFlinger")
    m = re.search(r"refresh-rate\s*:?\s*([\d.]+)", out)
    if not m:
        m = re.search(r"([\d.]+)\s*Hz", adb_shell(serial, "dumpsys display"))
    return float(m.group(1)) if m else None


SOFTWARE_GPU_MARKERS = ("swiftshader", "llvmpipe", "lavapipe", "softpipe",
                        "basic render", "basic display", "warp")


def gpu_adapter(serial: str) -> str:
    """Adapter the device composes on, e.g. 'Adreno (TM) 830' or the GLES
    renderer SurfaceFlinger reports. Frame-time and memory numbers are
    evidence only when this is a hardware GPU."""
    out = adb_shell(serial, "dumpsys SurfaceFlinger")
    m = re.search(r"GLES:\s*(.+)", out)
    gles = m.group(1).strip() if m else "unreported"
    if adb_shell(serial, "getprop ro.kernel.qemu").strip() == "1":
        gles += " (emulator)"
    return gles


def is_software_gpu(adapter: str) -> bool:
    a = adapter.lower()
    return any(m in a for m in SOFTWARE_GPU_MARKERS)


def wait_nominal(serial: str, timeout_s: int = 900) -> str:
    """Cool down between contestants: block until thermal status is
    nominal. The thermal listener API is app-side only, so from the adb
    shell the status is read every 30 s; a device still above nominal at
    the bound — or one reporting no status at all — fails the run: no
    contestant is ever measured at a non-nominal state."""
    t0 = time.monotonic()
    while True:
        st = thermal_status(serial)
        if st == "nominal":
            return st
        if st == "n/a":
            raise RuntimeError(
                "dumpsys thermalservice reports no mStatus — a nominal "
                "thermal state cannot be established")
        if time.monotonic() - t0 >= timeout_s:
            raise RuntimeError(
                f"thermal status still {st} after {timeout_s} s of "
                "cooldown — refusing to measure at a non-nominal state")
        print(f"  thermal {st}, cooling…", flush=True)
        time.sleep(30)


# ---------------------------------------------------------------------------
# contestant build
# ---------------------------------------------------------------------------


def ensure_debug_keystore(keystore: Path) -> None:
    """Materialise a debug keystore with the standard Android credentials.

    Nothing secret is committed: `keytool -genkeypair` produces a fresh
    throwaway key at build time — the same thing AGP does for
    ~/.android/debug.keystore when none is configured. Only the
    *credentials* (androiddebugkey / android) are standard so every
    contestant's artifacts stay installable side by side.
    """
    if keystore.exists():
        return
    keytool = Path(os.environ.get("JAVA_HOME", "")) / "bin" / "keytool"
    if not keytool.exists():
        keytool = Path(shutil.which("keytool") or "keytool")
    checked([
        str(keytool), "-genkeypair", "-keystore", str(keystore),
        "-storepass", "android", "-alias", "androiddebugkey",
        "-keypass", "android", "-keyalg", "RSA", "-keysize", "2048",
        "-validity", "10950", "-dname", "CN=Android Debug,O=Android,C=US",
    ])


def resolved_toolchain(man: dict) -> dict:
    """Resolve every tool the build invokes from manifest-declared pins.

    Nothing is picked from ad-hoc PATH entries: the water CLI is built by
    cargo install --locked --path cli from THIS checkout (cli/ is a
    workspace member; framework, CLI and backend are one commit — the
    checkout's HEAD sha is their identity), and each host tool's declared
    version is verified against what it reports.
    """
    t = man["toolchain"]
    res: dict = {"reported": {}}
    # the suite-shared cache builds one CLI per checkout+target — every
    # leg resolves the same binary for the same HEAD
    res["water"] = toolchain.provision_water_cli()
    res["reported"]["water"] = f"water (in-tree cli @ {toolchain.checkout_head()[:12]})"
    res["reported"]["waterui_head"] = toolchain.checkout_head()
    # the android backend's Kotlin runtime is pinned by the root manifest
    res["reported"]["android_backend_revision"] = \
        toolchain.android_backend_revision()
    res["sdk"] = toolchain.require_dir(
        "android.sdk_root", t["android"]["sdk_root"])
    toolchain.require_file(
        "adb", res["sdk"] / "platform-tools" / "adb")
    res["java_home"] = toolchain.require_dir(
        "android.java_home", t["android"]["java_home"])
    res["reported"]["java"] = toolchain.require_version(
        "JDK", [str(res["java_home"] / "bin" / "java"), "-version"],
        t["android"]["jdk_version"])
    res["flutter"] = toolchain.require_dir(
        "flutter.sdk_root", t["flutter"]["sdk_root"])
    res["reported"]["flutter"] = toolchain.require_version(
        "flutter", [str(res["flutter"] / "bin" / "flutter"),
                    "--version"],
        [f"Flutter {t['flutter']['version']}",
         f"Dart {t['flutter']['dart_version']}"])
    res["node"] = toolchain.require_dir(
        "reactnative.node_root", t["reactnative"]["node_root"])
    res["reported"]["node"] = toolchain.require_version(
        "node", [str(res["node"] / "bin" / "node"), "--version"],
        f"v{t['reactnative']['node_version']}")
    return res


def env(man: dict) -> dict:
    t = resolved_toolchain(man)
    (DIST).mkdir(parents=True, exist_ok=True)
    (DIST / "toolchain-resolved.json").write_text(
        json.dumps({"water": str(t["water"]),
                    "sdk_root": str(t["sdk"]),
                    "java_home": str(t["java_home"]),
                    "flutter": str(t["flutter"]),
                    "node": str(t["node"]),
                    "reported": t["reported"]}, indent=2))
    e = dict(os.environ)
    e["ANDROID_SDK_ROOT"] = str(t["sdk"])
    # AGP refuses a split identity (minifyReleaseWithR8): ANDROID_HOME and
    # ANDROID_SDK_ROOT must name the SAME directory — pin both.
    e["ANDROID_HOME"] = str(t["sdk"])
    e["JAVA_HOME"] = str(t["java_home"])
    e["PATH"] = ":".join(
        [
            str(t["water"].parent),
            str(t["flutter"] / "bin"),
            str(t["node"] / "bin"),
            str(t["sdk"] / "platform-tools"),
            str(t["java_home"] / "bin"),
            e["PATH"],
        ]
    )
    ght = shutil.which("gh") and sh(["gh", "auth", "token"]).stdout.strip()
    if ght:
        e["GITHUB_TOKEN"] = ght
    return e


# full manifest, set in main() so per-contestant builders can read the
# shared [toolchain] pins
FULL_MAN: dict = {}


def gradle_version_for(c: dict) -> str:
    return (c.get("gradle_version")
            or FULL_MAN["toolchain"]["native"]["gradle_version"])


def ensure_gradle_wrapper(proj: Path, version: str, e: dict) -> None:
    """Materialise gradlew + gradle-wrapper.jar from the pinned,
    SHA256-verified Gradle distribution — nothing binary is committed.

    The distributionUrl must match the manifest pin; the wrapper jar and
    scripts are regenerated by `gradle wrapper` from the verified dist.
    """
    props = proj / "gradle" / "wrapper" / "gradle-wrapper.properties"
    kv = dict(
        line.split("=", 1)
        for line in props.read_text().splitlines()
        if "=" in line and not line.lstrip().startswith("#"))
    got_url = kv.get("distributionUrl")
    want_url = ("https\\://services.gradle.org/distributions/"
                f"gradle-{version}-bin.zip")
    if got_url != want_url:
        raise RuntimeError(
            f"{proj}: wrapper distributionUrl is "
            f"{got_url or 'absent'}, manifest pins gradle "
            f"{version} ({want_url})")
    want_sha = GRADLE_SUMS[version]
    if kv.get("distributionSha256Sum") != want_sha:
        # the checksum the regenerated wrapper pins, exactly as
        # `gradle wrapper --gradle-distribution-sha256-sum` writes it
        lines = props.read_text().splitlines()
        lines = [l for l in lines
                 if not l.startswith("distributionSha256Sum=")]
        for i, l in enumerate(lines):
            if l.startswith("distributionUrl="):
                lines.insert(i + 1, f"distributionSha256Sum={want_sha}")
                break
        props.write_text("\n".join(lines) + "\n")
    if (proj / "gradlew").exists() and \
            (proj / "gradle" / "wrapper" / "gradle-wrapper.jar").exists():
        return
    dist_zip = toolchain.fetch_verified(
        GRADLE_DIST.format(v=version), want_sha,
        GRADLE_CACHE / f"gradle-{version}-bin.zip")
    dist = toolchain.unzip_verified(
        dist_zip, GRADLE_CACHE, f"gradle-{version}")
    gradle = dist / "bin" / "gradle"
    gradle.chmod(0o755)
    checked([str(gradle), "wrapper", "--gradle-version", version,
             "--gradle-distribution-sha256-sum", want_sha,
             "--console=plain"],
            cwd=proj, env=e)
    for f in (proj / "gradlew", proj / "gradlew.bat",
              proj / "gradle" / "wrapper" / "gradle-wrapper.jar"):
        if not f.exists():
            raise RuntimeError(
                f"gradle wrapper did not materialise {f}")


def build_waterui(man, dist_dir: Path, e: dict):
    d = (ROOT / man["dir"]).resolve()
    backend = man.get("backend", "android")
    # `water package` builds ONE ABI per --arch: the APKs are produced by
    # a separate `water package --arch <abi>` invocation per ABI and
    # recorded separately (app-release.apk is the CLI's output name; the
    # manifest's per-ABI names are the dist names). `--distribution`
    # yields the universal AAB. The managed backend is scaffolded into the
    # CLI's build cache (outside the checkout); packaged artifacts land in
    # the project's target/package/ (platforming::place_in_project).
    # Release signing comes from [signing.android] in Water.toml — the
    # debug keystore is generated at build time, never committed.
    ensure_debug_keystore(d / "debug.keystore")
    be = dict(e, WATERUI_ANDROID_STORE_PASSWORD="android",
              WATERUI_ANDROID_KEY_PASSWORD="android")
    base = ["water", "package", "--platform", "android", "--backend",
            backend, "--release", "-y"]
    outs = d / "target" / "package"
    # the manifest's logical abi keys are NOT the CLI's --arch spelling:
    # `water package --arch` accepts arm64|x86-64|armv7|x86 only
    cli_arch = {"arm64": "arm64", "x86_64": "x86-64"}
    for abi, name in man["abi_apks"].items():
        checked([*base, "--arch", cli_arch[abi]], cwd=d, env=be)
        shutil.copy(outs / "app-release.apk", dist_dir / name)
    checked([*base, "--arch", "arm64,x86-64", "--distribution"],
            cwd=d, env=be)
    shutil.copy(outs / "app-release.aab", dist_dir / man["aab"])


def rewrite_wrapper_pin(proj: Path, version: str) -> None:
    """Point a GENERATED gradle project's wrapper at the manifest pin —
    distributionUrl + distributionSha256Sum — so the verified distribution
    materialises the wrapper jar (nothing binary committed or trusted).
    Applies to generated projects only; committed contestants are checked
    strictly by ensure_gradle_wrapper."""
    props = proj / "gradle" / "wrapper" / "gradle-wrapper.properties"
    text = props.read_text()
    text = re.sub(
        r"distributionUrl=\S+",
        "distributionUrl="
        f"https\\\\://services.gradle.org/distributions/gradle-{version}-bin.zip",
        text)
    text = re.sub(r"distributionSha256Sum=\S+",
                  f"distributionSha256Sum={GRADLE_SUMS[version]}", text)
    if "distributionSha256Sum=" not in text:
        text = text.rstrip("\n") + \
            f"\ndistributionSha256Sum={GRADLE_SUMS[version]}\n"
    props.write_text(text)


def ensure_flutter_android(d: Path, e: dict) -> None:
    """The flutter app's android/ directory is generated, not committed:
    produced by the pinned Flutter SDK's `flutter create` in a scratch dir,
    then the authored override replaces the template MainActivity.
    A `.bench-generator` stamp records the generator pin; a stale
    android/ from a different flutter version is regenerated."""
    tag = ("flutter create --platforms android --project-name bench_flutter "
           "--org dev.bench --template app | flutter="
           + FULL_MAN["toolchain"]["flutter"]["version"])
    stamp = d / "android" / ".bench-generator"
    if (d / "android").is_dir() and stamp.exists()                 and stamp.read_text().strip() == tag:
        return
    shutil.rmtree(d / "android", ignore_errors=True)
    with tempfile.TemporaryDirectory() as td:
        checked(["flutter", "create", "--platforms", "android",
                 "--project-name", "bench_flutter", "--org", "dev.bench",
                 "--template", "app", str(Path(td) / "app")],
                cwd=td, env=e)
        shutil.copytree(Path(td) / "app" / "android", d / "android")
    shutil.copy(d / "android-override" / "MainActivity.kt",
                d / "android" / "app" / "src" / "main" / "kotlin" /
                "dev" / "bench" / "bench_flutter" / "MainActivity.kt")
    stamp.write_text(tag + "\n")


def ensure_rn_android(d: Path, e: dict) -> None:
    """The RN app's android/ directory is generated, not committed:
    produced by the manifest-pinned @react-native-community/cli `init` in a
    scratch dir, then the authored override replaces the template
    MainActivity and `npm ci` restores the lockfile-pinned node_modules.
    Root template files (index.js, app.json, metro/babel config…) come
    from toolchain.ensure_rn_template — same generator, digest-pinned."""
    t = FULL_MAN["toolchain"]["reactnative"]
    toolchain.ensure_rn_template(d, t["cli_version"], t["version"],
                                 t["template_sha256"], env=e)
    tag = (f"@react-native-community/cli@{t['cli_version']} init RnBench "
           f"--version {t['version']} --skip-install | android/ subtree")
    stamp = d / "android" / ".bench-generator"
    if not ((d / "android").is_dir() and stamp.exists()
            and stamp.read_text().strip() == tag):
        shutil.rmtree(d / "android", ignore_errors=True)
        with tempfile.TemporaryDirectory() as td:
            checked(["npx", f"@react-native-community/cli@{t['cli_version']}",
                     "init", "RnBench", "--version", t["version"],
                     "--directory", str(Path(td) / "RnBench"),
                     "--skip-install"],
                    cwd=td, env=e)
            shutil.copytree(Path(td) / "RnBench" / "android", d / "android")
        shutil.copy(d / "android-override" / "MainActivity.kt",
                    d / "android" / "app" / "src" / "main" / "java" /
                    "com" / "rnbench" / "MainActivity.kt")
        stamp.write_text(tag + "\n")
        checked(["npm", "ci"], cwd=d, env=e)
    # harness-authored template change: the manifest's per-ABI APK matrix
    # needs splits on, which the stock template leaves off. Applied
    # idempotently so trees generated before this change pick it up.
    app_gradle = d / "android" / "app" / "build.gradle"
    text = app_gradle.read_text()
    if ("universalApk" in text
            or "enableSeparateBuildPerCPUArchitecture = true" in text):
        pass
    elif "enableSeparateBuildPerCPUArchitecture" in text:
        text = text.replace(
            "enableSeparateBuildPerCPUArchitecture = false",
            "enableSeparateBuildPerCPUArchitecture = true")
        app_gradle.write_text(text)
    else:
        app_gradle.write_text(text + (
            "\nandroid {\n"
            "    splits {\n"
            "        abi {\n"
            "            enable true\n"
            "            reset()\n"
            "            include \"arm64-v8a\", \"x86_64\"\n"
            "            universalApk false\n"
            "        }\n"
            "    }\n"
            "}\n"))


def build_flutter(man, dist_dir: Path, e: dict):
    d = ROOT / man["dir"]
    ensure_flutter_android(d, e)
    # flutter build invokes the generated project's gradlew internally —
    # point its wrapper at the manifest pin, then materialise it from the
    # SHA256-verified distribution
    rewrite_wrapper_pin(d / man["gradle_project"], gradle_version_for(man))
    ensure_gradle_wrapper(d / man["gradle_project"],
                          gradle_version_for(man), e)
    checked(["flutter", "build", "apk", "--release", "--split-per-abi"],
            cwd=d, env=e)
    checked(["flutter", "build", "appbundle", "--release"], cwd=d, env=e)
    outs = d / "build/app/outputs"
    for abi, name in man["abi_apks"].items():
        shutil.copy(outs / "flutter-apk" / name, dist_dir / name)
    shutil.copy(outs / "bundle/release/app-release.aab",
                dist_dir / man["aab"])


def build_gradle(man, dist_dir: Path, e: dict, rn: bool = False):
    d = ROOT / man["dir"]
    if rn:
        ensure_rn_android(d, e)
        rewrite_wrapper_pin(d / man["gradle_project"],
                            gradle_version_for(man))
    proj = d / man["gradle_project"]
    if rn:
        # RN's signing config reads app/debug.keystore (module dir);
        # compose/views read rootProject.file at the project root.
        ensure_debug_keystore(proj / "app" / "debug.keystore")
    else:
        ensure_debug_keystore(proj / "debug.keystore")
    ensure_gradle_wrapper(proj, gradle_version_for(man), e)
    checked(["./gradlew", ":app:assembleRelease", "--console=plain"],
            cwd=proj, env=e)
    checked(["./gradlew", ":app:bundleRelease", "-PabiSplits=false",
             "--console=plain"], cwd=proj, env=e)
    stage(man, proj / "app/build/outputs", dist_dir)


def stage(man, outputs: Path, dist_dir: Path):
    for abi, name in man["abi_apks"].items():
        src = outputs / "apk/release" / name
        if not src.exists():
            got = sorted(p.name for p in (outputs / "apk/release").glob("*")
                         if p.is_file())
            raise RuntimeError(
                f"expected APK {name} missing; apk/release contains {got}")
        shutil.copy(src, dist_dir / name)
    aab = outputs / "bundle/release" / man["aab"]
    if not aab.exists():
        got = sorted(p.name for p in (outputs / "bundle/release").glob("*")
                     if p.is_file())
        raise RuntimeError(
            f"expected AAB {man['aab']} missing; bundle/release contains {got}")
    shutil.copy(aab, dist_dir / man["aab"])


def cmd_build(man):
    e = env(man)
    builders = {"waterui": build_waterui, "flutter": build_flutter,
                "reactnative": build_gradle, "native": build_gradle}
    failed = []
    for name, c in man["contestants"].items():
        print(f"== build {name}", flush=True)
        ddir = DIST / name
        ddir.mkdir(parents=True, exist_ok=True)
        # bootstrap (generated android/ trees, npm ci, wrapper pins) and
        # the build run on a clean tree and must leave it clean — a step
        # that rewrites a tracked file aborts the build command
        with toolchain.tracked_tree_unchanged(
                f"android build {name}", [ROOT, (ROOT / c["dir"]).resolve()]):
            try:
                if c["kind"] == "reactnative":
                    build_gradle(c, ddir, e, rn=True)
                else:
                    builders[c["kind"]](c, ddir, e)
                (ddir / "BUILD_ERROR.txt").unlink(missing_ok=True)
                print(f"   ok -> {ddir}", flush=True)
            except RuntimeError as ex:
                (ddir / "BUILD_ERROR.txt").write_text(str(ex))
                print(f"   FAILED — recorded to {ddir}/BUILD_ERROR.txt",
                      flush=True)
                failed.append(name)
    if failed:
        # a build that ends with only failed contestants must fail the
        # command, not present as a successful all-failed run
        raise SystemExit(
            f"build failed for: {', '.join(failed)} "
            "(see dist/*/BUILD_ERROR.txt)")


# ---------------------------------------------------------------------------
# static size metrics (host-side; no device needed)
# ---------------------------------------------------------------------------


def apk_sizes(path: Path) -> dict:
    groups = {"dex": 0, "native_libs": 0, "resources": 0, "assets": 0,
              "kotlin_meta": 0, "other": 0}
    with zipfile.ZipFile(path) as z:
        for info in z.infolist():
            if info.filename.startswith("classes") and \
                    info.filename.endswith(".dex"):
                groups["dex"] += info.file_size
            elif info.filename.startswith("lib/"):
                groups["native_libs"] += info.file_size
            elif info.filename == "resources.arsc" or \
                    info.filename.startswith("res/"):
                groups["resources"] += info.file_size
            elif info.filename.startswith("assets/"):
                groups["assets"] += info.file_size
            elif info.filename.endswith(".kotlin_metadata"):
                groups["kotlin_meta"] += info.file_size
            else:
                groups["other"] += info.file_size
    return {"apk_bytes": path.stat().st_size,
            "apk_uncompressed_bytes": sum(groups.values()),
            "apk_uncompressed_breakdown_bytes": groups,
            "sha256": toolchain.sha256_file(path)}


def ensure_bundletool(man: dict) -> Path:
    bt = man["toolchain"]["bundletool"]
    return toolchain.fetch_verified(
        BUNDLETOOL_URL.format(v=bt["version"]), bt["sha256"],
        BUNDLETOOL_JAR)


def aab_download_size(aab: Path, serial: str | None, man: dict) -> dict:
    jar = ensure_bundletool(man)
    java = _java_bin(man)
    with tempfile.TemporaryDirectory() as td:
        apks = Path(td) / "out.apks"
        spec = Path(td) / "spec.json"
        if serial:
            # the attached device's own spec; a failure here is a broken
            # adb/bundletool setup, not a reason to size for another device
            checked([java, "-jar", str(jar), "get-device-spec",
                     "--adb", _adb_bin(), "--device-id", serial,
                     "--output", str(spec)])
        else:
            # VM mode has no device: size for the documented reference spec
            spec.write_text(json.dumps(_default_spec()))
        checked([java, "-jar", str(jar), "build-apks",
                 "--bundle", str(aab), "--output", str(apks),
                 "--device-spec", str(spec)])
        out = checked([java, "-jar", str(jar), "get-size",
                       "total", "--apks", str(apks)])
        n = re.findall(r"(\d+)", out)
        if not n:
            raise RuntimeError(f"bundletool get-size printed no size: {out!r}")
        return {"aab_arm64_download_bytes": int(n[-1]),
                "aab_sha256": toolchain.sha256_file(aab),
                "device_spec": "attached device" if serial else "reference"}


def _default_spec() -> dict:
    return {"supportedAbis": ["arm64-v8a", "armeabi-v7a"], "supportedLocales": ["en-US"],
            "deviceFeatures": [], "glExtensions": [], "sdkVersion": 36,
            "screenDensity": 420}


def cmd_collect(man, results: dict, artifacts: Path = DIST,
                serial: str | None = None):
    for name, c in man["contestants"].items():
        ddir = artifacts / name
        entry = results.setdefault("results", {}).setdefault(name, {})
        if (ddir / "BUILD_ERROR.txt").exists():
            entry["build_error"] = (ddir / "BUILD_ERROR.txt").read_text()
            continue
        sizes = {}
        for abi, apk in c["abi_apks"].items():
            p = ddir / apk
            if p.exists():
                sizes[f"apk_{abi}"] = apk_sizes(p)
        aab = ddir / c["aab"]
        if aab.exists():
            sizes["aab"] = aab_download_size(aab, serial, man)
        entry["package_size"] = sizes
        print(f"== size {name} done", flush=True)


# ---------------------------------------------------------------------------
# on-device measurement
# ---------------------------------------------------------------------------


def apk_for_abi(man_entry, device_abi: str) -> str:
    key = "arm64" if device_abi.startswith("arm64") else "x86_64"
    return man_entry["abi_apks"][key]


def install(serial: str, apk: Path):
    adb(serial, "install", "-r", "--no-streaming", str(apk), timeout=300)


def verify_installed(serial: str, pkg: str, apk: Path) -> str:
    """Install `apk` for `pkg` and verify the installed base APK's sha256
    equals the artifact the row records. Two contestants may share one
    application id (waterui android vs hydrolysis backends): identity is
    the installed file's hash, checked per rep — never install order or
    a static label. Mismatch fails the attempt."""
    install(serial, apk)
    out = adb_shell(serial, f"pm path {pkg}")
    paths = [l.split(":", 1)[1].strip() for l in out.splitlines()
             if l.startswith("package:")]
    if not paths:
        raise RuntimeError(f"{pkg}: pm path reports no installed package")
    base = next((p for p in paths if p.endswith("base.apk")), paths[0])
    got = adb_shell(serial, f"sha256sum {base}").split()[0]
    want = toolchain.sha256_file(apk)
    if got != want:
        raise RuntimeError(
            f"{pkg}: installed APK sha256 {got} does not match the "
            f"measured artifact {apk.name} ({want}) — the package on "
            "device belongs to a different build")
    return got


def am_start_cmd(pkg: str, activity: str, workload: str, kind: str,
                 step: int | None = None) -> str:
    """`am start -W` for one cold launch of the contestant.

    `step` carries the capacity-workload level (W5 rect count / W6 row
    depth) — `--es step N` for every contestant and `waterui.env.BENCH_STEP`
    for WaterUI, whose scaffold forwards `waterui.env.*` extras into the
    process environment. `-W` blocks until the system reports the launch's
    first frame drawn (the Displayed event) and prints its TotalTime."""
    if kind == "waterui":
        extra = ["--es", "waterui.env.BENCH_WORKLOAD", workload]
        if step is not None:
            extra += ["--es", "waterui.env.BENCH_STEP", str(step)]
    else:
        extra = ["--es", "workload", workload]
        if step is not None:
            extra += ["--ei", "step", str(step)]
    return shlex.join(["am", "start", "-W", "-n", f"{pkg}/{activity}",
                       *extra])


def launch_total_ms(am_output: str) -> float:
    """TotalTime of an `am start -W` launch — launch to the first frame
    drawn, from the same ActivityMetricsLogger event as logcat's
    `Displayed` line. A launch that reports none never completed."""
    m = re.search(r"TotalTime:\s*(\d+)", am_output)
    if not m:
        raise RuntimeError(
            f"am start -W reported no completed launch:\n{am_output}")
    return float(m.group(1))


def launch(serial: str, pkg: str, activity: str, workload: str,
           kind: str, step: int | None = None) -> float:
    """Cold launch outside any trace (W1); returns launch → first frame
    drawn (ms)."""
    adb_shell(serial, f"am force-stop {pkg}")
    return launch_total_ms(adb_shell(
        serial, am_start_cmd(pkg, activity, workload, kind, step),
        timeout=60))


# Memory samples inside a capture window: the kernel's smaps_rollup Pss,
# every MEM_SAMPLE_S, read by the on-device program itself.
MEM_SAMPLE_S = 0.5

# The device program's window-start marker: an atrace slice the program
# writes through the kernel's trace_marker the instant it opens the window
# and starts the drive, closed when the window's hold ends. The traces
# record it through ftrace `print`, so the drive's start is a timestamp in
# the same trace — and the same clock — as the owned presents that define
# the window (WORKLOADS.md METHOD).
TRACE_MARKER = "/sys/kernel/tracing/trace_marker"
WINDOW_MARKER = "bench.window"


def device_program(am_start: str, pkg: str, warmup_ms: int, cap_ms: int,
                   drive: str | None, sample_mem: bool) -> str:
    """The whole capture as ONE on-device shell program, so nothing the
    host does lands between the launch and the drive (../WORKLOADS.md
    METHOD).

    `am start -W` returns on the launch's first-frame event; the declared
    warmup follows and the window opens. At window start the program
    records the steady memory, writes the `bench.window` begin marker into
    the trace, and starts the hold, the drive program and the in-window
    memory sampler in the background; the hold is exactly the capture
    length — the drive never shortens the window, and a drive that has not
    finished when the window closes fails the capture. The end marker is
    written when the hold ends. The trace analysis bounds the marker
    against the window it derives from the first owned present."""
    marker = shlex.quote(TRACE_MARKER)
    lines = [
        am_start,
        f"pid=$(pidof {shlex.quote(pkg)})",
        '[ -n "$pid" ] || { echo "BENCH_ERR no process after launch"; '
        'exit 11; }',
        'echo "BENCH_PID $pid"',
        f"sleep {warmup_ms / 1000.0}",
    ]
    if sample_mem:
        lines.append(
            'echo "BENCH_MEM_STEADY $(grep -E \'^(Pss|Rss):\' '
            '/proc/$pid/smaps_rollup | tr \'\\n\' \' \')"')
    # window start: the marker, then the hold and the drive at once
    lines += [
        f'echo "B|$$|{WINDOW_MARKER}" > {marker} || '
        f'{{ echo "BENCH_ERR cannot write the window marker to '
        f'{TRACE_MARKER}"; exit 13; }}',
        f"sleep {cap_ms / 1000.0} & hold=$!",
    ]
    if drive is not None:
        lines.append(f"( {drive} ) & drv=$!")
    if sample_mem:
        lines.append(
            '( while kill -0 $hold 2>/dev/null; do '
            'echo "BENCH_MEM_SAMPLE $(grep \'^Pss:\' '
            '/proc/$pid/smaps_rollup)"; '
            f"sleep {MEM_SAMPLE_S}; done ) & mem=$!")
    lines.append("wait $hold")
    # window end
    lines.append(
        f'echo "E|$$" > {marker} || '
        f'{{ echo "BENCH_ERR cannot write the window end marker to '
        f'{TRACE_MARKER}"; exit 14; }}')
    if drive is not None:
        lines.append(
            'if kill -0 $drv 2>/dev/null; then kill $drv; '
            'echo "BENCH_ERR drive program outlasted the capture window"; '
            "exit 12; fi")
    if sample_mem:
        lines.append("wait $mem")
    return "\n".join(lines) + "\n"


def parse_program_output(out: str, sample_mem: bool) -> dict:
    """What the on-device program reported: the launch's TotalTime, the
    launched pid, and (when sampled) the window's memory."""
    err = re.search(r"^BENCH_ERR (.+)$", out, re.M)
    if err:
        raise RuntimeError(f"capture program failed: {err.group(1)}\n{out}")
    res: dict = {"startup_ms": launch_total_ms(out)}
    m = re.search(r"^BENCH_PID (\d+)$", out, re.M)
    if not m:
        raise RuntimeError(f"capture program reported no pid:\n{out}")
    res["pid"] = int(m.group(1))
    if sample_mem:
        steady = re.search(r"^BENCH_MEM_STEADY (.*)$", out, re.M)
        vals = {}
        for key in ("Pss", "Rss"):
            k = re.search(rf"{key}:\s*(\d+) kB", steady.group(1)) \
                if steady else None
            if k is None:
                raise RuntimeError(
                    f"no {key} at window start in smaps_rollup:\n{out}")
            vals[key.lower() + "_kb"] = int(k.group(1))
        res["memory_steady"] = vals
        samples = [int(x) for x in re.findall(
            r"^BENCH_MEM_SAMPLE Pss:\s*(\d+) kB$", out, re.M)]
        if not samples:
            raise RuntimeError(
                f"no Pss sample inside the capture window:\n{out}")
        res["memory_peak_kb"] = max(samples)
    return res


def proc_mem(serial: str, pkg: str) -> dict:
    """Pss/Rss from /proc/<pid>/smaps_rollup — the sample source inside a
    capture window. `dumpsys meminfo` walks every VMA through a binder
    call heavy enough to perturb the workload being measured; the kernel
    rollup file is the same counters at negligible cost."""
    pid = adb_shell(serial, f"pidof {pkg}").strip()
    if not pid:
        return {"pss_kb": None, "rss_kb": None}
    out = adb_shell(serial, f"cat /proc/{pid.split()[0]}/smaps_rollup")
    vals = {}
    for key in ("Pss", "Rss"):
        m = re.search(rf"^{key}:\s*(\d+) kB", out, re.M)
        vals[key.lower() + "_kb"] = int(m.group(1)) if m else None
    return vals


# Every capture records FrameTimeline (the frame source), the process scan
# and ftrace `print` — the device program's window marker (WINDOW_MARKER),
# which places the drive's start in the trace beside the owned presents.
PERFETTO_CFG = """\
buffers { size_kb: 32768 fill_policy: RING_BUFFER }
data_sources { config { name: "android.surfaceflinger.frametimeline" } }
data_sources { config { name: "linux.process_stats" process_stats_config {
  scan_all_processes_on_start: true
  record_thread_names: true
} } }
data_sources { config { name: "linux.ftrace" ftrace_config {
  ftrace_events: "ftrace/print"
} } }
duration_ms: %d
"""

# Capacity-workload config: the same sources plus scheduler slices so CPU
# ms/frame on the app's UI + render threads can be attributed (W5/W6 only).
PERFETTO_CAP_CFG = """\
buffers { size_kb: 65536 fill_policy: RING_BUFFER }
data_sources { config { name: "android.surfaceflinger.frametimeline" } }
data_sources { config { name: "linux.process_stats" process_stats_config {
  scan_all_processes_on_start: true
  record_thread_names: true
} } }
data_sources { config { name: "linux.ftrace" ftrace_config {
  ftrace_events: "ftrace/print"
  ftrace_events: "sched/sched_switch"
  ftrace_events: "sched/sched_wakeup"
  ftrace_events: "task/task_newtask"
  ftrace_events: "task/task_rename"
} } }
duration_ms: %d
"""

# Frame source, one for every contestant: Perfetto FrameTimeline presents
# attributed to the contestant's owned process — actual_frame_timeline_
# slice (Choreographer-attributed) and surface_frame_timeline_slice
# (per-layer, which also sees SurfaceView/BLAST pipelines like Flutter's)
# together cover every pipeline the contestants use.


def _record_trace(serial: str, cfg: str, program: str,
                  program_timeout_s: float) -> tuple[Path, str]:
    """Record a trace that spans the whole on-device capture `program`.

    The previous instance of the contestant is stopped and the log cleared
    BEFORE the session starts, so the trace holds only the launch it
    measures. `--background-wait` returns once every data source has
    started and prints the tracing pid; the program then runs (launch,
    warmup, the fixed capture window); when it exits, SIGTERM ends the
    session and perfetto reads the buffers back into the file, and the
    capture waits for that process to be gone and checks the file against
    the byte count perfetto logged. The config's duration is only a safety
    cap. Returns the pulled trace and the program's output."""
    remote = f"/data/misc/perfetto-traces/bench_{int(time.time()*1000)}.perfetto-trace"
    proc = subprocess.run(
        [_adb_bin(), "-s", serial, "shell",
         f"perfetto --background-wait -c - --txt -o {remote}"],
        input=cfg, text=True, capture_output=True, timeout=40)
    pid = proc.stdout.strip().splitlines()[-1].strip() if proc.stdout.strip() else ""
    if proc.returncode != 0 or not pid.isdigit():
        raise RuntimeError(
            f"perfetto did not start: rc={proc.returncode} "
            f"out={proc.stdout.strip()!r} err={proc.stderr.strip()[-300:]!r}")
    try:
        out = adb_shell(serial, program, timeout=int(program_timeout_s))
    finally:
        # `kill -0` cannot be the exit test: SELinux denies the shell domain
        # `signull` on perfetto's, so it fails at once and the loop never
        # waited. /proc/<pid> exists until init reaps the daemonized client,
        # which happens only after it has read back and closed the trace.
        adb_shell(serial, f"kill -TERM {pid}; "
                          f"while [ -d /proc/{pid} ]; do sleep 0.05; done",
                  timeout=60)
    remote_size = int(adb_shell(serial, "stat", "-c", "%s", remote).strip())
    # perfetto logs the byte count it finalized the file with; the device
    # file must hold exactly that many, or the client had not finished
    wrote = re.findall(
        rf"Wrote (\d+) bytes into {re.escape(remote)}",
        adb(serial, "logcat", "-d", "-s", "perfetto:*"))
    if wrote != [str(remote_size)]:
        raise RuntimeError(
            f"perfetto finalized {remote} as {wrote} bytes, the device "
            f"file has {remote_size}")
    # one file per capture: a shared path lets any other writer (another
    # capture, another harness copy) interleave with this one
    TRACE_DIR.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix=f"bench_{serial}_",
                                suffix=".perfetto-trace", dir=TRACE_DIR)
    os.close(fd)
    local = Path(name)
    adb(serial, "pull", remote, str(local), timeout=120)
    if local.stat().st_size != remote_size:
        raise RuntimeError(
            f"pulled trace {local} has {local.stat().st_size} bytes, the "
            f"device file {remote} has {remote_size}")
    adb_shell(serial, f"rm -f {remote}")
    assert_trace_populated(serial, local, proc)
    return local, out


# bound on the launch itself inside a capture program: `am start -W`
# returns on the first frame or the program fails
LAUNCH_BOUND_S = 60


def run_capture(serial: str, cfg: str, c: dict, workload: str,
                step: int | None, warmup_ms: int, cap_ms: int,
                drive: str | None, sample_mem: bool) -> tuple[Path, dict]:
    """One traced capture: stop the old instance, clear the log, record
    while the on-device program launches, warms up and holds the fixed
    window. Returns the trace and the program's report."""
    pkg = c["package"]
    adb_shell(serial, f"am force-stop {pkg}")
    adb_shell(serial, "logcat -c")
    program = device_program(
        am_start_cmd(pkg, c["activity"], workload, c["kind"], step),
        pkg, warmup_ms, cap_ms, drive, sample_mem)
    total_s = (warmup_ms + cap_ms) / 1000.0
    local, out = _record_trace(
        serial, cfg % (int(total_s * 1000) + 120000), program,
        LAUNCH_BOUND_S + total_s + 30)
    return local, parse_program_output(out, sample_mem)


class AppNotRespondingError(RuntimeError):
    """The system declared the contestant not responding: its main thread
    stayed blocked past the input-dispatch timeout and the ANR dialog took
    focus. On a capacity ladder that is the app's limit, a result."""


class EmptyTraceError(RuntimeError):
    """The tracing session recorded nothing — a harness/device failure,
    never a measurement: it must not be read as a zero-frame window."""


def assert_trace_populated(serial: str, path: Path,
                           proc: subprocess.CompletedProcess) -> None:
    """A capture window that ran its drive always yields process data and a
    non-zero trace span; an empty trace means the tracing session produced
    nothing, so the run aborts with perfetto's own output and traced's log."""
    from perfetto.trace_processor import TraceProcessor
    with TraceProcessor(trace=str(path)) as tp:
        b = next(iter(tp.query("SELECT start_ts, end_ts FROM trace_bounds")))
        procs = next(iter(tp.query("SELECT count(*) AS c FROM process"))).c
    if b.end_ts > b.start_ts and procs > 1:
        return
    log = adb_shell(serial, "logcat", "-d", "-t", "200", "-s",
                    "perfetto:*", "traced:*", "traced_probes:*")
    raise EmptyTraceError(
        f"perfetto trace is empty (bounds {b.start_ts}..{b.end_ts}, "
        f"{procs} processes, {path.stat().st_size} bytes); perfetto "
        f"stdout={proc.stdout.strip()!r} stderr={proc.stderr.strip()!r}; "
        f"traced log:\n{log[-4000:]}")


def _pctl(xs, p):
    xs = sorted(xs)
    if not xs:
        return None
    k = (len(xs) - 1) * p
    f, c = math.floor(k), math.ceil(k)
    return xs[int(k)] if f == c else xs[f] + (xs[c] - xs[f]) * (k - f)


def _r(v, nd=3):
    return round(v, nd) if v is not None else None


class NoFramesError(RuntimeError):
    """A capture that ran cleanly but recorded zero frames for the app.

    Distinct from a capture error: on a live app that still owns layers a
    zero-frame window is a real measurement (the collapse point); on a
    dead app or missing surfaces it stays an abort."""


def sf_layers(serial: str, pkg: str) -> list[str]:
    out = adb_shell(serial, "dumpsys SurfaceFlinger --list")
    names = []
    for m in re.finditer(r"RequestedLayerState\{(.+?)\}", out):
        # the entry is "<layer name> <attr>=<val> [<attr>=<val>...]" — strip
        # every trailing key=value token so the name survives whatever
        # attribute set and order this Android version prints
        name = m.group(1)
        while True:
            stripped = re.sub(r"\s+\S+=\S+$", "", name)
            if stripped == name:
                break
            name = stripped
        name = name.strip()
        if pkg in name and name not in names:
            names.append(name)
    return names


def process_alive(serial: str, pkg: str) -> bool:
    try:
        return bool(adb_shell(serial, f"pidof {pkg}").strip())
    except Exception:
        return False


def last_exit_info(serial: str, pkg: str) -> dict:
    """The system's record of how the contestant's newest process ended:
    ApplicationExitInfo #0 from `dumpsys activity exit-info`. It names the
    reason (crash, native crash, low memory, signaled, excessive resource
    use, ...) even when nothing reached the crash buffer, and it must be read
    at death time: the package keeps only its last 16 exits, so a later
    force-stop pushes it out."""
    out = adb_shell(serial, "dumpsys", "activity", "exit-info", pkg,
                    timeout=20)
    entry = out.split("ApplicationExitInfo #0:", 1)
    if len(entry) != 2:
        raise RuntimeError(f"{pkg}: no ApplicationExitInfo recorded:\n{out}")
    first = entry[1].split("ApplicationExitInfo #1:", 1)[0]
    info = {}
    for key, pat in (("timestamp", r"timestamp=(\S+ \S+)"),
                     ("pid", r"\bpid=(\d+)"),
                     ("reason", r"reason=(\d+ \([^)]*\))"),
                     ("subreason", r"subreason=(\d+ \([^)]*\))"),
                     ("status", r"status=(-?\d+)"),
                     ("pss_kb", r"pss=([\d.]+)"),
                     ("rss_kb", r"rss=([\d.]+)"),
                     ("description", r"description=(.*?) state=")):
        m = re.search(pat, first)
        info[key] = m.group(1) if m else None
    return info


def crash_reason(serial: str, pkg: str) -> dict:
    """Why the contestant's process died: the system's exit record plus the
    abort line from the crash log buffer when there is one.

    The run holds the device exclusively, so the newest crash-buffer entry
    is this contestant's; the 'Abort message' line carries the cause (e.g.
    the JNI global-ref overflow). A kill leaves no crash-buffer entry; the
    exit record's reason says who killed it."""
    out = adb_shell(serial, "logcat -b crash -d -t 400", timeout=20)
    abort = None
    for line in out.splitlines():
        if "Abort message:" in line:
            abort = line.split("Abort message:", 1)[-1].strip().strip("'\"")
        elif ("FATAL" in line or "Fatal signal" in line) and pkg in line:
            abort = line.split(":", 2)[-1].strip()
    return {"exit": last_exit_info(serial, pkg), "abort": abort}


def death_summary(reason: dict) -> str:
    """One line for the report: the system's exit reason, and the abort
    message when the crash buffer had one."""
    ex = reason["exit"]
    if ex is None:
        text = "exit record not captured"
    else:
        text = f"{ex['reason']}, {ex['subreason']}"
    if reason["abort"]:
        text += f"; abort: {reason['abort']}"
    return text


def zero_frame_result() -> dict:
    """A capture window where a live app presented nothing: a real
    measurement (collapse point), not a harness failure."""
    return {
        "frames": 0,
        "frame_ms_p50": None,
        "frame_ms_p90": None,
        "frame_ms_p99": None,
        "dropped_pct": None,
        "fps": 0.0,
        "ivals_ms": [],
    }


# Refresh rate pinned for the whole measure run by device_preflight(); any
# capture that ran at a different rate is a harness failure and aborts.
PINNED_REFRESH: float | None = None


def assert_awake(serial: str) -> None:
    out = adb_shell(serial, "dumpsys power")
    m = re.search(r"mWakefulness=(\w+)", out)
    if not m or m.group(1) != "Awake":
        raise RuntimeError(
            f"device not awake (mWakefulness={m.group(1) if m else 'unknown'}); "
            "aborting run — check stayon/keyguard preflight")


def active_refresh(serial: str) -> float | None:
    """Hz of the active vsync mode, from SurfaceFlinger or dumpsys display."""
    try:
        out = adb_shell(serial, "dumpsys SurfaceFlinger")
        m = re.search(r"vsyncRate[=:]\s*([\d.]+)", out)
        if not m:
            m = re.search(r"activeMode[^\n]*?(\d+(?:\.\d+)?)\s*Hz", out)
        if m:
            return float(m.group(1))
    except Exception:
        pass
    try:
        out = adb_shell(serial, "dumpsys display")
        m = re.search(r"fps=([\d.]+)", out)
        if m:
            return float(m.group(1))
    except Exception:
        pass
    return refresh_hz(serial)


def display_modes(serial: str) -> list[float]:
    """Refresh rates the panel supports, e.g. [60.0, 120.0]."""
    try:
        out = adb_shell(serial, "dumpsys display")
        m = re.search(r"supportedModes \[([^\]]*)\]", out)
        rates = {float(x) for x in
                 re.findall(r"fps=([\d.]+)", m.group(1) if m else out)}
        return sorted(round(r) for r in rates)
    except Exception:
        return []


def _preflight_state_path(serial: str) -> Path:
    """Where the device's pre-run settings are kept until they are restored.

    The file outlives a killed run, so the next run restores the true
    originals instead of recording the pinned values a dead run left behind."""
    return Path.home() / ".cache" / "bench-android" / f"{serial}.json"


_PREFLIGHT_KEYS = [("global", "stay_on_while_plugged_in"),
                   ("system", "peak_refresh_rate"),
                   ("system", "min_refresh_rate"),
                   ("system", "screen_off_timeout"),
                   ("system", "screen_brightness_mode"),
                   ("system", "screen_brightness")]


_LOCKSCREEN_KEY = "locksettings.disabled"
# `settings global zen_mode` values and the `cmd notification set_dnd`
# argument that restores each one
_DND_KEY = "global.zen_mode"
_ZEN_MODE_DND = {"0": "off", "1": "priority", "2": "none", "3": "alarms"}


def panel_sleep(serial: str) -> None:
    """Turn the panel off and leave the launcher behind, so no contestant's
    static frame stays lit on the OLED."""
    adb_shell(serial, "input", "keyevent", "KEYCODE_HOME")
    adb_shell(serial, "input", "keyevent", "KEYCODE_SLEEP")


def device_preflight(serial: str) -> dict[str, str]:
    """Wake the panel, dismiss the keyguard, keep it on at minimum
    brightness, and pin the refresh rate to the device's maximum mode.
    Returns prior settings to restore.

    Without this a device run can silently degrade: screen_off_timeout turns
    the panel off mid-run, refresh drops to 60 Hz and captures go empty.
    Brightness is pinned to the minimum for the run: frame pacing does not
    depend on it, and the panel spends hours lit."""
    state = _preflight_state_path(serial)
    if state.exists():
        prev = json.loads(state.read_text())
        print(f"   restoring originals left by an interrupted run: {state}",
              flush=True)
    else:
        prev = {}
        for ns, key in _PREFLIGHT_KEYS:
            prev[f"{ns}.{key}"] = (
                adb_shell(serial, "settings", "get", ns, key).strip()
                or "null")
        prev[_LOCKSCREEN_KEY] = adb_shell(
            serial, "locksettings", "get-disabled").strip()
        prev[_DND_KEY] = adb_shell(
            serial, "settings", "get", "global", "zen_mode").strip()
        state.parent.mkdir(parents=True, exist_ok=True)
        state.write_text(json.dumps(prev))
    # the keyguard must not come back mid-run: one dismissal at start is not
    # enough, a keyguard that reappears covers the contestant and its capture
    # records SystemUI's frames instead
    adb_shell(serial, "locksettings", "set-disabled", "true")
    # a heads-up notification lands at the top of the panel, and a drive
    # program's swipe that crosses it pulls the notification shade over the
    # contestant: total silence keeps every notification off the screen
    adb_shell(serial, "cmd", "notification", "set_dnd", "none")
    # silence alone did not keep the shade shut: it has opened fully during
    # a hold with no input from the harness at all (run r10g), so the panel
    # is locked against expansion for the whole run. SystemUI keeps this
    # flag in memory only; restore sends `none` to lift it.
    adb_shell(serial, "cmd", "statusbar", "send-disable-flag",
              "statusbar-expansion")
    adb_shell(serial, "svc", "power", "stayon", "true")
    adb_shell(serial, "input", "keyevent", "KEYCODE_WAKEUP")
    adb_shell(serial, "wm", "dismiss-keyguard")
    adb_shell(serial, "settings", "put", "system",
              "screen_off_timeout", "1800000")
    adb_shell(serial, "settings", "put", "system",
              "screen_brightness_mode", "0")
    adb_shell(serial, "settings", "put", "system", "screen_brightness", "1")
    global PINNED_REFRESH
    modes = display_modes(serial)
    target = round(max(modes)) if modes else round(refresh_hz(serial) or 60.0)
    adb_shell(serial, "settings", "put", "system", "peak_refresh_rate",
              str(target))
    adb_shell(serial, "settings", "put", "system", "min_refresh_rate",
              str(target))
    # the pin takes effect when the display re-configures: wait for the
    # active vsync to reach it rather than for a fixed interval
    t0 = time.monotonic()
    while True:
        cur = active_refresh(serial)
        if cur is not None and abs(cur - float(target)) <= 1.0:
            break
        if time.monotonic() - t0 > 10:
            raise RuntimeError(
                f"refresh pin to {target} Hz not applied (active {cur} Hz)")
        time.sleep(0.1)
    PINNED_REFRESH = float(target)
    return prev


def device_preflight_restore(serial: str, prev: dict[str, str]) -> None:
    """Restore the settings device_preflight() recorded, then turn the panel
    off. The state file is removed only once every setting is back."""
    global PINNED_REFRESH
    PINNED_REFRESH = None
    failed = False
    for dotted, val in prev.items():
        ns, key = dotted.split(".", 1)
        try:
            if dotted == _LOCKSCREEN_KEY:
                adb_shell(serial, "locksettings", "set-disabled", val)
            elif dotted == _DND_KEY:
                adb_shell(serial, "cmd", "notification", "set_dnd",
                          _ZEN_MODE_DND[val])
            elif val == "null":
                adb_shell(serial, "settings", "delete", ns, key)
            else:
                adb_shell(serial, "settings", "put", ns, key, val)
        except Exception as ex:
            failed = True
            print(f"   ! restore {dotted} failed: {ex}", flush=True)
    adb_shell(serial, "cmd", "statusbar", "send-disable-flag", "none")
    panel_sleep(serial)
    if not failed:
        _preflight_state_path(serial).unlink(missing_ok=True)


def _exit_on_signal(signum, _frame) -> None:
    """SIGTERM/SIGHUP end the run through the normal unwinding path, so the
    `finally` that restores the device settings runs."""
    raise SystemExit(128 + signum)


def assert_capture_ready(serial: str) -> None:
    """Preconditions before every capture: panel awake, refresh still pinned."""
    assert_awake(serial)
    if PINNED_REFRESH is not None:
        cur = active_refresh(serial)
        if cur is not None and abs(cur - PINNED_REFRESH) > 1.0:
            raise RuntimeError(
                f"refresh drifted: pinned {PINNED_REFRESH:g} Hz but "
                f"active vsync is {cur:g} Hz; aborting run")


def assert_foreground(serial: str, pkg: str) -> None:
    """The focused window belongs to the contestant.

    A keyguard or notification shade over the app leaves its process alive
    and its layers attached while every present comes from SystemUI; a
    capture taken then measures SystemUI, so it aborts the run."""
    out = adb_shell(serial, "dumpsys window displays")
    m = re.search(r"mCurrentFocus=Window\{\S+ \S+ ([^}]+)\}", out)
    focus = m.group(1) if m else None
    if focus == f"Application Not Responding: {pkg}":
        raise AppNotRespondingError(
            f"{pkg} is not responding: the system ANR dialog has focus")
    if focus is None or not focus.startswith(pkg + "/"):
        power = adb_shell(serial, "dumpsys power")
        wake = re.search(r"mWakefulness=(\w+)", power)
        keyguard = re.search(r"isKeyguardShowing=(\w+)", out)
        shade = adb_shell(serial, "dumpsys", "activity", "service",
                          "com.android.systemui/.SystemUIService")
        expanded = re.findall(r"(statusBarStateController\.state=\w+|"
                              r"isPanelExpanded\(\)=\w+|shadeExpansion: [\d.]+)",
                              shade)
        resumed = re.search(
            r"topResumedActivity=ActivityRecord\{\S+ \S+ (\S+)",
            adb_shell(serial, "dumpsys activity activities"))
        # with no focused window at all, what the input dispatcher and the
        # contestant's own window report is what names the cause
        focused_app = re.search(r"mFocusedApp=(.*)", out)
        dispatch = re.findall(r"(Focused(?:Application|Window)s:\n.*)",
                              adb_shell(serial, "dumpsys input"))
        windows = adb_shell(serial, "dumpsys window windows")
        own = [ln.strip() for ln in windows.splitlines()
               if pkg in ln or re.search(
                   r"mHasSurface|isVisible|mViewVisibility|isOnScreen", ln)]
        events = adb(serial, "logcat", "-d", "-b", "events", "-t", "40")
        raise RuntimeError(
            f"{pkg} is not the focused window (focus={focus!r}, "
            f"wakefulness={wake.group(1) if wake else None}, "
            f"keyguard={keyguard.group(1) if keyguard else None}, "
            f"shade={sorted(set(expanded))}, "
            f"top_resumed={resumed.group(1) if resumed else None}, "
            f"focused_app={focused_app.group(1).strip() if focused_app else None}, "
            f"input_dispatcher={dispatch}); "
            "aborting run: the capture would measure the window on top\n"
            "window state:\n" + "\n".join(own[:60])
            + "\nevents log:\n" + events)


def frame_capture(serial: str, c: dict, workload: str, warmup_ms: int,
                  cap_ms: int, drive: str | None, refresh: float | None,
                  marker_tolerance_ms: float) -> tuple[dict, dict]:
    """One frame source for every contestant: Perfetto FrameTimeline over
    the on-device capture program (launch + warmup + the fixed window,
    drive inside it); a capture error aborts the run. The window anchors
    on the launched process's own first present in the trace. Returns
    (frame stats, program report)."""
    assert_capture_ready(serial)
    local, prog = run_capture(serial, PERFETTO_CFG, c, workload, None,
                              warmup_ms, cap_ms, drive, sample_mem=True)
    st = analyze_trace(local, c["package"], prog["pid"], warmup_ms, cap_ms,
                       refresh, marker_tolerance_ms)
    assert_foreground(serial, c["package"])
    return st, prog


def _timeline_tables(tp) -> list[str]:
    """FrameTimeline slice tables present in this trace.

    `actual_frame_timeline_slice` keys frames by the vsync id the app's
    Choreographer posted; frames produced through SurfaceView/BLAST buffer
    streams (Flutter's pipeline) carry no such id, so they exist only in
    `surface_frame_timeline_slice`, SurfaceFlinger's per-layer tracker.
    Both belong to the same contestant's layers (layer_name contains the
    package, including `SurfaceView[pkg/...]` names) and a contestant that
    posts only surface frames must still measure as perfetto."""
    names = {r.name for r in tp.query(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name "
        "GLOB '*frame_timeline_slice'")}
    return [t for t in
            ("actual_frame_timeline_slice", "surface_frame_timeline_slice")
            if t in names]


def _table_columns(tp, table: str) -> set:
    """Columns of a trace table. Trace-processor tables are virtual
    (`CREATE VIRTUAL TABLE … USING __intrinsic_dataframe`), so their DDL
    names no columns; `pragma_table_info` is the source of truth."""
    return {r.name for r in tp.query(
        f"SELECT name FROM pragma_table_info('{table}')")}


def _owned_upid(tp, pid: int) -> int:
    """The trace's process entry for the launched contestant, by pid alone.

    Identity was established on the device: the capture program's `pidof
    <package>` named this pid after `am start -W` returned. The trace only
    has to map it to exactly one process whose lifetime covers the
    window: one with no recorded start (trace_processor created it from
    the pid FrameTimeline reported) or one forked inside the trace. The process name is never
    consulted: without a scan of the new pid it is NULL, and at fork it is
    still zygote's. A pid that maps to no row or to several (reuse inside
    the trace) cannot be attributed and fails the capture."""
    start = next(iter(tp.query(
        "SELECT start_ts FROM trace_bounds"))).start_ts
    rows = list(tp.query(
        f"SELECT upid, name, start_ts FROM process WHERE pid = {int(pid)} "
        f"AND (start_ts IS NULL OR start_ts >= {int(start)})"))
    if len(rows) != 1:
        raise RuntimeError(
            f"launched pid {pid} resolves to "
            f"{[(r.upid, r.name, r.start_ts) for r in rows]} in the trace's "
            "process table (trace start "
            f"{start}) — exactly one process must own the window; the "
            "capture cannot be attributed")
    return rows[0].upid


def _window_marker_ns(tp) -> tuple[int, int]:
    """The device program's `bench.window` slice: (drive start, hold end)
    in trace ns. Exactly one complete slice must exist — a missing or
    unterminated marker means the program's window was never recorded."""
    rows = list(tp.query(
        "SELECT ts, dur FROM slice "
        f"WHERE name = '{WINDOW_MARKER}'"))
    if len(rows) != 1 or rows[0].dur is None or rows[0].dur < 0:
        raise RuntimeError(
            f"the trace holds {[(r.ts, r.dur) for r in rows]} for the "
            f"{WINDOW_MARKER} marker — the capture program's window must "
            "be exactly one complete slice (ftrace/print recorded)")
    return rows[0].ts, rows[0].ts + rows[0].dur


def require_marker_at_window(marker_ns: int, window_start_ns: int,
                             tolerance_ms: float) -> float:
    """The drive starts at window start (METHOD). The device program opens
    its window `warmup` after `am start -W` returns; the trace's window
    opens `warmup` after the first owned present. The marker is the
    program's window start in trace time, so the difference is measured,
    returned as evidence and bounded by the manifest's declared tolerance
    ([pacing].window_marker_tolerance_ms) — a capture outside it fails."""
    offset_ms = (marker_ns - window_start_ns) / 1e6
    if abs(offset_ms) > tolerance_ms:
        raise RuntimeError(
            f"the drive started {offset_ms:+.1f} ms from the window start "
            f"(first owned present + warmup); the declared bound is "
            f"±{tolerance_ms:g} ms")
    return offset_ms


def _frame_rows(tp, upid: int):
    """Present rows of the owned process across every frame-timeline table
    the trace emitted, as (ts, dur, jank_type, layer_name, present_type)."""
    rows = []
    for table in _timeline_tables(tp):
        cols = _table_columns(tp, table)
        if "upid" not in cols:
            raise RuntimeError(
                f"{table} carries no upid column — its frames cannot be "
                "attributed to the launched process")
        sel = ["a.ts AS ts", "a.dur AS dur",
               "a.jank_type AS jank_type"
               if "jank_type" in cols else "NULL AS jank_type",
               "a.layer_name AS layer_name"
               if "layer_name" in cols else "NULL AS layer_name",
               "a.present_type AS present_type"
               if "present_type" in cols else "NULL AS present_type"]
        for r in tp.query(
                f"SELECT {', '.join(sel)} FROM {table} a "
                f"WHERE a.upid = {int(upid)} ORDER BY a.ts"):
            rows.append((r.ts, r.dur, r.jank_type, r.layer_name,
                         r.present_type))
    rows.sort(key=lambda r: r[0])
    return rows


def capture_window_ns(first_present_ns: int, warmup_ms: int,
                      cap_ms: int) -> tuple[int, int]:
    """[first owned present + warmup, + capture] in trace ns (METHOD)."""
    start = first_present_ns + int(warmup_ms * 1_000_000)
    return start, start + int(cap_ms * 1_000_000)


def require_window_covered(trace_end_ns: int, window: tuple[int, int]) -> None:
    """The trace must span the whole window: a capture that ended early
    would measure a shorter window than the one declared."""
    if trace_end_ns < window[1]:
        raise RuntimeError(
            f"trace ends {(window[1] - trace_end_ns) / 1e6:.1f} ms before "
            "the measurement window closes — the capture does not cover "
            "the declared window")


def analyze_trace(path: Path, pkg: str, pid: int, warmup_ms: int,
                  cap_ms: int, refresh: float | None,
                  marker_tolerance_ms: float,
                  with_cpu: bool = False) -> dict:
    """FrameTimeline presents of the launched process: present interval
    statistics over the window and the jank share from the frame-timeline
    tables; with `with_cpu`, CPU ms/frame of its UI + render threads over
    the same window.

    Raises NoFramesError when the process presented nothing — a capture
    miss is never aggregated around."""
    from perfetto.trace_processor import TraceProcessor
    with TraceProcessor(trace=str(path)) as tp:
        upid = _owned_upid(tp, pid)
        rows = _frame_rows(tp, upid)
        # dropped frames never presented — they are jank evidence, not
        # presents, and must not join the interval math
        presented = [r for r in rows
                     if not (r[4] and "Dropped" in str(r[4]))]
        if not presented:
            raise NoFramesError(
                f"no FrameTimeline presents for {pkg} pid {pid} in capture "
                f"(frame_source=perfetto)")
        # present timestamp = slice end; the window is anchored on the
        # process's own first present (METHOD), never on the capture start
        pts = [r[0] + (r[1] or 0) for r in presented]
        window = capture_window_ns(pts[0], warmup_ms, cap_ms)
        bounds = next(iter(tp.query(
            "SELECT start_ts, end_ts FROM trace_bounds")))
        require_window_covered(bounds.end_ts, window)
        marker = _window_marker_ns(tp)
        offset_ms = require_marker_at_window(marker[0], window[0],
                                             marker_tolerance_ms)
        cpu = _window_cpu(tp, upid, pkg, window) if with_cpu else None
    stats = lib_frames.frame_statistics(
        [t / 1e6 for t in pts], window[0] / 1e6, cap_ms,
        1000.0 / (refresh or 60.0))
    # jank_type != None is perfetto's own drop classification — kept as
    # evidence beside the unified interval rule, over the window's rows
    in_win = [r for r in rows
              if window[0] <= r[0] + (r[1] or 0) <= window[1]]
    janky = [r for r in in_win
             if str(r[2]) not in ("None", "null", "None.None", "")]
    stats["dropped_pct"] = (round(100.0 * len(janky) / len(in_win), 2)
                            if in_win else None)
    stats["drive_offset_ms"] = round(offset_ms, 3)
    stats["frames"] = stats.pop("presents")
    stats["ivals_ms"] = stats.pop("intervals_ms")
    if cpu is not None:
        frames = stats["frames"]
        stats["cpu_threads"] = {k: round(v, 1) for k, v in cpu.items()}
        sel = sum(v for k, v in cpu.items() if _ui_render_thread(k, pkg))
        stats["cpu_ms_per_frame"] = (round(sel / frames, 3)
                                     if frames and sel > 0 else None)
    return stats


# Threads counted as "UI + render" for CPU ms/frame: the process main thread
# (thread name == process name), the platform RenderThread, HWUI worker
# threads, raster threads (Flutter/Skia) and the JIT compiler thread.
def _ui_render_thread(tname: str, pkg: str) -> bool:
    return (
        # main thread is named after the process (comm is 15-char
        # truncated, so also match the truncated prefix)
        tname == pkg or pkg.startswith(tname)
        or tname.startswith("RenderThread")
        or tname.startswith("hwuiTask")
        or tname == "1.ui"
        or "raster" in tname.lower()
        or tname.startswith("Jit"))


def _window_cpu(tp, upid: int, pkg: str,
                window: tuple[int, int]) -> dict[str, float]:
    """Scheduler-slice CPU ms per thread of the owned process, clipped to
    the measurement window. Missing sched data raises — a trace that
    cannot attribute CPU is a harness failure, not a null data point."""
    lo, hi = window
    rows = list(tp.query(
        "SELECT th.name AS tname, "
        f"SUM(MIN(ss.ts + ss.dur, {hi}) - MAX(ss.ts, {lo})) AS cpu_ns "
        "FROM sched_slice ss JOIN thread th ON ss.utid = th.utid "
        f"WHERE th.upid = {int(upid)} AND ss.ts < {hi} "
        f"AND ss.ts + ss.dur > {lo} GROUP BY th.name"))
    if not rows:
        raise RuntimeError(
            f"no sched_slice rows for {pkg} inside the window — "
            "PERFETTO_CAP_CFG needs the sched ftrace events")
    return {(r.tname or "?"): (r.cpu_ns or 0) / 1e6 for r in rows}


BUDGET_120_MS = 8.33
# Device minutes one framework may take across all reps.
MEASURE_BUDGET_MIN = 30
BUDGET_60_MS = 16.67


def within_budget(ivals: list[float], budget_ms: float) -> float | None:
    """Fraction of present intervals inside one refresh period.

    Intervals cluster at exactly one vsync, so a present counts as in budget
    up to 1.5x the period — jitter across the vsync line is not a miss.
    Intervals >= 500 ms are deliberate drive idles (the fling program's
    inter-swipe settles), not frames — same exclusion as dropped_pct."""
    xs = [x for x in ivals if x < 500.0]
    if not xs:
        return None
    return round(sum(1 for x in xs if x <= 1.5 * budget_ms) / len(xs), 4)


def capacity_capture(serial: str, c: dict, workload: str, step: int,
                     warmup_ms: int, cap_ms: int, drive: str | None,
                     refresh: float | None,
                     marker_tolerance_ms: float) -> dict:
    """Frame capture for one capacity step, Perfetto FrameTimeline like
    every other capture plus CPU ms/frame from sched_slice. The step's
    launch and settle run inside the trace so the window anchors on THIS
    step's first owned present (settle_ms is the declared warmup)."""
    pkg = c["package"]
    assert_capture_ready(serial)
    local, prog = run_capture(serial, PERFETTO_CAP_CFG, c, workload, step,
                              warmup_ms, cap_ms, drive, sample_mem=False)
    try:
        st = analyze_trace(local, pkg, prog["pid"], warmup_ms, cap_ms,
                           refresh, marker_tolerance_ms, with_cpu=True)
    except NoFramesError:
        # live app, surfaces still present, zero presents: the collapse
        # point is a measurement; a dead process or vanished surfaces
        # remains a capture failure that aborts the run
        if not (process_alive(serial, pkg) and sf_layers(serial, pkg)):
            raise
        st = zero_frame_result()
        st["cpu_ms_per_frame"] = None
    # a step result is a measurement only while the app is on top
    assert_foreground(serial, pkg)
    return st


def screen_dims(serial: str) -> tuple[int, int]:
    out = adb_shell(serial, "wm size").rsplit(":", 1)[-1].strip()
    screen = re.match(r"(\d+)x(\d+)", out)
    if not screen:
        raise RuntimeError(f"unparseable `wm size` output: {out!r}")
    return int(screen.group(1)), int(screen.group(2))


def measure_capacity(man, name: str, c: dict, serial: str, wl: str,
                     dims: tuple[int, int], refresh: float | None) -> dict:
    """Stepped capacity workload: one cold launch per step, settle then hold.

    Stops early when pacing collapses (fewer than collapse_frac of presents
    within two 60 Hz budgets). A launch or capture error aborts the run.
    """
    spec = man["workloads"][wl]
    settle_ms = spec["settle_ms"]
    hold_ms = spec["hold_ms"]
    collapse_frac = spec["collapse_frac"]
    pkg = c["package"]
    out: dict = {"steps": [], "collapsed_at": None, "crashed": None,
                 "not_responding": None}
    for n in spec["steps"]:
        print(f"    {wl} step {n}", flush=True)
        try:
            # settle_ms is the declared warmup (never 0), hold_ms the
            # capture; W6 runs the fling program inside the hold
            st = capacity_capture(
                serial, c, wl, n, settle_ms, hold_ms,
                fling_program(man["fling"], dims) if wl == "w6" else None,
                refresh, man["pacing"]["window_marker_tolerance_ms"])
        except AppNotRespondingError:
            # the step blocked the main thread past the ANR timeout: the
            # ladder's limit. Stopping the app dismisses the dialog before
            # the next workload launches.
            print(f"    {pkg} not responding at {wl} step {n}", flush=True)
            out["not_responding"] = {"step": n}
            if out["collapsed_at"] is None:
                out["collapsed_at"] = n
            adb_shell(serial, f"am force-stop {pkg}")
            break
        except Exception:
            # a contestant process that dies mid-ladder is a result, not a
            # harness failure: record it and stop the ladder; the run
            # continues with the next workload and contestant. A live
            # process keeps every failure an abort.
            if not process_alive(serial, pkg):
                reason = crash_reason(serial, pkg)
                print(f"    {pkg} died at {wl} step {n}: "
                      f"{reason['exit']['reason']} / "
                      f"{reason['exit']['subreason']}: "
                      f"{reason['exit']['description']}; "
                      f"abort: {reason['abort']}", flush=True)
                out["crashed"] = {"crashed": True, "step": n, "reason": reason}
                break
            raise
        ivals = st.get("ivals_ms") or []
        rec = {
            "step": n,
            "frames": st.get("frames"),
            "frame_ms_p50": st.get("frame_ms_p50"),
            "frame_ms_p99": st.get("frame_ms_p99"),
            "fps": st.get("fps"),
            "within_120hz": within_budget(ivals, BUDGET_120_MS),
            "within_60hz": within_budget(ivals, BUDGET_60_MS),
            "cpu_ms_per_frame": st.get("cpu_ms_per_frame"),
            "drive_offset_ms": st.get("drive_offset_ms"),
        }
        if st.get("cpu_threads"):
            rec["cpu_threads"] = st["cpu_threads"]
        out["steps"].append(rec)
        # pacing collapse: fewer than collapse_frac of presents land inside
        # TWO 60 Hz budgets — the workload is drawing slower than ~30 fps,
        # not merely jittering at the refresh line.
        alive = within_budget(ivals, 2 * BUDGET_60_MS)
        # a step producing <4 presents in the hold window is collapsed
        # pacing by definition (~1 fps), likewise <collapse_frac inside
        # two 60 Hz budgets
        if ((st.get("frames") or 0) < 4
                or (alive is not None and alive < collapse_frac)):
            out["collapsed_at"] = n
            break
    adb_shell(serial, f"am force-stop {pkg}")
    return out


def fling_program(fling: dict, screen: tuple[int, int]) -> str:
    """The shared fling protocol (../WORKLOADS.md) as an on-device shell
    program: OS-level `input swipe` outside the app — 8 down then 2 up,
    75%→15% of the surface height, 250 ms gesture, 350 ms pause —
    identical for every contestant. It runs inside the capture program,
    so gesture pacing is exact: no host round-trip lands between swipes."""
    sw, shp = screen
    x = int(sw * fling["margin_x_frac"])
    y0, y1 = int(shp * fling["start_y_frac"]), int(shp * fling["end_y_frac"])
    dur, pause = fling["duration_ms"], fling["pause_between_ms"] / 1000.0
    steps = ([f"input swipe {x} {y0} {x} {y1} {dur}"] * fling["down_swipes"]
             + [f"input swipe {x} {y1} {x} {y0} {dur}"] * fling["up_swipes"])
    return "; ".join(f"{st}; sleep {pause}" for st in steps)


ALL_WORKLOADS = ("w1", "w2", "w3", "w4", "w5", "w6")


def measure_rep(man, name: str, c: dict, serial: str,
                artifacts: Path, workloads=ALL_WORKLOADS) -> dict:
    pkg, activity, kind = c["package"], c["activity"], c["kind"]
    dims = screen_dims(serial)

    pacing = man["pacing"]
    warmup_ms = pacing["warmup_ms"]
    rep = {"refresh_hz": refresh_hz(serial),
           "thermal_state": thermal_status(serial)}
    for w in workloads:
        if w in ("w5", "w6"):
            # capacity workloads drive their own per-step launches
            rep[w] = {"capacity": measure_capacity(
                man, name, c, serial, w, dims, rep["refresh_hz"])}
            continue
        wr = {}
        if w == "w1":
            # startup + memory only — no frame window. `am start -W` is the
            # first-frame event; steady memory is sampled after the same
            # declared warmup as the measured workloads
            wr["startup_ms"] = launch(serial, pkg, activity, w, kind)
            time.sleep(warmup_ms / 1000.0)
            wr["memory_steady"] = proc_mem(serial, pkg)
            wr["memory_peak_kb"] = wr["memory_steady"]["pss_kb"]
            rep[w] = wr
            continue
        # launch + warmup + the fixed window run as one on-device program
        # inside the trace: the window anchors on the contestant's first
        # owned present and the drive starts at window start (METHOD);
        # W3 animates by itself, so its window holds without input
        frames, prog = frame_capture(
            serial, c, w, warmup_ms, pacing["capture_ms"][w],
            fling_program(man["fling"], dims) if w in ("w2", "w4") else None,
            rep["refresh_hz"], pacing["window_marker_tolerance_ms"])
        wr["startup_ms"] = prog["startup_ms"]
        wr["memory_steady"] = prog["memory_steady"]
        wr["memory_peak_kb"] = prog["memory_peak_kb"]
        wr["frames"] = frames
        rep[w] = wr
    adb_shell(serial, f"am force-stop {pkg}")
    return rep


def harness_fingerprint(man, workloads, reps: int, artifacts: Path) -> str:
    """Identity of a measurement protocol: this runner, the manifest, the
    workload set, the rep count and every contestant artifact. A saved run
    resumes only under the same fingerprint; anything else starts over."""
    h = hashlib.sha256()
    h.update(Path(__file__).read_bytes())
    h.update(MANIFEST.read_bytes())
    h.update(",".join(workloads).encode())
    h.update(str(reps).encode())
    for n in sorted(man["contestants"]):
        for f in sorted((artifacts / n).glob("*.apk")):
            h.update(f.name.encode())
            h.update(hashlib.sha256(f.read_bytes()).digest())
    return h.hexdigest()


def cmd_measure(man, serial: str, reps: int, locks_dir: Path | None,
                artifacts: Path, results: dict, workloads=ALL_WORKLOADS,
                save=lambda: None, development: bool = False):
    """Interleaved reps over every contestant.

    Each finished (rep, contestant) is saved at once, and a run started
    again under the same fingerprint continues after the last saved one,
    so stopping a run loses at most the contestant-rep in flight."""
    names = list(man["contestants"].keys())
    fp = harness_fingerprint(man, workloads, reps, artifacts)
    if results.get("fingerprint") != fp:
        for entry in results.get("results", {}).values():
            entry.pop("runs", None)
            entry.pop("measure_s", None)
        results.get("device", {}).pop("frame_source", None)
        results["fingerprint"] = fp
    res = results.setdefault("results", {})
    dev = results.setdefault("device", {})
    dev["serial"] = serial
    dev["abi"] = adb_shell(serial, "getprop ro.product.cpu.abi").strip()
    dev["model"] = adb_shell(serial, "getprop ro.product.model").strip()
    dev["android_release"] = adb_shell(
        serial, "getprop ro.build.version.release").strip()
    dev["sdk"] = adb_shell(serial, "getprop ro.build.version.sdk").strip()
    dev["gpu"] = gpu_adapter(serial)
    if is_software_gpu(dev["gpu"]):
        results["gpu_software"] = True
        if not development:
            raise SystemExit(
                f"refusing to measure on '{dev['gpu']}': a software "
                "rasterizer makes frame-time and memory numbers worthless "
                "as evidence. Re-run on hardware, or pass --development to "
                "record the run as development-only.")
    if development:
        results["development_only"] = True

    # Panel must stay awake at the pinned refresh rate for the whole run
    # so Perfetto FrameTimeline emits slices under a uniform vsync. Prior
    # settings are restored at exit even when a capture aborts.
    signal.signal(signal.SIGTERM, _exit_on_signal)
    signal.signal(signal.SIGHUP, _exit_on_signal)
    # the device is this run's from the first setting it pins to the last
    # one it restores: preflight, install and every rep happen under one
    # lock, so no other session's work can land between two of them
    lock = DeviceLock(locks_dir, serial) if locks_dir else _nullctx()
    with lock:
        _measure_locked(man, serial, reps, locks_dir, artifacts, results,
                        workloads, save, names, res, dev)


def _measure_locked(man, serial: str, reps: int, locks_dir: Path | None,
                    artifacts: Path, results: dict, workloads, save,
                    names: list[str], res: dict, dev: dict) -> None:
    prev = device_preflight(serial)
    try:
        dev["vsync_hz"] = PINNED_REFRESH
        dev["vsync_hz_active"] = active_refresh(serial)
        abi = dev["abi"]
        dev["frame_source"] = "perfetto"
        model = dev.get("model", "this target")
        lims: list[str] = [
            f"{model}: frame metrics for every contestant use Perfetto "
            "FrameTimeline (actual and surface frame_timeline_slice over "
            "the contestant's layers), the issue-mandated single source; "
            "CPU ms/frame from sched_slice on UI+render threads."
        ]
        results["limitations"] = lims
        dev["limitations"] = lims
        for rep in range(reps):
            # interleaved order: rotate contestant order each rep so thermal
            # drift and background load are shared evenly
            order = names[rep % len(names):] + names[:rep % len(names)]
            for name in order:
                c = man["contestants"][name]
                if (artifacts / name / "BUILD_ERROR.txt").exists():
                    continue
                if len(res.get(name, {}).get("runs", [])) > rep:
                    continue  # saved by an earlier run of this protocol
                if locks_dir:
                    st = wait_nominal(serial)
                    print(f"rep {rep} {name}: thermal {st}", flush=True)
                print(f"rep {rep} {name}: measuring", flush=True)
                # the measured binary is installed+verified per rep —
                # two contestants share dev.waterui.bench, and anything
                # else on device could have replaced it between reps
                entry = res.setdefault(name, {})
                apk = artifacts / name / apk_for_abi(c, abi)
                try:
                    verify_installed(serial, c["package"], apk)
                except RuntimeError as ex:
                    entry.setdefault("runs", []).append(
                        {"workload": None, "repeat": rep,
                         "error": f"install verify: {ex}"})
                    print(f"rep {rep} {name}: FAILED {ex}", flush=True)
                    save()
                    continue
                # a capture error aborts the run — never stored as a
                # per-run error and aggregated around
                t0 = time.monotonic()
                since = adb_shell(serial, "date +%s.%3N").strip()
                r = measure_rep(man, name, c, serial, artifacts, workloads)
                installs = package_installs_since(serial, since)
                if installs:
                    # the store updating apps kills and restarts their
                    # processes and loads CPU and storage under the
                    # capture: the rep measured the update, not the
                    # contestant — it fails, it is never taken again
                    raise RuntimeError(
                        f"rep {rep} {name}: packages were installed during "
                        f"the rep ({', '.join(installs)}) — the rep is not "
                        "a measurement; nothing from it is saved")
                spent = time.monotonic() - t0
                entry.setdefault("runs", []).append(r)
                entry["measure_s"] = round(entry.get("measure_s", 0) + spent, 1)
                print(f"rep {rep} {name}: {spent:.0f} s "
                      f"(total {entry['measure_s'] / 60:.1f} min)", flush=True)
                save()
        check_repetitions(res, names, reps, artifacts)
        if locks_dir:
            wait_nominal(serial)
    finally:
        device_preflight_restore(serial, prev)


def check_repetitions(res: dict, names: list[str], reps: int,
                      artifacts: Path) -> None:
    """A cell counts only with >= `reps` successful runs — `runs`
    records successes only (a capture error aborts the run, never
    stored as data); a shortfall rejects the whole measurement."""
    short = []
    for name in names:
        if (artifacts / name / "BUILD_ERROR.txt").exists():
            continue
        entry = res.get(name, {})
        ok = sum(1 for r in entry.get("runs", []) if not r.get("error"))
        entry["successful_reps"] = ok
        if ok < reps:
            short.append(f"{name}: {ok}/{reps} successful")
    if short:
        raise SystemExit(
            "repetition requirement not met — refusing to emit an "
            "incomplete measurement: " + "; ".join(short))


def package_installs_since(serial: str, since: str) -> list[str]:
    """Packages the system replaced after device time `since` (epoch s).

    Every install stops the old package's processes with the reason
    `installPackageLI` in the events log."""
    log = adb(serial, "logcat", "-d", "-b", "events", "-T", since)
    return sorted(set(re.findall(
        r"am_kill\s*:\s*\[\d+,\d+,([^,\]]+),\d+,stop \S+ due to "
        r"installPackageLI", log)))


class _nullctx:
    def __enter__(self):
        return self
    def __exit__(self, *a):
        return False


# ---------------------------------------------------------------------------
# aggregation + report
# ---------------------------------------------------------------------------


def capacity_at(rep_w: dict, budget_key: str) -> int | None:
    """Largest step of a capacity workload keeping >=99% within budget."""
    cap = rep_w.get("capacity", {})
    ok = [s["step"] for s in cap.get("steps", [])
          if s.get(budget_key) is not None and s[budget_key] >= 0.99]
    return max(ok) if ok else None


def aggregate(results: dict):
    """Per contestant x workload x metric: median/min/max/samples."""
    agg = {}
    for name, c in results.get("results", {}).items():
        runs = c.get("runs", [])
        ca = {}
        for w in ("w5", "w6"):
            reps = [r[w]["capacity"] for r in runs
                    if w in r and "capacity" in r.get(w, {})]
            if not reps:
                continue
            cap_stats = {}
            for budget in ("within_120hz", "within_60hz"):
                xs = [capacity_at({"capacity": rr}, budget) for rr in reps]
                xs = [x for x in xs if x is not None]
                if xs:
                    # a capacity is a ladder step that was measured: an even
                    # rep count takes the lower middle, never the mean of two
                    cap_stats[f"capacity_{budget[7:]}"] = _stat(
                        xs, median=statistics.median_low)
            # per-step medians across reps
            step_ids = sorted({s["step"] for rr in reps for s in rr["steps"]})
            per_step = []
            for n in step_ids:
                cells = [next(s for s in rr["steps"] if s["step"] == n)
                         for rr in reps if any(s["step"] == n
                                               for s in rr["steps"])]
                row = {"step": n, "n_runs": len(cells)}
                for k in ("frame_ms_p50", "frame_ms_p99", "fps",
                          "within_120hz", "within_60hz",
                          "cpu_ms_per_frame"):
                    xs = [s[k] for s in cells if s.get(k) is not None]
                    row[k] = round(statistics.median(xs), 3) if xs else None
                per_step.append(row)
            ca[w] = {"capacity": cap_stats, "steps": per_step}
        for w in ("w1", "w2", "w3", "w4"):
            metrics = {}
            for key in ("startup_ms", "memory_peak_kb"):
                xs = [r[w][key] for r in runs
                      if w in r and r[w].get(key) is not None]
                if xs:
                    metrics[key] = _stat(xs)
            for key in ("pss_kb", "rss_kb"):
                xs = [r[w]["memory_steady"][key] for r in runs
                      if w in r and r[w].get("memory_steady", {}).get(key)]
                if xs:
                    metrics[f"memory_steady_{key}"] = _stat(xs)
            for key in ("frame_ms_p50", "frame_ms_p90", "frame_ms_p99",
                        "dropped_pct", "fps"):
                xs = [r[w]["frames"][key] for r in runs
                      if w in r and r[w].get("frames", {}).get(key) is not None]
                if xs:
                    metrics[f"frames_{key}"] = _stat(xs)
            if metrics:
                ca[w] = metrics
        ca["package_size"] = c.get("package_size")
        agg[name] = ca
    results["aggregates"] = agg


def _stat(xs, median=statistics.median):
    return {"median": round(median(xs), 3), "min": min(xs),
            "max": max(xs), "samples": xs, "n": len(xs)}


def cmd_report(results: dict) -> str:
    agg = results["aggregates"]
    names = list(agg.keys())
    lines = ["# Competitive benchmark — Android baseline\n"]
    lines.append("Issue: water-rs/waterui#1262 · generated from results.json\n")
    dev = results.get("device", {})
    if results.get("development_only"):
        lines.append(
            "> **Development-only run** — the measured GPU adapter "
            f"({dev.get('gpu','?')}) is a software rasterizer; the "
            "frame-time and memory numbers below are not publishable "
            "evidence.\n")
    lines.append(
        f"Device: {dev.get('model','?')} (Android {dev.get('android_release','?')}, "
        f"sdk {dev.get('sdk','?')}, {dev.get('abi','?')}, serial {dev.get('serial','?')})\n")
    if dev.get("gpu"):
        lines.append(f"GPU: {dev['gpu']}\n")
    if results.get("fingerprint"):
        lines.append(
            f"Harness+artifact fingerprint: `{results['fingerprint']}`\n")
    mach = results.get("machine", {})
    lines.append(
        f"Host: {mach.get('system','?')} {mach.get('release','?')} "
        f"{mach.get('machine','?')}, Python {mach.get('python','?')}\n")

    def ratio(v, ref):
        return f"{v/ref:.2f}×" if v is not None and ref else "—"

    wui = agg.get("waterui", {})
    for label, key, unit in [
        ("Startup (cold → first frame, ms)", "startup_ms", "w1"),
        ("Steady PSS (KB)", "memory_steady_pss_kb", "w1"),
        ("Peak PSS (KB)", "memory_peak_kb", "w1"),
    ]:
        lines.append(f"\n## {label} — W1\n")
        lines.append("| contestant | median | min | max | n | "
                     "ratio to WaterUI |")
        lines.append("|---|---|---|---|---|---|")
        base = wui.get(unit, {}).get(key, {}).get("median")
        for n in names:
            s = agg[n].get(unit, {}).get(key)
            if s:
                lines.append(
                    f"| {n} | {s['median']} | {s['min']} | {s['max']} | "
                    f"{s.get('n', '—')} | {ratio(s['median'], base)} |")
            else:
                lines.append(f"| {n} | — | — | — | — | — |")

    for w in ("w2", "w3"):
        lines.append(f"\n## Frame timing — {w.upper()}\n")
        lines.append("| contestant | p50 ms | p90 ms | p99 ms | dropped % | fps | n |")
        lines.append("|---|---|---|---|---|---|---|")
        for n in names:
            f = agg[n].get(w, {})
            g = lambda k: f.get(f"frames_{k}", {}).get("median", "—")
            nrun = f.get("frames_frame_ms_p50", {}).get("n", 0)
            lines.append(
                f"| {n} | {g('frame_ms_p50')} | {g('frame_ms_p90')} | "
                f"{g('frame_ms_p99')} | {g('dropped_pct')} | {g('fps')} | "
                f"{nrun} |")
        sparse = [
            n for n in names
            if 0 < agg[n].get(w, {}).get("frames_frame_ms_p50", {}).get("n", 0)
            < sum(1 for r in results.get("results", {}).get(n, {})
                  .get("runs", []) if w in r)
        ]
        if sparse:
            lines.append(
                f"\n_{w}: {', '.join(sparse)} have <5 frame samples — a rep "
                "whose capture window found <4 presents on the app's busiest "
                "layer is recorded as a per-run error in results.json rather "
                "than substituted._")

    # --- capacity workloads W5/W6 -----------------------------------------
    cap_wls = [w for w in ("w5", "w6") if any(agg[n].get(w) for n in names)]
    for w in cap_wls:
        title = ("W5 Motion capacity" if w == "w5" else "W6 Feed capacity")
        lines.append(f"\n## Capacity — {title}\n")
        lines.append(
            "Capacity = largest step with ≥99% of presents inside the budget "
            "(median across reps)."
        )
        lines.append("\n| contestant | capacity @120 Hz (8.33 ms) | "
                     "capacity @60 Hz (16.67 ms) | collapsed at |")
        lines.append("|---|---|---|---|")
        for n in names:
            cw = agg[n].get(w, {})
            if not cw:
                lines.append(f"| {n} | — | — | — |")
                continue
            c120 = cw.get("capacity", {}).get("capacity_120hz", {})
            c60 = cw.get("capacity", {}).get("capacity_60hz", {})
            m120 = c120.get("median", "—")
            m60 = c60.get("median", "—")
            collapsed = [s for r in results["results"][n].get("runs", [])
                         for s in [r.get(w, {}).get("capacity", {}).get(
                             "collapsed_at")] if s is not None]
            crashed = [s for r in results["results"][n].get("runs", [])
                       for s in [r.get(w, {}).get("capacity", {}).get(
                           "crashed")] if s]
            hung = [s for r in results["results"][n].get("runs", [])
                    for s in [r.get(w, {}).get("capacity", {}).get(
                        "not_responding")] if s]
            coll = (str(int(statistics.median(collapsed)))
                    if collapsed else "—")
            if hung:
                coll += "; not responding at " + ",".join(
                    str(s["step"]) for s in hung)
            if crashed:
                coll += "; died at " + ", ".join(
                    f"{s['step']} ({death_summary(s['reason'])})"
                    for s in crashed)
            lines.append(f"| {n} | {m120} | {m60} | {coll} |")
        lines.append("\n| step | " + " | ".join(names) + " |")
        lines.append("|---|" + "---|" * len(names))
        all_steps = sorted({s["step"] for n in names
                            for s in agg[n].get(w, {}).get("steps", [])})
        for stp in all_steps:
            cells = []
            for n in names:
                row = next((s for s in agg[n].get(w, {}).get("steps", [])
                            if s["step"] == stp), None)
                if row is None or row.get("frame_ms_p50") is None:
                    cells.append("—")
                else:
                    cpu = row.get("cpu_ms_per_frame")
                    cpu_s = f"{cpu}" if cpu is not None else "n/a"
                    cells.append(
                        f"p50 {row['frame_ms_p50']} · p99 "
                        f"{row['frame_ms_p99']} · in60 {row['within_60hz']} "
                        f"· cpu {cpu_s}")
            lines.append(f"| {stp} | " + " | ".join(cells) + " |")
        lines.append(
            "\n_Cells: p50/p99 present-interval ms (active intervals only) "
            "· in60 = share inside the 16.67 ms budget · cpu = CPU ms/frame "
            "on UI+render threads (perfetto sched slices; n/a on fallback "
            "capture paths)._"
        )

    lines.append("\n## Package size\n")
    lines.append("| contestant | APK arm64 (B) | APK x86_64 (B) | "
                 "AAB arm64 download (B) | ratio arm64 |")
    lines.append("|---|---|---|---|---|")
    sha_rows = []
    base = (wui.get("package_size") or {}).get("apk_arm64", {}).get("apk_bytes")
    for n in names:
        ps = agg[n].get("package_size")
        if not ps and not results["results"][n].get("build_error"):
            raise RuntimeError(f"{n}: no package sizes in the results; run "
                               "`bench.py collect` (measure collects them)")
        ps = ps or {}
        a = ps.get("apk_arm64", {}).get("apk_bytes", "—")
        x = ps.get("apk_x86_64", {}).get("apk_bytes", "—")
        b = ps.get("aab", {}).get("aab_arm64_download_bytes", "—")
        r = ratio(a if isinstance(a, int) else None, base)
        lines.append(f"| {n} | {a} | {x} | {b} | {r} |")
        sha_rows.append(
            f"| {n} | {ps.get('apk_arm64', {}).get('sha256', '—')} | "
            f"{ps.get('aab', {}).get('aab_sha256', '—')} |")

    if any("—" not in row for row in sha_rows):
        lines.append("\n## Artifact fingerprints (SHA256)\n")
        lines.append("| contestant | arm64 APK | AAB |")
        lines.append("|---|---|---|")
        lines.extend(sha_rows)

    timed = [(n, c["measure_s"]) for n, c in results.get("results", {}).items()
             if c.get("measure_s") is not None]
    if timed:
        lines.append("\n## Measurement time\n")
        lines.append("Device time per framework across all reps "
                     f"(budget {MEASURE_BUDGET_MIN} min).\n")
        lines.append("| framework | minutes | successful reps | "
                     "within budget |")
        lines.append("|---|---|---|---|")
        for n, sec in timed:
            runs = sum(1 for r in results["results"][n].get("runs", [])
                       if not r.get("error"))
            ok = "yes" if sec / 60 <= MEASURE_BUDGET_MIN else "**no**"
            lines.append(f"| {n} | {sec / 60:.1f} | {runs} | {ok} |")

    lims = results.get("limitations", [])
    if lims:
        lines.append("\n## Limitations\n")
        for l in lims:
            lines.append(f"- {l}")
    return "\n".join(lines) + "\n"


def _fixture_trace(path: Path, *, pid: int, fork: bool,
                   scan_pid: bool, presents_ns: list[int],
                   marker_ns: tuple[int, int] | None) -> None:
    """A synthetic Perfetto trace shaped like a capture: the start-of-trace
    process scan, optionally the contestant's fork (task_newtask, as the
    capacity config records it), the device program's ftrace `print`
    window marker, and the contestant's FrameTimeline surface frames.

    The pip `perfetto` protos omit FrameTimelineEvent (TracePacket field
    76), so its frames are serialized from a runtime descriptor of the
    upstream message layout and merged into the packet as that field."""
    from google.protobuf import (descriptor_pb2, descriptor_pool,
                                 message_factory)
    from perfetto.trace_builder.proto_builder import TraceProtoBuilder
    F = descriptor_pb2.FieldDescriptorProto
    fd = descriptor_pb2.FileDescriptorProto(
        name="frame_timeline_fixture.proto", package="fixture",
        syntax="proto2")

    def message(name: str, fields) -> None:
        m = fd.message_type.add(name=name)
        for fname, num, typ, tname in fields:
            f = m.field.add(name=fname, number=num, type=typ,
                            label=F.LABEL_OPTIONAL)
            if tname:
                f.type_name = tname
    message("ActualSurfaceFrameStart", [
        ("cookie", 1, F.TYPE_INT64, None),
        ("token", 2, F.TYPE_INT64, None),
        ("display_frame_token", 3, F.TYPE_INT64, None),
        ("pid", 4, F.TYPE_INT32, None),
        ("layer_name", 5, F.TYPE_STRING, None),
        ("present_type", 6, F.TYPE_INT32, None),
        ("on_time_finish", 7, F.TYPE_BOOL, None),
        ("gpu_composition", 8, F.TYPE_BOOL, None),
        ("jank_type", 9, F.TYPE_INT32, None),
        ("prediction_type", 10, F.TYPE_INT32, None),
        ("is_buffer", 11, F.TYPE_BOOL, None)])
    message("FrameEnd", [("cookie", 1, F.TYPE_INT64, None)])
    message("FrameTimelineEvent", [
        ("actual_surface_frame_start", 4, F.TYPE_MESSAGE,
         ".fixture.ActualSurfaceFrameStart"),
        ("frame_end", 5, F.TYPE_MESSAGE, ".fixture.FrameEnd")])
    message("Packet", [
        ("timestamp", 8, F.TYPE_UINT64, None),
        ("trusted_packet_sequence_id", 10, F.TYPE_UINT32, None),
        ("frame_timeline_event", 76, F.TYPE_MESSAGE,
         ".fixture.FrameTimelineEvent")])
    pool = descriptor_pool.DescriptorPool()
    pool.Add(fd)
    Packet = message_factory.GetMessageClass(
        pool.FindMessageTypeByName("fixture.Packet"))

    b = TraceProtoBuilder()
    t0 = presents_ns[0] - 500_000_000
    pk = b.add_packet()
    pk.timestamp = t0
    for spid, name in [(1, "/system/bin/init"), (700, "zygote64")] + (
            [(pid, "com.example.stale")] if scan_pid else []):
        pr = pk.process_tree.processes.add()
        pr.pid, pr.ppid = spid, (0 if spid == 1 else 1)
        pr.cmdline.append(name)
    pk = b.add_packet()
    pk.timestamp = t0
    pk.trusted_packet_sequence_id = 1
    fb = pk.ftrace_events
    fb.cpu = 0
    if fork:
        ev = fb.event.add()
        ev.timestamp, ev.pid = t0 + 100_000_000, 700
        ev.task_newtask.pid = pid
        ev.task_newtask.comm = "zygote64"
        ev.task_newtask.clone_flags = 0
    if marker_ns is not None:
        for ts, buf in ((marker_ns[0], f"B|900|{WINDOW_MARKER}\n"),
                        (marker_ns[1], "E|900\n")):
            ev = fb.event.add()
            ev.timestamp, ev.pid = ts, 900
            ev.print.buf = buf
    for i, end in enumerate(presents_ns):
        start = Packet(timestamp=end - 4_000_000,
                       trusted_packet_sequence_id=2)
        a = start.frame_timeline_event.actual_surface_frame_start
        a.cookie, a.token, a.display_frame_token = i + 1, 1000 + i, 5000 + i
        a.pid, a.layer_name = pid, "dev.bench.views/dev.bench.views.Main#0"
        a.present_type, a.on_time_finish, a.gpu_composition = 1, True, False
        a.jank_type, a.prediction_type, a.is_buffer = 1, 1, True
        b.add_packet().MergeFromString(start.SerializeToString())
        fin = Packet(timestamp=end, trusted_packet_sequence_id=2)
        fin.frame_timeline_event.frame_end.cookie = i + 1
        b.add_packet().MergeFromString(fin.SerializeToString())
    path.write_bytes(b.serialize())


def _self_test_trace_attribution() -> None:
    """Pid attribution, the window marker and the window arithmetic of
    analyze_trace against synthetic traces in the real trace_processor."""
    from perfetto.trace_processor import TraceProcessor
    pid, period = 4242, 8_333_333
    first = 2_000_000_000
    presents = [first + i * period for i in range(120)]  # 1 s at 120 Hz
    warm, cap = 100, 500
    win_start = first + warm * 1_000_000
    good_marker = (win_start + 20_000_000, win_start + 20_000_000
                   + cap * 1_000_000)
    with tempfile.TemporaryDirectory() as td:
        def trace(name, **kw) -> Path:
            path = Path(td) / f"{name}.perfetto-trace"
            _fixture_trace(path, pid=pid, presents_ns=presents, **kw)
            return path
        # W2/W3/W4 shape: no fork event, the process row comes from
        # FrameTimeline's pid with no name — attributed by pid alone
        w2 = trace("w2", fork=False, scan_pid=False, marker_ns=good_marker)
        st = analyze_trace(w2, "dev.bench.views", pid, warm, cap, 120.0,
                           100)
        assert st["drive_offset_ms"] == 20.0, st
        assert st["frames"] == 60, st
        # capacity shape: forked inside the trace, comm still zygote's
        cap_trace = trace("cap", fork=True, scan_pid=False,
                          marker_ns=good_marker)
        with TraceProcessor(trace=str(cap_trace)) as tp:
            upid = _owned_upid(tp, pid)
            name = next(iter(tp.query(
                f"SELECT name FROM process WHERE upid = {upid}"))).name
            assert name != "dev.bench.views", name
        # pid reuse inside the trace: the start scan and the fork both
        # claim the pid — two candidate processes, no attribution
        reuse = trace("reuse", fork=True, scan_pid=True,
                      marker_ns=good_marker)
        with TraceProcessor(trace=str(reuse)) as tp:
            try:
                _owned_upid(tp, pid)
            except RuntimeError as e:
                assert "cannot be attributed" in str(e)
            else:
                raise AssertionError("ambiguous pid attributed")
        # the drive started 150 ms after the window opened: out of bound
        late = trace("late", fork=False, scan_pid=False,
                     marker_ns=(win_start + 150_000_000,
                                win_start + 650_000_000))
        try:
            analyze_trace(late, "dev.bench.views", pid, warm, cap, 120.0,
                          100)
        except RuntimeError as e:
            assert "+150.0 ms from the window start" in str(e), e
        else:
            raise AssertionError("late drive accepted")
        # no marker recorded: the program's window is unknown
        bare = trace("bare", fork=False, scan_pid=False, marker_ns=None)
        try:
            analyze_trace(bare, "dev.bench.views", pid, warm, cap, 120.0,
                          100)
        except RuntimeError as e:
            assert WINDOW_MARKER in str(e), e
        else:
            raise AssertionError("capture without a window marker accepted")


def _self_test() -> None:
    """Failure-direction checks through the report path — no device."""
    toolchain._self_test()
    with tempfile.TemporaryDirectory() as td:
        art = Path(td)
        ok_run = {"w1": {"startup_ms": 100, "memory_peak_kb": 6000,
                          "memory_steady": {"pss_kb": 5000,
                                            "rss_kb": 6000}}}
        res = {
            "ok": {"runs": [dict(ok_run) for _ in range(5)]},
            "short": {"runs": [dict(ok_run) for _ in range(3)]},
            "broken": {"build_error": "javac blew up"},
        }
        (art / "broken").mkdir()
        (art / "broken" / "BUILD_ERROR.txt").write_text("javac blew up")
        # mixed: 3/5 reps rejected; all-failure: nothing meets the floor
        try:
            check_repetitions(res, ["ok", "short", "broken"], 5, art)
        except SystemExit as e:
            assert "short: 3/5" in str(e)
        else:
            raise AssertionError("3/5 reps accepted")
        try:
            check_repetitions(res, ["ok", "short"], 99, art)
        except SystemExit:
            pass
        else:
            raise AssertionError("all-failure run accepted")
        check_repetitions(res, ["ok"], 5, art)
        assert res["ok"]["successful_reps"] == 5

        # report renders on the mixed data: n column + fingerprints
        results = {
            "machine": {}, "device": {"gpu": "Adreno (TM) 830"},
            "fingerprint": "f" * 64,
            "results": {
                "ok": {**res["ok"], "measure_s": 10,
                       "package_size": {
                           "apk_arm64": {"apk_bytes": 1,
                                         "apk_uncompressed_bytes": 2,
                                         "sha256": "a" * 64},
                           "aab": {"aab_arm64_download_bytes": 3,
                                   "aab_sha256": "b" * 64}}},
                "short": {**res["short"], "measure_s": 5,
                          "package_size": {
                              "apk_arm64": {"apk_bytes": 9,
                                            "apk_uncompressed_bytes": 9,
                                            "sha256": "c" * 64}}},
            },
        }
        aggregate(results)
        text = cmd_report(results)
        assert "| n |" in text and "successful reps" in text
        assert "Artifact fingerprints" in text and "a" * 64 in text
        assert "f" * 64 in text

        # gradle bootstrap: a properties file not pointing at the pinned
        # distribution is rejected before anything is fetched
        proj = art / "proj"
        (proj / "gradle" / "wrapper").mkdir(parents=True)
        (proj / "gradle" / "wrapper" /
         "gradle-wrapper.properties").write_text(
            "distributionUrl="
            "https\\://services.gradle.org/distributions/"
            "gradle-8.0-bin.zip\n")
        try:
            ensure_gradle_wrapper(proj, "9.6.1", {})
        except RuntimeError as e:
            assert "manifest pins" in str(e)
        else:
            raise AssertionError("wrong distributionUrl accepted")

    # capture program: launch, warmup, then the fixed window — the drive
    # and the memory sampler start at window start, the hold is the
    # capture, and an unfinished drive fails the capture
    fling = {"margin_x_frac": 0.5, "start_y_frac": 0.75,
             "end_y_frac": 0.15, "duration_ms": 250,
             "pause_between_ms": 350, "down_swipes": 8, "up_swipes": 2}
    drv = fling_program(fling, (1000, 2000))
    assert drv.count("input swipe 500 1500 500 300 250") == 8
    assert drv.count("input swipe 500 300 500 1500 250") == 2
    prog = device_program(
        am_start_cmd("dev.bench.views", ".MainActivity", "w2", "native"),
        "dev.bench.views", 4000, 12000, drv, sample_mem=True)
    order = [prog.index(k) for k in (
        "am start -W -n dev.bench.views/.MainActivity --es workload w2",
        "sleep 4.0", "BENCH_MEM_STEADY",
        f'echo "B|$$|{WINDOW_MARKER}" > {TRACE_MARKER}',
        "sleep 12.0 & hold=$!", "input swipe", "BENCH_MEM_SAMPLE",
        "wait $hold", f'echo "E|$$" > {TRACE_MARKER}',
        "outlasted the capture window", "wait $mem")]
    assert order == sorted(order), prog
    _self_test_trace_attribution()
    assert "--es waterui.env.BENCH_STEP 400" in am_start_cmd(
        "dev.waterui.bench", ".MainActivity", "w5", "waterui", 400)
    out = ("Status: ok\nTotalTime: 812\nWaitTime: 815\nComplete\n"
           "BENCH_PID 4242\n"
           "BENCH_MEM_STEADY Pss:  51200 kB Rss:  90000 kB \n"
           "BENCH_MEM_SAMPLE Pss:  52000 kB\n"
           "BENCH_MEM_SAMPLE Pss:  60100 kB\n")
    rep = parse_program_output(out, sample_mem=True)
    assert rep == {"startup_ms": 812.0, "pid": 4242,
                   "memory_steady": {"pss_kb": 51200, "rss_kb": 90000},
                   "memory_peak_kb": 60100}, rep
    for bad in (out.replace("TotalTime: 812\n", ""),
                out.replace("BENCH_PID 4242\n", ""),
                out + "BENCH_ERR drive program outlasted the capture window\n",
                out.replace("BENCH_MEM_SAMPLE Pss:  52000 kB\n", "")
                   .replace("BENCH_MEM_SAMPLE Pss:  60100 kB\n", "")):
        try:
            parse_program_output(bad, sample_mem=True)
        except RuntimeError:
            pass
        else:
            raise AssertionError(f"accepted a broken capture:\n{bad}")
    # the window is first owned present + warmup, capture wide, and a
    # trace ending before its close is a failed capture
    win = capture_window_ns(1_000_000_000, 4000, 12000)
    assert win == (5_000_000_000, 17_000_000_000)
    require_window_covered(17_000_000_000, win)
    try:
        require_window_covered(16_990_000_000, win)
    except RuntimeError as e:
        assert "10.0 ms before" in str(e)
    else:
        raise AssertionError("truncated window accepted")
    print("android bench self-test ok")


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("build")
    sub.add_parser("selftest")
    p = sub.add_parser("collect")
    p.add_argument("--serial", help="size the AAB for this attached device "
                   "(bundletool get-device-spec) instead of the reference spec")
    p = sub.add_parser("measure")
    p.add_argument("--serial", required=True)
    p.add_argument("--reps", type=int, default=5)
    p.add_argument("--locks-dir", type=Path)
    p.add_argument("--artifacts", type=Path, default=DIST)
    p.add_argument("--workloads", default="w1,w2,w3,w4,w5,w6",
                   help="comma-separated workload ids to run")
    p.add_argument("--contestants",
                   help="comma-separated contestant names (default: all)")
    p.add_argument("--development", action="store_true",
                   help="permit a software GPU adapter and mark the run "
                   "development-only")
    p = sub.add_parser("report")
    p.add_argument("--out", type=Path, default=RESULTS)
    p = sub.add_parser("all")
    p.add_argument("--serial", required=True)
    p.add_argument("--reps", type=int, default=5)
    p.add_argument("--workloads", default="w1,w2,w3,w4,w5,w6")
    p.add_argument("--development", action="store_true",
                   help="permit a software GPU adapter and mark the run "
                   "development-only")
    # Physical-device mode (Mac mini + Pixel 9 Pro): prebuilt artifacts only,
    # per-contestant flock + thermal cooldown, no compilation.
    p = sub.add_parser("device")
    p.add_argument("--serial", required=True)
    p.add_argument("--reps", type=int, default=5)
    p.add_argument("--locks-dir", type=Path, default=Path("/tmp/device-locks"))
    p.add_argument("--artifacts", type=Path, required=True)
    p.add_argument("--out", type=Path, default=RESULTS)
    p.add_argument("--workloads", default="w1,w2,w3,w4,w5,w6",
                   help="comma-separated workload ids to run, e.g. W5,W6")
    p.add_argument("--contestants",
                   help="comma-separated contestant names (default: all)")
    p.add_argument("--development", action="store_true",
                   help="permit a software GPU adapter and mark the run "
                   "development-only")
    args = ap.parse_args()
    workloads = tuple(w.strip().lower()
                      for w in getattr(args, "workloads",
                                       "w1,w2,w3,w4,w5,w6").split(",")
                      if w.strip())

    man = tomllib.loads(MANIFEST.read_text())
    FULL_MAN.update(man)
    if args.cmd == "selftest":
        _self_test()
        return
    if getattr(args, "contestants", None):
        wanted = [c.strip() for c in args.contestants.split(",") if c.strip()]
        unknown = [c for c in wanted if c not in man["contestants"]]
        if unknown:
            raise SystemExit(f"unknown contestants {unknown}; the manifest "
                             f"has {list(man['contestants'])}")
        man["contestants"] = {c: man["contestants"][c] for c in wanted}

    out = getattr(args, "out", RESULTS)
    results = {}
    if args.cmd != "device" and out.exists():
        results = json.loads(out.read_text())
    results.setdefault("machine", {
        "system": platform.system(), "release": platform.release(),
        "machine": platform.machine(), "python": platform.python_version(),
        "cpu": platform.processor() or "n/a",
        "nproc": os.cpu_count(),
    })

    if args.cmd == "device":
        # device mode owns its own JSON (never merges the VM results file)
        results = {"machine": results["machine"], "mode": "device"}
        if out.exists():
            results = json.loads(out.read_text())
            results["mode"] = "device"
        cmd_collect(man, results, args.artifacts, args.serial)

        def save():
            out.write_text(json.dumps(results, indent=2))

        cmd_measure(man, args.serial, args.reps, args.locks_dir,
                    args.artifacts, results, workloads, save,
                    development=args.development)
    if args.cmd in ("build", "all"):
        cmd_build(man)
    if args.cmd in ("collect", "all"):
        cmd_collect(man, results, serial=getattr(args, "serial", None))
    if args.cmd in ("measure", "all"):
        serial = args.serial
        locks = getattr(args, "locks_dir", None)
        # every finished (rep, contestant) lands on disk at once: a
        # rerun of the same protocol resumes from this file
        def save_measure():
            out.write_text(json.dumps(results, indent=2))

        # package sizes belong to the same report: collect them from the
        # artifacts being measured, sized for the attached device
        cmd_collect(man, results, getattr(args, "artifacts", DIST), serial)
        cmd_measure(man, serial, args.reps, locks,
                    getattr(args, "artifacts", DIST), results, workloads,
                    save_measure, development=args.development)
    aggregate(results)
    out.write_text(json.dumps(results, indent=2))
    if args.cmd in ("report", "all", "measure", "device"):
        # the report sits beside the results it was generated from, so a
        # second run with its own --out never overwrites another run's report
        text = cmd_report(results)
        out.with_suffix(".md").write_text(text)
        print(text)


if __name__ == "__main__":
    main()
