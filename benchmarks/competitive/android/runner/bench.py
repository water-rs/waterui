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

import argparse
import fcntl
import hashlib
import json
import math
import os
import platform
import re
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


def _adb_bin(man: dict | None = None) -> str:
    """adb from the manifest-declared SDK, else PATH (device hosts that
    only run prebuilt artifacts and carry no toolchain pins)."""
    if man is not None:
        sdk = toolchain.require_dir(
            "android.sdk_root", man["toolchain"]["android"]["sdk_root"])
        return str(toolchain.require_file(
            "adb", sdk / "platform-tools" / "adb"))
    return shutil.which("adb") or str(
        Path.home() / "Android" / "Sdk" / "platform-tools" / "adb")


# manifest-resolved adb, set in main() when the declared SDK exists
_ADB_OVERRIDE: str | None = None


def adb(serial: str, *args: str, timeout: int = 60) -> str:
    return checked([_ADB_OVERRIDE or _adb_bin(), "-s", serial, *args],
                   timeout=timeout).strip()


_JAVA: str | None = None


def _java_bin(man: dict | None = None) -> str:
    """The manifest-declared JDK (build hosts). Device-only hosts that
    pass no manifest fall back to $JAVA_HOME then PATH."""
    global _JAVA
    if _JAVA:
        return _JAVA
    if man is not None:
        jh = toolchain.require_dir(
            "android.java_home", man["toolchain"]["android"]["java_home"])
        java = toolchain.require_file("java", jh / "bin" / "java")
        toolchain.require_version(
            "JDK", [str(java), "-version"],
            man["toolchain"]["android"]["jdk_version"])
        _JAVA = str(java)
        return _JAVA
    for cand in [os.environ.get("JAVA_HOME", "") + "/bin/java",
                 shutil.which("java") or ""]:
        if cand and Path(cand).exists() and \
                sh([cand, "-version"]).returncode == 0:
            _JAVA = cand
            return cand
    raise RuntimeError("no working JDK; the manifest declares "
                       "toolchain.android.java_home for build hosts")


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
    """Cool down between contestants: block until thermal status is nominal."""
    t0 = time.time()
    while time.time() - t0 < timeout_s:
        st = thermal_status(serial)
        if st in ("nominal", "n/a"):
            return st
        print(f"  thermal {st}, cooling…", flush=True)
        time.sleep(30)
    return thermal_status(serial)


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
    for abi, name in man["abi_apks"].items():
        checked([*base, "--arch", abi], cwd=d, env=be)
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
    if (d / "android").is_dir() and stamp.exists()                 and stamp.read_text().strip() == tag:
        return
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
    # harness-authored template change: the manifest's per-ABI APK matrix
    # needs splits on, which the stock template leaves off
    app_gradle = d / "android" / "app" / "build.gradle"
    app_gradle.write_text(app_gradle.read_text().replace(
        "enableSeparateBuildPerCPUArchitecture = false",
        "enableSeparateBuildPerCPUArchitecture = true"))
    stamp.write_text(tag + "\n")
    checked(["npm", "ci"], cwd=d, env=e)


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
    # RN autolinking creates per-module builds whose project repositories are
    # not covered by the Gradle init-script mirror. When the local maven proxy
    # (runner/maven_proxy.py) is reachable we point RN at it via
    # exclusiveEnterpriseRepository; that clears every project's repo list.
    proxy = None
    extra: list[str] = []
    if rn:
        # RN's signing config reads app/debug.keystore (module dir);
        # compose/views read rootProject.file at the project root.
        ensure_debug_keystore(proj / "app" / "debug.keystore")
        proxy = maybe_start_maven_proxy()
        if proxy:
            extra = [f"-PexclusiveEnterpriseRepository={proxy}"]
    else:
        ensure_debug_keystore(proj / "debug.keystore")
    ensure_gradle_wrapper(proj, gradle_version_for(man), e)
    try:
        checked(["./gradlew", ":app:assembleRelease", "--console=plain",
                 *extra], cwd=proj, env=e)
        checked(["./gradlew", ":app:bundleRelease", "-PabiSplits=false",
                 "--console=plain", *extra], cwd=proj, env=e)
    finally:
        if proxy:
            stop_maven_proxy()
    stage(man, proj / "app/build/outputs", dist_dir)


_PROXY_PROC = None
_PROXY_URL = "http://127.0.0.1:8765/m2"


def maybe_start_maven_proxy() -> str | None:
    """Return the proxy URL if the local maven mirror is/was reachable."""
    global _PROXY_PROC
    import urllib.request
    try:
        urllib.request.urlopen(_PROXY_URL + "/", timeout=1)
        return _PROXY_URL
    except Exception:
        pass
    script = Path(__file__).with_name("maven_proxy.py")
    if not script.exists():
        return None
    _PROXY_PROC = subprocess.Popen(
        [sys.executable, str(script)],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(20):
        try:
            urllib.request.urlopen(_PROXY_URL + "/", timeout=1)
            return _PROXY_URL
        except Exception:
            time.sleep(0.25)
    return None


def stop_maven_proxy():
    global _PROXY_PROC
    if _PROXY_PROC:
        _PROXY_PROC.terminate()
        _PROXY_PROC = None


def stage(man, outputs: Path, dist_dir: Path):
    for abi, name in man["abi_apks"].items():
        shutil.copy(outputs / "apk/release" / name, dist_dir / name)
    shutil.copy(outputs / "bundle/release" / man["aab"], dist_dir / man["aab"])


def cmd_build(man):
    e = env(man)
    builders = {"waterui": build_waterui, "flutter": build_flutter,
                "reactnative": build_gradle, "native": build_gradle}
    failed = []
    for name, c in man["contestants"].items():
        print(f"== build {name}", flush=True)
        ddir = DIST / name
        ddir.mkdir(parents=True, exist_ok=True)
        try:
            if c["kind"] == "reactnative":
                build_gradle(c, ddir, e, rn=True)
            else:
                builders[c["kind"]](c, ddir, e)
            print(f"   ok -> {ddir}", flush=True)
        except RuntimeError as ex:
            (ddir / "BUILD_ERROR.txt").write_text(str(ex))
            print(f"   FAILED — recorded to {ddir}/BUILD_ERROR.txt", flush=True)
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


def aab_download_size(aab: Path, serial: str | None,
                      man: dict | None = None) -> dict:
    jar = ensure_bundletool(man) if man is not None else BUNDLETOOL_JAR
    with tempfile.TemporaryDirectory() as td:
        apks = Path(td) / "out.apks"
        spec = Path(td) / "spec.json"
        if serial:
            # the attached device's own spec; a failure here is a broken
            # adb/bundletool setup, not a reason to size for another device
            checked([_java_bin(man), "-jar", str(jar), "get-device-spec",
                     "--adb", _adb_bin(man), "--device-id", serial,
                     "--output", str(spec)])
        else:
            # VM mode has no device: size for the documented reference spec
            spec.write_text(json.dumps(_default_spec()))
        checked([_java_bin(man), "-jar", str(jar), "build-apks",
                 "--bundle", str(aab), "--output", str(apks),
                 "--device-spec", str(spec)])
        out = checked([_java_bin(man), "-jar", str(jar), "get-size",
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


def launch(serial: str, pkg: str, activity: str, workload: str,
           kind: str, step: int | None = None) -> float | None:
    """Cold launch; returns reported time-to-first-frame (Displayed ms).

    `step` carries the capacity-workload level (W5 rect count / W6 row
    depth) — `--es step N` for every contestant and `waterui.env.BENCH_STEP`
    for WaterUI, whose scaffold forwards `waterui.env.*` extras into the
    process environment."""
    adb_shell(serial, f"am force-stop {pkg}")
    adb_shell(serial, "logcat -c")
    if kind == "waterui":
        extra = ["--es", "waterui.env.BENCH_WORKLOAD", workload]
        if step is not None:
            extra += ["--es", "waterui.env.BENCH_STEP", str(step)]
    else:
        extra = ["--es", "workload", workload]
        if step is not None:
            extra += ["--ei", "step", str(step)]
    out = adb_shell(
        serial,
        "am start -W -n " + pkg + "/" + activity + " " + " ".join(extra),
        timeout=60,
    )
    total = None
    m = re.search(r"TotalTime:\s*(\d+)", out)
    if m:
        total = float(m.group(1))
    # prefer logcat Displayed (launch -> first frame drawn); formats are
    # "+510ms" or "+1s40ms"
    if total is None:
        return None  # the launch never completed: no Displayed line follows
    # `am start -W` and the Displayed line come from the same launch event;
    # block on the line itself (the buffer was cleared before the launch)
    log = adb_shell(
        serial, "logcat -m 1 -s ActivityTaskManager:I -e 'Displayed "
        + pkg + "'", timeout=30)
    m = re.search(r"Displayed\s+\S+?:\s*\+?(?:(\d+)s)?(\d+)ms", log)
    if m:
        return float((int(m.group(1) or 0)) * 1000 + int(m.group(2)))
    return total


def meminfo(serial: str, pkg: str) -> dict:
    out = adb_shell(serial, f"dumpsys meminfo {pkg}")
    pss = rss = None
    m = re.search(r"TOTAL PSS:\s*([\d,]+)", out)
    if m:
        pss = int(m.group(1).replace(",", ""))
    m = re.search(r"TOTAL RSS:\s*([\d,]+)", out)
    if m:
        rss = int(m.group(1).replace(",", ""))
    if pss is None:
        m = re.search(r"^\s*TOTAL\s+([\d,]+)", out, re.M)
        if m:
            pss = int(m.group(1).replace(",", ""))
    return {"pss_kb": pss, "rss_kb": rss}


PERFETTO_CFG = """\
buffers { size_kb: 32768 fill_policy: RING_BUFFER }
data_sources { config { name: "android.surfaceflinger.frametimeline" } }
data_sources { config { name: "linux.process_stats" process_stats_config {
  scan_all_processes_on_start: true
  record_thread_names: true
} } }
duration_ms: %d
"""

# Capacity-workload config: FrameTimeline + process stats plus scheduler
# slices so CPU ms/frame on the app's UI + render threads can be attributed
# (W5/W6 only; the emulator path has no FrameTimeline).
PERFETTO_CAP_CFG = """\
buffers { size_kb: 65536 fill_policy: RING_BUFFER }
data_sources { config { name: "android.surfaceflinger.frametimeline" } }
data_sources { config { name: "linux.process_stats" process_stats_config {
  scan_all_processes_on_start: true
  record_thread_names: true
} } }
data_sources { config { name: "linux.ftrace" ftrace_config {
  ftrace_events: "sched/sched_switch"
  ftrace_events: "sched/sched_wakeup"
  ftrace_events: "task/task_newtask"
  ftrace_events: "task/task_rename"
} } }
duration_ms: %d
"""

PERFETTO_ATRACE_CFG = """\
buffers { size_kb: 32768 fill_policy: RING_BUFFER }
data_sources { config { name: "linux.ftrace" ftrace_config {
  atrace_categories: "sf" atrace_categories: "view"
  atrace_apps: "*" } } }
duration_ms: %d
"""

# Frame source per contestant, decided once per measure run by
# probe_frame_sources(). FrameTimeline is the issue-mandated source; a
# contestant whose presents land in no frame-timeline table falls back to
# SurfaceFlinger atrace events (`onFrameAvailable` per app surface), then to
# per-layer `dumpsys` latency rings — degraded alone, never dragging the
# other contestants down with it.
FRAME_SOURCES: dict[str, str] = {}


def frame_source_for(name: str) -> str:
    return FRAME_SOURCES.get(name, "atrace")


def _record_trace(serial: str, cfg: str, ms: int, drive) -> Path:
    """Record a trace whose window is exactly `drive()`.

    `--background-wait` returns once every data source has started and
    prints the tracing pid; when the drive program finishes, SIGTERM ends
    the session and perfetto reads the buffers back into the file, and the
    capture waits for that process to be gone and checks the file against
    the byte count perfetto logged. The config's duration is only a safety cap. No wall-clock
    padding: the window is the drive, the wait is the process exit."""
    del ms  # the window is the drive program; the config caps it
    remote = f"/data/misc/perfetto-traces/bench_{int(time.time()*1000)}.perfetto-trace"
    proc = subprocess.run(
        [_ADB_OVERRIDE or _adb_bin(), "-s", serial, "shell",
         f"perfetto --background-wait -c - --txt -o {remote}"],
        input=cfg, text=True, capture_output=True, timeout=40)
    pid = proc.stdout.strip().splitlines()[-1].strip() if proc.stdout.strip() else ""
    if proc.returncode != 0 or not pid.isdigit():
        raise RuntimeError(
            f"perfetto did not start: rc={proc.returncode} "
            f"out={proc.stdout.strip()!r} err={proc.stderr.strip()[-300:]!r}")
    try:
        drive()
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
    return local


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


def perfetto_capture(serial: str, ms: int, drive, pkg: str) -> dict:
    """Record FrameTimeline on-device while `drive()` runs, then analyze."""
    local = _record_trace(serial, PERFETTO_CFG % (ms + 30000), ms, drive)
    return analyze_trace(local, pkg)


def _pctl(xs, p):
    xs = sorted(xs)
    if not xs:
        return None
    k = (len(xs) - 1) * p
    f, c = math.floor(k), math.ceil(k)
    return xs[int(k)] if f == c else xs[f] + (xs[c] - xs[f]) * (k - f)


def _r(v, nd=3):
    return round(v, nd) if v is not None else None


def _clip_window(ts: list[int], cap_ms: int) -> list[int]:
    """Drop timestamps outside the capture window.

    SurfaceFlinger's latency ring can hold stale or future-dated entries
    (clock-domain wraps) that survive the >baseline filter; an interval
    longer than the drive window itself is impossible and is an artifact,
    not a stall."""
    if not ts:
        return ts
    cap_ns = (cap_ms + 1500) * 1e6  # slack for poll lag
    return [t for t in ts if t - ts[0] <= cap_ns]


class NoFramesError(RuntimeError):
    """A capture that ran cleanly but recorded zero frames for the app.

    Distinct from a capture error: on a live app that still owns layers a
    zero-frame window is a real measurement (the collapse point); on a
    dead app or missing surfaces it stays an abort."""


def analyze_atrace(path: Path, pkg: str, refresh: float | None,
                   cap_ms: int = 20000) -> dict:
    """Fallback analysis: count one frame per SurfaceFlinger frameTimelineInfo
    (or onFrameAvailable) atrace slice in the capture window — a global stream,
    so the workload under test is the only thing drawing. Report inter-frame
    interval percentiles, missed-vsync share and fps.

    Raises RuntimeError on empty/malformed traces — a capture miss aborts the
    run, it is never stored as a per-run error."""
    from perfetto.trace_processor import TraceProcessor
    with TraceProcessor(trace=str(path)) as tp:
        # SurfaceFlinger's FrameTimeline atrace writes one
        # frameTimelineInfo(frameNumber=…, vsyncId=…) slice per produced
        # frame — a global stream covering whichever app is on screen
        # during the drive window (uniform across renderers).
        rows = list(tp.query(
            "SELECT s.ts AS ts FROM slice s "
            "WHERE s.name GLOB 'frameTimelineInfo*' "
            "OR s.name GLOB 'onFrameAvailable -*' "
            "ORDER BY s.ts"))
    ts = _clip_window([r.ts for r in rows], cap_ms)
    if not ts:
        raise NoFramesError(f"no frame events in trace for {pkg}")
    ivals = [(b - a) / 1e6 for a, b in zip(ts, ts[1:]) if b > a]  # ms
    # percentiles over active intervals only (>=500 ms gaps are drive idles)
    active = [x for x in ivals if x < 500.0]
    interval = 1000.0 / (refresh or 60.0)
    # >1.5 vsync periods = a missed present; >=500 ms = an idle gap in the
    # drive sequence (app genuinely produced no frames), not a drop.
    janky = sum(1 for x in ivals if 1.5 * interval < x < 500.0)
    span_s = (ts[-1] - ts[0]) / 1e9
    return {
        "frames": len(ts),
        "frame_ms_p50": _r(_pctl(active, 0.50)),
        "frame_ms_p90": _r(_pctl(active, 0.90)),
        "frame_ms_p99": _r(_pctl(active, 0.99)),
        "dropped_pct": round(100.0 * janky / len(ivals), 2) if ivals else None,
        "fps": round(len(ivals) / span_s, 1) if span_s else None,
        "ivals_ms": [round(x, 3) for x in ivals],
    }


def atrace_capture(serial: str, ms: int, drive, pkg: str,
                   refresh: float | None) -> dict:
    local = _record_trace(serial, PERFETTO_ATRACE_CFG % (ms + 30000), ms, drive)
    return analyze_atrace(local, pkg, refresh, cap_ms=ms)


# --- tier 2: SurfaceFlinger per-layer latency -------------------------------
# `dumpsys SurfaceFlinger --latency <layer>` returns a 128-slot ring of
# (desired, actual-present, ready) ns timestamps per presented frame for one
# layer. Emulator SurfaceFlinger keeps it populated for every producer —
# including Flutter's SurfaceView pipeline that never posts ViewRootImpl
# frame markers. We poll the app's layers during the drive window and take
# the layer that produced the most frames.


def sf_latency_detail(serial: str, layer: str) -> tuple[list[int], int]:
    """(valid present timestamps, populated ring rows) for one layer.

    The ring's first line carries the vsync period, then 128 slot rows of
    (desired, actual-present, ready) ns. Rows are detected by content, not
    position — Android versions differ in column count and separators —
    and a token is a timestamp only when it parses as a number, so a drifted
    format or a cleared slot can never masquerade as an empty ring. Empty
    slots hold 0, cleared slots LONG_MAX: neither is a present."""
    out = adb_shell(
        serial, f"dumpsys SurfaceFlinger --latency '{layer}'")
    ts: list[int] = []
    rows = 0
    for line in out.splitlines():
        nums = [x for x in re.split(r"[\s,;|]+", line)
                if re.fullmatch(r"-?\d+(?:\.\d+)?", x or "")]
        if len(nums) < 2:
            continue  # period header, comments, empty ring rows
        # some builds prefix each slot with a small row index; real
        # columns are ns timestamps (>= 1e6). Drop a leading index so the
        # present column stays at position 1 either way.
        if len(nums) >= 4 and float(nums[0]) < 1e6:
            nums = nums[1:]
        rows += 1
        try:
            t = int(float(nums[1]))  # actual present time, ns
        except ValueError:
            continue
        if 0 < t < 2**62:
            ts.append(t)
    return ts, rows


def sf_latency(serial: str, layer: str) -> list[int]:
    return sf_latency_detail(serial, layer)[0]


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


# Layer names that are NOT the app's own content: window-manager transition
# leashes, splash screens, input sinks. They present frames while the app
# itself may still be starting, so the first-frame gate must ignore them.
_NONCONTENT_LAYER = re.compile(
    r"leash|ActivityRecordInputSink|Bounds for|Splash|splash|Snapshot")


def content_layers(serial: str, pkg: str) -> list[str]:
    return [l for l in sf_layers(serial, pkg)
            if not _NONCONTENT_LAYER.search(l)]


def gfxinfo_frames(serial: str, pkg: str) -> int | None:
    """dumpsys gfxinfo 'Total frames rendered' — a process-level present
    counter independent of SurfaceFlinger layer names, so it still sees
    SurfaceView/BLAST pipelines whose ring does not track presents."""
    try:
        out = adb_shell(serial, f"dumpsys gfxinfo {pkg}")
    except Exception:
        return None
    m = re.search(r"Total frames rendered:\s*(\d+)", out)
    return int(m.group(1)) if m else None


def wait_first_frame(serial: str, pkg: str, timeout_ms: int = 20000) -> None:
    """Block until the app has presented at least once since this launch.

    launch() force-stops the package first: the process and its
    SurfaceFlinger layers are fresh, so ANY populated content-layer ring —
    and any nonzero gfxinfo frame count — is a present produced by this
    launch. That is the gate condition, launch-relative rather than
    gate-relative: an app that draws before the gate's own baseline (every
    launch path already waits via `am start -W`, so on a healthy device the
    first frame has long presented when the gate runs) returns immediately,
    while an app that genuinely never draws burns the timeout. Engine
    startup latency varies wildly per contestant (Flutter's SurfaceView/
    BLAST pipeline can take seconds on a slow target); without this gate
    the capture window can close before the first present and a startup
    stall is misread as collapsed pacing."""
    t0 = time.time()
    while time.time() - t0 < timeout_ms / 1000:
        for l in content_layers(serial, pkg):
            try:
                if sf_latency(serial, l):
                    return  # ring populated since this launch = presented
            except Exception:
                continue
        gfx = gfxinfo_frames(serial, pkg)
        if gfx:
            return  # process-fresh counter >0 = presented since launch
        time.sleep(0.2)
    # no present within the timeout: fall through to the capture — a
    # zero-frame window records a collapsed step (live app with layers) or
    # aborts (dead process / no surfaces); a late-starting app is still
    # measured fairly. The inventory prints every queried name verbatim
    # with valid/ring counts so the diagnosis is the raw truth.
    inv = {}
    for l in sf_layers(serial, pkg):
        try:
            ts, rows = sf_latency_detail(serial, l)
        except Exception as ex:
            inv[l[:60]] = f"error:{ex}"
            continue
        inv[l[:60]] = f"{len(ts)} valid / {rows} rows"
        if rows and not ts:
            # populated ring, zero presents: surface the first raw slot so
            # an unrecognized column format is diagnosable from the log
            raw = adb_shell(
                serial, f"dumpsys SurfaceFlinger --latency '{l}'"
            ).splitlines()
            first = next(
                (x for x in raw[1:]
                 if len(re.split(r"[\s,;|]+", x.strip())) >= 2), "")
            inv[l[:60]] += f" raw[{first.strip()[:50]}]"
    print(f"   ! {pkg}: no present within {timeout_ms} ms of launch "
          f"(layers: {inv})", flush=True)


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


def sflinger_capture(serial: str, ms: int, drive, pkg: str,
                     refresh: float | None) -> dict:
    layers = sf_layers(serial, pkg)
    if not layers:
        raise RuntimeError(f"no SurfaceFlinger layer for {pkg}")
    # the ring keeps up to 128 PAST presents — snapshot each layer's newest
    # timestamp before driving and keep only frames presented after that
    base: dict[str, int] = {}
    for l in layers:
        try:
            b = sf_latency(serial, l)
            base[l] = max(b) if b else 0
        except Exception:
            base[l] = 0
    seen: dict[str, set] = {l: set() for l in layers}
    stop = [False]

    def poll():
        # layers churn (BLAST leashes, surface recreation on relaunch), so
        # re-enumerate each pass and keep every layer that ever matches
        while not stop[0]:
            for l in sf_layers(serial, pkg):
                if l not in seen:
                    # first sighting: base at the current ring head so only
                    # frames presented after this pass count
                    try:
                        b = sf_latency(serial, l)
                        base[l] = max(b) if b else 0
                    except Exception:
                        base[l] = 0
                    seen[l] = set()
                try:
                    seen[l].update(
                        t for t in sf_latency(serial, l) if t > base[l])
                except Exception:
                    pass
            time.sleep(0.25)

    import threading
    t = threading.Thread(target=poll, daemon=True)
    t.start()
    drive()
    time.sleep(0.5)
    stop[0] = True
    t.join(timeout=5)
    best = max(seen.values(), key=len)
    ts = _clip_window(sorted(best), ms)
    if not ts:
        # zero presents: collapse measurement only when the app is alive
        # and still owns layers; a dead process or vanished surfaces is a
        # capture failure and must abort the run
        if process_alive(serial, pkg) and sf_layers(serial, pkg):
            return zero_frame_result()
        raise RuntimeError(
            f"no frames on SF layers for {pkg} "
            f"({ {k: len(v) for k, v in seen.items()} })")
    ivals = [(b - a) / 1e6 for a, b in zip(ts, ts[1:]) if b > a]
    active = [x for x in ivals if x < 500.0]
    interval = 1000.0 / (refresh or 60.0)
    janky = sum(1 for x in ivals if 1.5 * interval < x < 500.0)
    span_s = (ts[-1] - ts[0]) / 1e9
    return {
        "frames": len(ts),
        "frame_ms_p50": _r(_pctl(active, 0.50)),
        "frame_ms_p90": _r(_pctl(active, 0.90)),
        "frame_ms_p99": _r(_pctl(active, 0.99)),
        "dropped_pct": round(100.0 * janky / len(ivals), 2) if ivals else None,
        "fps": round(len(ivals) / span_s, 1) if span_s else None,
        "ivals_ms": [round(x, 3) for x in ivals],
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


def frame_capture(serial: str, ms: int, drive, pkg: str,
                  refresh: float | None, src: str) -> dict:
    def once():
        wait_first_frame(serial, pkg)
        assert_foreground(serial, pkg)
        if src == "perfetto":
            st = perfetto_capture(serial, ms, drive, pkg)
        elif src == "sflinger":
            st = sflinger_capture(serial, ms, drive, pkg, refresh)
        else:
            st = atrace_capture(serial, ms, drive, pkg, refresh)
        assert_foreground(serial, pkg)
        return st

    assert_capture_ready(serial)
    try:
        return once()
    except RuntimeError:
        if src == "perfetto":
            raise
        # a capture miss (layer churn mid-drive etc.) gets one identical
        # retry for every contestant; a second miss aborts the run
        assert_capture_ready(serial)
        return once()


# The probe window, the floor below which a contestant is not yet animating
# (a quarter of the pinned refresh — the W5 probe step animates every vsync
# on any healthy contestant), and how many presents FrameTimeline must see
# relative to SurfaceFlinger's own per-layer ring for perfetto to count.
PROBE_WINDOW_MS = 3000
PROBE_ANIMATING_FRAC = 0.25
PROBE_AGREEMENT = 0.9
PROBE_ATTEMPTS = 5


def _presented_count(path: Path, pkg: str) -> int:
    return sum(1 for r in _frame_rows(path, pkg)
               if not (r[4] and "Dropped" in str(r[4])))


def probe_contestant(serial: str, name: str, pkg: str) -> str:
    """Pick the frame source that sees this contestant's presents.

    Perfetto records while the SurfaceFlinger latency capture runs, so both
    sources observe the same window of the same animation. SurfaceFlinger's
    per-layer ring sees every present of every layer the app owns; perfetto
    is chosen when FrameTimeline attributes at least PROBE_AGREEMENT of
    them to the contestant. A window in which SurfaceFlinger itself sees
    the app below the animating floor says nothing about either source (the
    engine is still starting), so the probe captures again instead of
    deciding on it."""
    floor = PROBE_ANIMATING_FRAC * (PINNED_REFRESH or 60.0) \
        * PROBE_WINDOW_MS / 1000
    for attempt in range(1, PROBE_ATTEMPTS + 1):
        wait_first_frame(serial, pkg)
        assert_foreground(serial, pkg)
        box: dict = {}

        def drive():
            box["sf"] = sflinger_capture(
                serial, PROBE_WINDOW_MS,
                lambda: time.sleep(PROBE_WINDOW_MS / 1000), pkg, None)

        local = _record_trace(serial, PERFETTO_CFG % (PROBE_WINDOW_MS + 30000),
                              PROBE_WINDOW_MS, drive)
        assert_foreground(serial, pkg)
        sf = box["sf"]["frames"] or 0
        pf = _presented_count(local, pkg)
        if sf < floor:
            print(f"   probe ({name}) attempt {attempt}: SurfaceFlinger saw "
                  f"{sf} presents (< {floor:.0f}) — not animating yet",
                  flush=True)
            continue
        src = "perfetto" if pf >= PROBE_AGREEMENT * sf else "sflinger"
        print(f"   probe ({name}): FrameTimeline {pf} / SurfaceFlinger {sf} "
              f"presents — {src}", flush=True)
        return src
    raise RuntimeError(
        f"{pkg} never reached {floor:.0f} presents in a "
        f"{PROBE_WINDOW_MS} ms probe window over {PROBE_ATTEMPTS} attempts")


def probe_frame_sources(man, serial: str, artifacts: Path) -> dict:
    """Decide the frame source for EVERY contestant independently, on an
    ANIMATING workload (the W5 probe step), by cross-checking FrameTimeline
    against SurfaceFlinger over the same window (probe_contestant). A
    contestant whose presents do not land in FrameTimeline — Flutter's
    SurfaceView stream carries no vsync id — measures through
    SurfaceFlinger alone; it never forces a different source on the rest."""
    global FRAME_SOURCES
    names = [n for n in man["contestants"]
             if not (artifacts / n / "BUILD_ERROR.txt").exists()]
    probe_step = man["workloads"]["w5"]["steps"][0]
    srcs: dict[str, str] = {}
    for n in names:
        c = man["contestants"][n]
        launch(serial, c["package"], c["activity"], "w5", c["kind"],
               step=probe_step)
        srcs[n] = probe_contestant(serial, n, c["package"])
    FRAME_SOURCES = srcs
    return srcs


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


def _frame_rows(path: Path, pkg: str):
    """Present rows for the contestant's layers across every frame-timeline
    table the trace emitted, as (ts, dur, jank_type, layer_name,
    present_type, table)."""
    from perfetto.trace_processor import TraceProcessor
    rows = []
    with TraceProcessor(trace=str(path)) as tp:
        for table in _timeline_tables(tp):
            cols = _table_columns(tp, table)
            if "layer_name" not in cols:
                continue
            sel = ["a.ts AS ts", "a.dur AS dur",
                   "a.jank_type AS jank_type"
                   if "jank_type" in cols else "NULL AS jank_type",
                   "a.layer_name AS layer_name",
                   "a.present_type AS present_type"
                   if "present_type" in cols else "NULL AS present_type"]
            join = ""
            if "upid" in cols:
                # process_stats is in both perfetto configs, so the upid
                # join resolves the process as a second attribution —
                # belt & suspenders next to the layer name
                join = " LEFT JOIN process p ON a.upid = p.upid"
                cond = (f"a.layer_name GLOB '*{pkg}*' OR "
                        f"p.name GLOB '*{pkg}*'")
            else:
                cond = f"a.layer_name GLOB '*{pkg}*'"
            for r in tp.query(
                    f"SELECT {', '.join(sel)} FROM {table} a{join} "
                    f"WHERE {cond} ORDER BY a.ts"):
                rows.append((r.ts, r.dur, r.jank_type, r.layer_name,
                             r.present_type, table))
    rows.sort(key=lambda r: r[0])
    return rows


def timeline_slice_counts(path: Path, pkg: str) -> tuple[int, int]:
    """(total, matching) frame-timeline rows — probe diagnostics.

    Runs the same package filter analyze_trace uses, so a probe fallback on
    a FrameTimeline-capable device shows whether zero slices came from the
    device emitting none or from the filter missing the contestant."""
    from perfetto.trace_processor import TraceProcessor
    total = matched = 0
    with TraceProcessor(trace=str(path)) as tp:
        for table in _timeline_tables(tp):
            cols = _table_columns(tp, table)
            if "layer_name" not in cols:
                continue
            total += int(next(iter(tp.query(
                f"SELECT COUNT(*) AS c FROM {table}"))).c)
            join = ""
            cond = f"layer_name GLOB '*{pkg}*'"
            if "upid" in cols:
                join = " a LEFT JOIN process p ON a.upid = p.upid"
                cond = (f"a.layer_name GLOB '*{pkg}*' OR "
                        f"p.name GLOB '*{pkg}*'")
            else:
                join = " a"
            matched += int(next(iter(tp.query(
                f"SELECT COUNT(*) AS c FROM {table}{join} "
                f"WHERE {cond}"))).c)
    return total, matched


def analyze_trace(path: Path, pkg: str) -> dict:
    """FrameTimeline slices for the app's layers: present interval
    percentiles and the jank share from the frame-timeline tables.

    Raises RuntimeError when the trace has no frame rows — a capture miss is
    a harness failure and must abort the run, not aggregate around an empty
    sample."""
    rows = _frame_rows(path, pkg)
    # dropped frames never presented — they are jank evidence, not presents,
    # and must not join the interval math as if a frame landed on screen
    presented = [r for r in rows
                 if not (r[4] and "Dropped" in str(r[4]))]
    if not presented:
        raise NoFramesError(
            f"no FrameTimeline slices for {pkg} in capture "
            f"(frame_source=perfetto)")
    # 1-3 rows is a real result (a collapsed step presents barely anything);
    # measure_capacity folds frames<4 into collapsed_at
    # present timestamp = slice end; present interval = gap between them
    pts = [r[0] + (r[1] or 0) for r in presented]
    ivals = [(b - a) / 1e6 for a, b in zip(pts, pts[1:]) if b > a]
    # percentiles over active intervals only — >=500 ms gaps are drive
    # idles (the fling program's settles), the same rule dropped_pct uses
    active = [x for x in ivals if x < 500.0]
    janky = [r for r in rows
             if str(r[2]) not in ("None", "null", "None.None", "")]
    span_s = (pts[-1] - pts[0]) / 1e9
    return {
        "frames": len(presented),
        "frame_ms_p50": _r(_pctl(active, 0.50)),
        "frame_ms_p90": _r(_pctl(active, 0.90)),
        "frame_ms_p99": _r(_pctl(active, 0.99)),
        "dropped_pct": round(100.0 * len(janky) / len(rows), 2)
        if rows else None,
        "fps": round(len(ivals) / span_s, 1) if span_s else None,
        "ivals_ms": [round(x, 3) for x in ivals],
    }


# Threads counted as "UI + render" for CPU ms/frame: the process main thread
# (thread name == process name), the platform RenderThread, HWUI worker
# threads, raster threads (Flutter/Skia) and the JIT compiler thread.
def cpu_ms_per_frame(path: Path, pkg: str, frames: int | None) -> dict:
    """Scheduler-slice CPU time of the app's UI+render threads / frame count.

    Returns {"cpu_ms_per_frame": float|None, "cpu_threads": {name: ms}}.
    Requires the sched ftrace events (PERFETTO_CAP_CFG); missing sched data
    raises — a trace that cannot attribute CPU is a harness failure, not a
    null data point.
    """
    if not frames:
        return {"cpu_ms_per_frame": None, "cpu_threads": {}}
    from perfetto.trace_processor import TraceProcessor
    with TraceProcessor(trace=str(path)) as tp:
        rows = list(tp.query(
            "SELECT th.name AS tname, p.name AS pname, "
            "SUM(ss.dur) AS cpu_ns FROM sched_slice ss "
            "JOIN thread th ON ss.utid = th.utid "
            "JOIN process p ON th.upid = p.upid "
            f"WHERE p.name GLOB '*{pkg}*' GROUP BY th.name"))
    if not rows:
        raise RuntimeError(
            f"no sched_slice rows for {pkg} in capture — PERFETTO_CAP_CFG "
            "needs linux.process_stats (scan_all_processes_on_start)")
    out = {"cpu_ms_per_frame": None, "cpu_threads": {}}
    cpu_ms = 0.0
    for r in rows:
        cpu = (r.cpu_ns or 0) / 1e6
        sel = (
            # main thread is named after the process (comm is 15-char
            # truncated, so also match the truncated prefix)
            r.tname == r.pname or r.pname.startswith(r.tname)
            or r.tname.startswith("RenderThread")
            or r.tname.startswith("hwuiTask")
            or r.tname == "1.ui"
            or "raster" in r.tname.lower()
            or r.tname.startswith("Jit"))
        out["cpu_threads"][r.tname] = round(cpu, 1)
        if sel:
            cpu_ms += cpu
    if cpu_ms > 0:
        out["cpu_ms_per_frame"] = round(cpu_ms / frames, 3)
    return out


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


def capacity_capture(serial: str, ms: int, drive, pkg: str,
                     refresh: float | None, src: str) -> dict:
    """Frame capture for one capacity step; adds CPU ms/frame on perfetto."""
    assert_capture_ready(serial)
    if drive is None:
        # undriven workloads present continuously from launch; driven ones
        # stay idle until the capture's own drive program produces frames
        wait_first_frame(serial, pkg)
    assert_foreground(serial, pkg)
    try:
        if src == "perfetto":
            local = _record_trace(
                serial, PERFETTO_CAP_CFG % (ms + 30000), ms, drive)
            st = analyze_trace(local, pkg)
            st.update(cpu_ms_per_frame(local, pkg, st.get("frames")))
            assert_foreground(serial, pkg)
            return st
        if src == "sflinger":
            st = sflinger_capture(serial, ms, drive, pkg, refresh)
        else:
            st = atrace_capture(serial, ms, drive, pkg, refresh)
        assert_foreground(serial, pkg)
    except NoFramesError:
        # live app, surfaces still present, zero presents in the window:
        # the collapse point is a measurement; a dead process or vanished
        # surfaces remains a capture failure that aborts the run
        if process_alive(serial, pkg) and sf_layers(serial, pkg):
            # a collapse is a measurement only while the app is on top
            assert_foreground(serial, pkg)
            st = zero_frame_result()
        else:
            raise
    st["cpu_ms_per_frame"] = None
    return st


def screen_dims(serial: str) -> tuple[int, int]:
    screen = re.match(r"(\d+)x(\d+)",
                      adb_shell(serial, "wm size").rsplit(":", 1)[-1].strip())
    return (int(screen.group(1)), int(screen.group(2))) if screen else (1080, 2400)


def measure_capacity(man, name: str, c: dict, serial: str, wl: str,
                     dims: tuple[int, int], refresh: float | None) -> dict:
    """Stepped capacity workload: one cold launch per step, settle then hold.

    Stops early when pacing collapses (fewer than collapse_frac of presents
    within two 60 Hz budgets). A launch or capture error aborts the run.
    """
    spec = man["workloads"][wl]
    settle = spec["settle_ms"] / 1000.0
    hold = spec["hold_ms"]
    collapse_frac = spec.get("collapse_frac", 0.5)
    pkg, activity, kind = c["package"], c["activity"], c["kind"]
    out: dict = {"steps": [], "collapsed_at": None, "crashed": None,
                 "not_responding": None}
    for n in spec["steps"]:
        print(f"    {wl} step {n}", flush=True)
        try:
            launch(serial, pkg, activity, wl, kind, step=n)
            time.sleep(settle)
            if wl == "w6":
                def drive():  # noqa: B023 — dims bound at call time
                    drive_workload(serial, man["fling"], dims)
            else:
                def drive():
                    time.sleep(hold / 1000.0)
            st = capacity_capture(serial, hold, drive, pkg, refresh,
                                  frame_source_for(name))
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


def drive_workload(serial: str, fling: dict, screen: tuple[int, int]):
    """The shared fling protocol (../WORKLOADS.md): OS-level `input swipe`
    outside the app — 8 down then 2 up, 75%→15% of the surface height,
    250 ms gesture, 350 ms pause — identical for every contestant."""
    sw, shp = screen
    x = int(sw * fling["margin_x_frac"])
    y0, y1 = int(shp * fling["start_y_frac"]), int(shp * fling["end_y_frac"])
    dur, pause = fling["duration_ms"], fling["pause_between_ms"] / 1000.0
    for _ in range(fling["down_swipes"]):
        adb_shell(serial, "input swipe %d %d %d %d %d" % (x, y0, x, y1, dur))
        time.sleep(pause)
    for _ in range(fling["up_swipes"]):
        adb_shell(serial, "input swipe %d %d %d %d %d" % (x, y1, x, y0, dur))
        time.sleep(pause)


ALL_WORKLOADS = ("w1", "w2", "w3", "w4", "w5", "w6")


def measure_rep(man, name: str, c: dict, serial: str,
                artifacts: Path, workloads=ALL_WORKLOADS) -> dict:
    pkg, activity, kind = c["package"], c["activity"], c["kind"]
    dims = screen_dims(serial)

    rep = {"refresh_hz": refresh_hz(serial),
           "thermal_state": thermal_status(serial)}
    for w in workloads:
        if w in ("w5", "w6"):
            # capacity workloads drive their own per-step launches
            rep[w] = {"capacity": measure_capacity(
                man, name, c, serial, w, dims, rep["refresh_hz"])}
            continue
        wr = {}
        wr["startup_ms"] = launch(serial, pkg, activity, w, kind)
        # steady-state memory: idle after launch
        time.sleep(4)
        wr["memory_steady"] = meminfo(serial, pkg)
        # peak memory sampled while the workload is driven
        samples = []

        def sample_loop(stop):
            while not stop[0]:
                s = meminfo(serial, pkg)
                if s["pss_kb"]:
                    samples.append(s["pss_kb"])
                time.sleep(0.5)

        import threading
        stop = [False]
        t = threading.Thread(target=sample_loop, args=(stop,), daemon=True)
        t.start()
        if w in ("w2", "w3", "w4"):
            ms = 12000 if w in ("w2", "w4") else 15000
            if w in ("w2", "w4"):
                def drive():  # noqa: B023 — dims bound at call time
                    drive_workload(serial, man["fling"], dims)
            else:
                # W3 animates by itself: the window is the hold
                def drive():  # noqa: B023 — ms bound at call time
                    time.sleep(ms / 1000.0)
            wr["frames"] = frame_capture(
                serial, ms, drive, pkg, rep["refresh_hz"],
                frame_source_for(name))
        else:
            time.sleep(5)
        stop[0] = True
        t.join(timeout=2)
        wr["memory_peak_kb"] = max(samples) if samples else None
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
    global FRAME_SOURCES
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

    # Panel must stay awake at the pinned refresh rate for the whole run —
    # including the frame-source probe, which needs a visible SurfaceFlinger
    # composition to emit slices. Prior settings are restored at exit even
    # when a capture aborts.
    signal.signal(signal.SIGTERM, _exit_on_signal)
    signal.signal(signal.SIGHUP, _exit_on_signal)
    # the device is this run's from the first setting it pins to the last
    # one it restores: preflight, install, probe and every rep happen under
    # one lock, so no other session's work can land between two of them
    lock = DeviceLock(locks_dir, serial) if locks_dir else _nullctx()
    with lock:
        _measure_locked(man, serial, reps, locks_dir, artifacts, results,
                        workloads, save, names, res, dev)


def _measure_locked(man, serial: str, reps: int, locks_dir: Path | None,
                    artifacts: Path, results: dict, workloads, save,
                    names: list[str], res: dict, dev: dict) -> None:
    global FRAME_SOURCES
    prev = device_preflight(serial)
    try:
        dev["vsync_hz"] = PINNED_REFRESH
        dev["vsync_hz_active"] = active_refresh(serial)
        abi = dev["abi"]
        if dev.get("frame_source"):
            # resumed run: keep the sources its saved reps were measured with
            FRAME_SOURCES = srcs = dev["frame_source"]
        else:
            srcs = probe_frame_sources(man, serial, artifacts)
            dev["frame_source"] = srcs
            save()
        model = dev.get("model", "this target")
        # limitations describe this run's probe result — regenerate, never
        # accumulate strings from earlier harness versions. Sources are
        # per-contestant: group the contestants by what the probe picked.
        lims: list[str] = []
        results["limitations"] = lims

        def _lim(text: str) -> None:
            if text not in lims:
                lims.append(text)

        perfetto_names = [n for n, s in srcs.items() if s == "perfetto"]
        fallback = {s: [n for n, x in srcs.items() if x == s]
                  for s in ("sflinger", "atrace")}
        if perfetto_names:
            _lim(
                f"{model}: frame metrics for {', '.join(perfetto_names)} "
                "use Perfetto FrameTimeline (actual and surface "
                "frame_timeline_slice over the contestant's layers), the "
                "issue-mandated source; CPU ms/frame from sched_slice on "
                "UI+render threads."
            )
        for fb, fb_names in fallback.items():
            if not fb_names:
                continue
            _lim(
                f"{model}: FrameTimeline attributed fewer than "
                f"{PROBE_AGREEMENT:.0%} of SurfaceFlinger's presents to "
                f"{', '.join(fb_names)} in the probe, so their frame "
                "metrics use the " + (
                    "SurfaceFlinger per-layer `--latency` fallback — "
                    "per-frame present timestamps of the app's busiest "
                    "layer"
                    if fb == "sflinger" else
                    "SurfaceFlinger ftrace `frameTimelineInfo` fallback — "
                    "one event per produced frame, global to the "
                    "compositor"
                ) + ". Reported: present-interval p50/p90/p99 over active "
                "intervals, missed-vsync share, fps."
            )
        if any(w in workloads for w in ("w5", "w6")) and (
                fallback["sflinger"] or fallback["atrace"]):
            _lim(
                "cpu_ms_per_frame comes from perfetto sched_slice "
                "records, which the fallback sources do not produce; "
                "it is null for the fallback contestants on this run."
            )

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
                while True:
                    since = adb_shell(serial, "date +%s.%3N").strip()
                    r = measure_rep(
                        man, name, c, serial, artifacts, workloads)
                    installs = package_installs_since(serial, since)
                    if not installs:
                        break
                    # the store updating apps kills and restarts their
                    # processes and loads CPU and storage under the capture:
                    # that rep measured the update, so it is taken again
                    print(f"rep {rep} {name}: packages installed during the "
                          f"rep ({', '.join(installs)}); measuring again",
                          flush=True)
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
    global _ADB_OVERRIDE
    try:
        _ADB_OVERRIDE = _adb_bin(man)
    except RuntimeError:
        pass  # device-only host without the declared SDK: PATH adb
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
