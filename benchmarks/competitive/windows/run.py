"""water-rs/waterui#1262 competitive benchmark — Windows runner.

One entry point: ``uv run run.py``. Builds every contestant, launches each
workload, drives it identically, and measures:

  * package size      — installed directory, uncompressed and zip-compressed
  * memory            — median (steady) and peak private working set of
                        the owned tree, sampled from NtQuerySystemInformation
                        every 100 ms inside the measurement window
  * frame rate        — per-frame submission timestamps from one ETW trace
                        per run (DXGI + DxgKrnl + Kernel-Process providers,
                        the same stream PresentMon consumes), parsed into
                        frame intervals → fps, p50/p90/p99, missed vsyncs
  * startup           — cold launch to first presented frame

Every metric is the median of >= 5 runs with min/max and all samples kept.
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
import ctypes
import csv
import io
import json
import os
import platform
import re
import shutil
import statistics
import struct
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
import traceback
import zipfile
try:
    # Windows-only ctypes surface; guarded so argparse/--help still works
    # on a non-Windows host.
    from ctypes import (
        WINFUNCTYPE,
        byref,
        c_int,
        c_long,
        c_ulong,
        c_void_p,
        create_string_buffer,
        sizeof,
        windll,
        wstring_at,
    )
except ImportError:
    windll = None
try:
    if platform.system() != "Windows":
        raise ImportError
    # maintained bindings for Job Object / CreateProcess / pipes.
    # subprocess.CREATE_SUSPENDED is not exported on CPython >= 3.7 and
    # the previous ctypes Job Object code truncated 64-bit HANDLEs, so
    # process ownership uses pywin32 throughout
    import win32api
    import win32con
    import win32event
    import win32file
    import win32job
    import win32pipe
    import win32process
    import win32security
except ImportError:
    win32api = win32con = win32event = win32file = None
    win32job = win32pipe = win32process = win32security = None
try:
    if platform.system() != "Windows":
        raise ImportError
    import comtypes
    from comtypes import COMMETHOD, GUID, HRESULT, IUnknown, POINTER
    from ctypes import wintypes
except ImportError:
    comtypes = COMMETHOD = GUID = HRESULT = IUnknown = POINTER = None
    wintypes = None
from datetime import datetime, timezone
from pathlib import Path

import psutil

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "lib"))
import toolchain  # benchmarks/competitive/lib/toolchain.py
import frame_stats as lib_frames  # benchmarks/competitive/lib/frame_stats.py

ROOT = Path(__file__).resolve().parent
# the shared cross-platform projects live under benchmarks/competitive/apps;
# only this platform's native contestant (winui3) lives in the leg dir
APPS = ROOT.parent / "apps"
WINUI3 = ROOT / "winui3"
RESULTS_DIR = ROOT / "results"
TRACE_DIR = ROOT / "out"

# ---------------------------------------------------------------------------
# Contestant table — everything launch-specific lives here.
# ---------------------------------------------------------------------------

def staged_dxc_dll(manifest: dict, packaged_dir: Path, dll: str) -> Path:
    """The DXC runtime DLL `water package` staged beside its artifact.

    The CLI owns which DXC a WaterUI build ships, so the harness takes
    the DLL from the packaged output, never from a second tool dir, and
    verifies its file version against the manifest pin."""
    path = toolchain.require_file(f"packaged {dll}", packaged_dir / dll)
    info = win32api.GetFileVersionInfo(str(path), "\\")
    ms, ls = info["FileVersionMS"], info["FileVersionLS"]
    version = f"{ms >> 16}.{ms & 0xFFFF}.{ls >> 16}.{ls & 0xFFFF}"
    if version != manifest["toolchain"]["dxc"]:
        raise RuntimeError(
            f"water package staged {dll} {version}, the manifest pins dxc "
            f"{manifest['toolchain']['dxc']}")
    return path

CONTESTANTS = {
    "waterui": {
        "title": "WaterUI (hydrolysis)",
        "exe": "bench_waterui-hydrolysis.exe",
        "exe_dir": APPS / "waterui" / "dist" / "bench_waterui",
        "env": {
            # hydrolysis logs its adapter choice at info level; scoped to
            # the gpu target so the log stays small.
            "RUST_LOG": "hydrolysis::gpu=info",
        },
        "adapter": "per-run: hydrolysis 'selected wgpu adapter' log line",
        # Required evidence — the adapter hydrolysis actually selected,
        # read from the owned process's own output.
        "adapter_from_log": True,
    },
    "flutter": {
        "title": "Flutter",
        "exe": "bench_flutter.exe",
        "exe_dir": APPS / "flutter" / "build" / "windows" / "x64" / "runner" / "Release",
        "env": {},
        # Required evidence — this process's GPU-engine usage attributed
        # to an adapter LUID, resolved through DXGI.
        "gpu_engine_evidence": True,
        "adapter": "per-run: owned-pid GPU-engine → DXGI adapter LUID",
    },
    "electron": {
        "title": "Electron",
        "exe": "bench-electron.exe",
        "exe_dir": APPS / "electron" / "dist" / "bench-electron-win32-x64",
        "env": {},
        # Required evidence — app.getGPUInfo('complete') reported by the
        # measured process itself (main.js emits BENCH_GPUINFO).
        "gpuinfo_log": True,
        "adapter": "per-run: Electron app.getGPUInfo from measured process",
    },
    "winui3": {
        "title": "WinUI 3",
        "exe": "winui3.exe",
        "exe_dir": WINUI3 / "dist" / "win-x64",
        "env": {},
        "gpu_engine_evidence": True,
        "adapter": "per-run: owned-pid GPU-engine → DXGI adapter LUID",
    },
}

WORKLOAD_NAMES = {
    "w1": "Hello — centred label + counter button",
    "w2": "Feed — lazy list of 10,000 rows, automated fling",
    "w3": "Motion — 200 independently animated rounded rectangles",
    "w4": "Text — scrolling screen of 50 mixed Latin/CJK/emoji paragraphs",
}

# ---------------------------------------------------------------------------
# Build steps
# ---------------------------------------------------------------------------


def sh(cmd: list[str], cwd: Path | None = None, env_extra: dict | None = None) -> None:
    env = os.environ.copy()
    if env_extra:
        env.update(env_extra)
    print(f"$ {' '.join(map(str, cmd))}", flush=True)
    proc = subprocess.run([str(c) for c in cmd], cwd=cwd, env=env)
    if proc.returncode != 0:
        raise RuntimeError(f"build step failed ({proc.returncode}): {cmd}")


def dotnet(manifest: dict) -> str:
    """Manifest-declared dotnet, version-verified before build."""
    t = manifest["toolchain"]
    root = toolchain.require_dir("toolchain.dotnet_root",
                                 t["dotnet_root"])
    exe = toolchain.require_file("dotnet", root / "dotnet.exe")
    toolchain.require_version(
        "dotnet SDK", [str(exe), "--version"], t["dotnet_sdk"])
    return str(exe)


def water_cli(manifest: dict) -> str:
    """The in-tree water CLI — compiled from this checkout's cli/ into the
    suite-shared cache; never a PATH/cargo-bin guess."""
    del manifest
    return str(toolchain.provision_water_cli())


def node22_bin(manifest: dict) -> Path:
    """Manifest-declared Node 22 runtime, version-verified."""
    t = manifest["toolchain"]
    root = toolchain.require_dir("toolchain.node_root", t["node_root"])
    node = toolchain.require_file("node", root / "node.exe")
    toolchain.require_version("node", [str(node), "--version"],
                              "v" + t["node"])
    return root


def flutter_bin(manifest: dict) -> str:
    """Manifest-declared Flutter SDK, version-verified."""
    t = manifest["toolchain"]
    root = toolchain.require_dir("toolchain.flutter_root",
                                 t["flutter_root"])
    bat = toolchain.require_file("flutter", root / "bin" / "flutter.bat")
    out = subprocess.run(["cmd", "/c", str(bat), "--version"],
                         capture_output=True, text=True)
    if out.returncode != 0 or f"Flutter {t['flutter']}" not in \
            out.stdout + out.stderr:
        raise RuntimeError(
            f"flutter at {bat} does not report version "
            f"{t['flutter']}: {(out.stdout + out.stderr)[:300]}")
    return str(bat)


def build_waterui(manifest: dict) -> None:
    app = APPS / "waterui"
    # `water fetch` downloads the project's declared fonts into the CLI font
    # cache; `water package` then stages backends/hydrolysis/resources/
    # (fonts + waterui_assets) and generates app-icon.ico itself. Neither
    # output is committed — both are the CLI's own build artifacts.
    sh([water_cli(manifest), "fetch"], cwd=app)
    sh(
        [water_cli(manifest), "package", "--platform", "windows",
         "--backend", "hydrolysis"],
        cwd=app,
    )
    dist = app / "dist" / "bench_waterui"
    dist.mkdir(parents=True, exist_ok=True)
    exe = next((app / "target" / "package").glob("*-hydrolysis*.exe"))
    # stage under a stable name — the packaged file name carries a per-
    # project hash suffix that changes when the lock set is regenerated
    shutil.copy2(exe, dist / "bench_waterui-hydrolysis.exe")
    res_src = (app / "backends" / "hydrolysis" / "dist" / "windows"
               / "release" / "resources")
    if res_src.exists():
        dst = dist / "resources"
        if dst.exists():
            shutil.rmtree(dst)
        shutil.copytree(res_src, dst)
    for dll in ("dxil.dll", "dxcompiler.dll"):
        # vello's shader pipeline hard-requires DXC beside the binary.
        shutil.copy2(staged_dxc_dll(manifest, exe.parent, dll), dist / dll)


def build_flutter(manifest: dict) -> None:
    flutter = flutter_bin(manifest)
    # the windows/ platform dir is generated, not committed — the pinned
    # SDK's `flutter create` produces it in a scratch dir so no committed
    # file (pubspec, .gitignore) is rewritten. `.bench-generator` records
    # the generator pin; a stale dir from a different SDK is regenerated.
    app = APPS / "flutter"
    tag = ("flutter create --platforms windows --project-name bench_flutter "
           "--org dev.bench --template app | flutter_ver="
           + manifest["toolchain"]["flutter"])
    stamp = app / "windows" / ".bench-generator"
    if not ((app / "windows").is_dir() and stamp.exists()
            and stamp.read_text().strip() == tag):
        shutil.rmtree(app / "windows", ignore_errors=True)
        with tempfile.TemporaryDirectory() as td:
            gen = Path(td) / "app"
            sh(["cmd", "/c", flutter, "create", "--platforms=windows",
                "--project-name", "bench_flutter", "--org", "dev.bench",
                "--template", "app", str(gen)])
            shutil.copytree(gen / "windows", app / "windows")
            stamp.write_text(tag + "\n")
    # CreateProcess can't execute .bat directly — go through cmd.
    sh(["cmd", "/c", flutter, "build", "windows", "--release"],
       cwd=app)


def build_electron(manifest: dict) -> None:
    app = APPS / "electron"
    nb = node22_bin(manifest)
    npm = nb / "npm.cmd"
    node = nb / "node.exe"
    env = {"PATH": str(nb) + ";" + os.environ.get("PATH", "")}
    sh([npm, "ci"], cwd=app, env_extra=env)
    sh(
        [
            node,
            app / "node_modules" / "@electron" / "packager" / "bin" / "electron-packager.mjs",
            ".",
            "bench-electron",
            "--platform=win32",
            "--arch=x64",
            "--out=dist",
            "--overwrite",
        ],
        cwd=app,
        env_extra=env,
    )


def build_winui3(manifest: dict) -> None:
    sh(
        [
            dotnet(manifest), "publish", "-c", "Release", "-r", "win-x64",
            "--self-contained", "-o", "dist/win-x64",
        ],
        cwd=WINUI3,
    )


def builders(manifest: dict) -> dict:
    return {
        "waterui": lambda: build_waterui(manifest),
        "flutter": lambda: build_flutter(manifest),
        "electron": lambda: build_electron(manifest),
        "winui3": lambda: build_winui3(manifest),
    }

# ---------------------------------------------------------------------------
# Process / window helpers (ctypes — no third-party dependency).
# ---------------------------------------------------------------------------

psapi = windll.psapi if windll else None
kernel32 = windll.kernel32 if windll else None
user32 = windll.user32 if windll else None
ntdll = windll.ntdll if windll else None


def minimize_other_windows() -> list[int]:
    """Minimize every visible, non-minimized top-level window with a title
    (shell chrome excluded) so the contestant launches onto a clean
    desktop — a window created under an occluder stays occluded (Chromium
    stops compositing). Returns the hwnds it changed; windows that were
    already minimized are left (and reported) untouched."""
    SW_MINIMIZE = 6
    changed: list[int] = []

    @WINFUNCTYPE(c_int, c_void_p, c_void_p)
    def cb(hwnd, _lp):
        if not user32.IsWindowVisible(hwnd) or user32.IsIconic(hwnd):
            return 1
        length = user32.GetWindowTextLengthW(hwnd)
        if not length:
            return 1
        buf = ctypes.create_unicode_buffer(length + 1)
        user32.GetWindowTextW(hwnd, buf, length + 1)
        if buf.value and "Program Manager" not in buf.value:
            user32.ShowWindow(hwnd, SW_MINIMIZE)
            changed.append(hwnd)
        return 1

    user32.EnumWindows(cb, 0)
    return changed


def restore_windows(hwnds: list[int]) -> None:
    """Restore only the windows this runner minimized — pre-existing
    minimized state is preserved by construction."""
    if not hwnds:
        return
    SW_RESTORE = 9
    for h in hwnds:
        try:
            if user32.IsWindow(h):
                user32.ShowWindow(h, SW_RESTORE)
        except Exception:
            pass


def pin_topmost(hwnd: int) -> None:
    """Raise the app window above everything so it is never occluded —
    Chromium and composition engines stop producing frames while occluded."""
    HWND_TOPMOST = -1
    SWP_NOMOVE_NOSIZE_SHOW = 0x0001 | 0x0002 | 0x0040
    user32.SetWindowPos(
        hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE_NOSIZE_SHOW
    )


SPI_GETFOREGROUNDLOCKTIMEOUT = 0x2000


def require_foreground_eligible() -> None:
    """SetForegroundWindow succeeds for a background process only while
    the foreground lock time-out is 0 — Windows otherwise refuses it
    whenever the runner is not the foreground process, which it is not
    once a contestant window has focus. Checked once before any cell; the
    runner never changes the setting and never fakes input to get around
    the lock."""
    timeout = c_ulong(0)
    if not user32.SystemParametersInfoW(
            SPI_GETFOREGROUNDLOCKTIMEOUT, 0, byref(timeout), 0):
        raise SystemExit(
            "cannot read the foreground lock time-out "
            f"(winerror {kernel32.GetLastError()})")
    if timeout.value != 0:
        raise SystemExit(
            "refusing to measure: every rep brings the contestant window "
            "to the foreground with SetForegroundWindow, which Windows "
            "refuses to a background process while the foreground lock "
            f"time-out is {timeout.value} ms. Precondition: set "
            "HKCU\\Control Panel\\Desktop\\ForegroundLockTimeout to 0 "
            "for the measuring user and sign in again.")


def bring_to_foreground(hwnd: int) -> None:
    """Make the app window the visible, topmost foreground window so it
    is never occluded (occluded/hidden apps stop compositing entirely —
    Chromium's RAF throttling). Uniform for every contestant. A failure
    leaves the app compositing at best unreliably — it fails the rep,
    it is not swallowed."""
    SW_RESTORE = 9
    kernel32.SetLastError(0)
    user32.ShowWindow(hwnd, SW_RESTORE)
    show_err = kernel32.GetLastError()
    if show_err:
        raise RuntimeError(
            f"ShowWindow({hwnd:#x}, SW_RESTORE) failed: "
            f"winerror {show_err}")
    for name, call in (("BringWindowToTop", user32.BringWindowToTop),
                       ("SetForegroundWindow", user32.SetForegroundWindow)):
        kernel32.SetLastError(0)
        if not call(hwnd):
            raise RuntimeError(
                f"{name}({hwnd:#x}) failed: "
                f"winerror {kernel32.GetLastError()}")
    kernel32.SetLastError(0)
    pin_topmost(hwnd)
    if not user32.IsWindow(hwnd) or kernel32.GetLastError():
        raise RuntimeError(
            f"pin_topmost({hwnd:#x}) failed: "
            f"winerror {kernel32.GetLastError()}")


def window_for_pids(pids: set[int]):
    """Largest visible top-level window owned by one of pids."""
    results: list[tuple[int, tuple, int]] = []

    @WINFUNCTYPE(c_int, c_void_p, c_void_p)
    def cb(hwnd, _lparam):
        pid = c_ulong(0)
        user32.GetWindowThreadProcessId(hwnd, byref(pid))
        if pid.value in pids and user32.IsWindowVisible(hwnd):
            rect = (c_int * 4)()
            user32.GetWindowRect(hwnd, rect)
            area = max(0, rect[2] - rect[0]) * max(0, rect[3] - rect[1])
            if area:
                results.append((hwnd, tuple(rect), area))
        return 1

    user32.EnumWindows(cb, 0)
    if not results:
        return None
    hwnd, rect, _ = max(results, key=lambda r: r[2])
    return hwnd, rect


WM_MOUSEWHEEL = 0x020A


SCROLL_WORKLOADS = ("w2", "w4")


def fling_window(hwnd: int, rect: tuple[int, int, int, int], seconds: float,
                 fling: dict) -> None:
    """The shared fling protocol (../WORKLOADS.md) as real wheel input:
    SendInput with the cursor parked over the window centre, routed exactly
    like physical wheel input so every framework's scroll handler sees it.
    One fling = `fling_detents` -120-unit detents spread over
    `fling_duration_ms`, then a `fling_pause_ms` pause; `fling_down`
    flings down (content scrolls up) then `fling_up` back — repeated to
    cover the measurement window."""
    MOUSEEVENTF_WHEEL = 0x0800
    WHEEL_DELTA = -120  # down (content scrolls up); negated for up

    class MOUSEINPUT(ctypes.Structure):
        _fields_ = [
            ("dx", c_long),
            ("dy", c_ulong),
            ("mouseData", c_ulong),
            ("dwFlags", c_ulong),
            ("time", c_ulong),
            ("dwExtraInfo", ctypes.POINTER(c_ulong)),
        ]

    class INPUT(ctypes.Structure):
        _fields_ = [("type", c_ulong), ("mi", MOUSEINPUT)]

    INPUT_MOUSE = 0
    cx = (rect[0] + rect[2]) // 2
    cy = (rect[1] + rect[3]) // 2
    user32.SetCursorPos(cx, cy)
    detent_s = fling["fling_duration_ms"] / 1000.0 / fling["fling_detents"]
    pause_s = fling["fling_pause_ms"] / 1000.0

    def one_fling(direction: int) -> None:
        mi = MOUSEINPUT(0, 0, (direction * WHEEL_DELTA) & 0xFFFFFFFF,
                        MOUSEEVENTF_WHEEL, 0, None)
        inp = INPUT(INPUT_MOUSE, mi)
        for _ in range(fling["fling_detents"]):
            user32.SendInput(1, byref(inp), sizeof(INPUT))
            time.sleep(detent_s)
        time.sleep(pause_s)

    end = time.monotonic() + seconds
    while time.monotonic() < end:
        for _ in range(fling["fling_down"]):
            one_fling(1)
        for _ in range(fling["fling_up"]):
            one_fling(-1)


# ---------------------------------------------------------------------------
# Owned launch — one Job Object per measured process tree.
# The root process is created SUSPENDED and assigned to the job before it
# can spawn a single descendant, so the job's own process-id list is the
# complete attribution set: memory accounting and termination only ever
# touch pids the job reports. JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE makes
# even an abandoned runner clean up its app.
# ---------------------------------------------------------------------------

class OwnedApp:
    """A launched contestant process owned by a Job Object.

    `pids()` is the ONLY attribution set used for memory accounting and
    termination — no exe-name matching, no global process scans.
    Implemented on pywin32 (win32job/win32process), not hand-rolled
    ctypes: subprocess.CREATE_SUSPENDED does not exist on CPython 3.12
    and untyped ctypes calls truncate 64-bit HANDLEs.
    """

    def __init__(self, c: dict, workload: str,
                 log_path: Path | None = None):
        if win32job is None:
            raise RuntimeError("OwnedApp requires a Windows host")
        self.log = None
        self.job = None
        self.hproc = None
        self.hthread = None
        self.pid: int | None = None
        self._rpipe = None
        self._reader = None
        self._drain_errors: list[str] = []
        env = os.environ.copy()
        env["BENCH_WORKLOAD"] = workload
        env.update(c["env"])
        try:
            self.log = (open(log_path, "w", encoding="utf-8",
                             errors="replace")
                        if log_path else None)
            job = win32job.CreateJobObject(
                None, f"bench1262-{os.getpid()}-{int(time.time() * 1e3)}")
            info = win32job.QueryInformationJobObject(
                job, win32job.JobObjectExtendedLimitInformation)
            info["BasicLimitInformation"]["LimitFlags"] = \
                win32job.JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            win32job.SetInformationJobObject(
                job, win32job.JobObjectExtendedLimitInformation, info)
            self.job = job
            # real stdout/stderr capture through a pipe: only the WRITE
            # end is inheritable — an inheritable read end would leak
            # into every descendant and keep the pipe open after the
            # root exits, stalling the reader thread
            sa = win32security.SECURITY_ATTRIBUTES()
            sa.bInheritHandle = True
            rpipe, wpipe = win32pipe.CreatePipe(sa, 0)
            win32api.SetHandleInformation(
                rpipe, win32con.HANDLE_FLAG_INHERIT, 0)
            self._rpipe = rpipe
            si = win32process.STARTUPINFO()
            si.dwFlags = win32con.STARTF_USESTDHANDLES
            si.hStdOutput = wpipe
            si.hStdError = wpipe
            try:
                cmdline = f'"{c["exe_dir"] / c["exe"]}"'
                if c.get("args"):
                    cmdline += " " + c["args"]
                hproc, hthread, pid, _tid = win32process.CreateProcess(
                    None, cmdline, None, None, True,
                    win32con.CREATE_SUSPENDED, env, str(c["exe_dir"]), si)
            finally:
                win32api.CloseHandle(wpipe)
            self.hproc, self.hthread, self.pid = hproc, hthread, pid
            self._reader = threading.Thread(
                target=self._drain, args=(rpipe,), daemon=True)
            self._reader.start()
            # assign before resume — no descendant can escape the job
            win32job.AssignProcessToJobObject(self.job, self.hproc)
            win32process.ResumeThread(self.hthread)
        except BaseException as orig:
            # The launched root must die even when it never entered the
            # job (assignment failure leaves it suspended) — terminate
            # the exact handle, never a name/pid scan — and every
            # acquired resource is released even when one cleanup
            # itself fails; original + cleanup errors are both kept.
            errors = []
            if self.hproc is not None:
                try:
                    win32api.TerminateProcess(self.hproc, 1)
                except Exception as e:
                    errors.append(f"TerminateProcess: {e}")
                try:
                    rc = win32event.WaitForSingleObject(self.hproc, 5000)
                    if rc == win32event.WAIT_TIMEOUT:
                        errors.append("root did not exit in 5s")
                except Exception as e:
                    errors.append(f"root wait: {e}")
            for handle, what in ((self.job, "job"), (self.hproc, "proc"),
                                 (self.hthread, "thread"),
                                 (self._rpipe, "pipe")):
                if handle is not None:
                    try:
                        win32api.CloseHandle(handle)
                    except Exception as e:
                        errors.append(f"CloseHandle {what}: {e}")
            self.job = self.hproc = self.hthread = self._rpipe = None
            if self.log is not None:
                try:
                    self.log.close()
                except Exception as e:
                    errors.append(f"log close: {e}")
                self.log = None
            if errors:
                raise RuntimeError(
                    f"launch failed ({orig}); cleanup: "
                    + "; ".join(errors)) from orig
            raise

    def _drain(self, rpipe) -> None:
        import codecs
        dec = codecs.getincrementaldecoder("utf-8")(errors="replace")
        try:
            while True:
                _hr, data = win32file.ReadFile(rpipe, 65536)
                if not data:
                    break
                if self.log is not None:
                    self.log.write(dec.decode(data))
                    self.log.flush()
            if self.log is not None:
                self.log.write(dec.decode(b"", final=True))
                self.log.flush()
        except Exception as e:
            # ERROR_BROKEN_PIPE means the write end closed — it is end
            # of stream, not a drain failure. Everything else must reach
            # the record: the captured output is an evidence channel.
            if getattr(e, "winerror", None) != 109:
                self._drain_errors.append(f"{type(e).__name__}: {e}")
        finally:
            try:
                win32api.CloseHandle(rpipe)
            except Exception as e:
                self._drain_errors.append(f"pipe close: {e}")

    def pids(self) -> set[int]:
        """Pids the job accounts for — exactly the owned tree. pywin32
        sizes the ProcessIdList buffer itself."""
        return set(win32job.QueryInformationJobObject(
            self.job, win32job.JobObjectBasicProcessIdList))

    def create_filetime(self) -> int:
        """Root process creation as a FILETIME (100ns units since
        1601-01-01 UTC) — the same clock the ETW dump's clock column
        carries, so ETW timestamps subtract directly. pywin32 returns
        CreationTime as a PyTime (datetime); converting it, not int().
        """
        ct = win32process.GetProcessTimes(self.hproc)["CreationTime"]
        ts = ct.timestamp() if hasattr(ct, "timestamp") else float(ct)
        return int((ts + 11644473600) * 10_000_000)

    def terminate(self) -> None:
        """Kill the whole owned tree, then release every resource —
        each step runs even when an earlier one fails."""
        errors = []
        if self.job is not None:
            try:
                win32job.TerminateJobObject(self.job, 1)
            except Exception as e:
                errors.append(f"TerminateJobObject: {e}")
            try:
                win32api.CloseHandle(self.job)
            except Exception as e:
                errors.append(f"CloseHandle job: {e}")
            self.job = None
        if self.hproc is not None:
            try:
                rc = win32event.WaitForSingleObject(self.hproc, 5000)
                if rc == win32event.WAIT_TIMEOUT:
                    errors.append("process did not exit in 5s")
            except Exception as e:
                errors.append(f"proc wait: {e}")
            try:
                win32api.CloseHandle(self.hproc)
            except Exception as e:
                errors.append(f"CloseHandle proc: {e}")
            self.hproc = None
        if self.hthread is not None:
            try:
                win32api.CloseHandle(self.hthread)
            except Exception as e:
                errors.append(f"CloseHandle thread: {e}")
            self.hthread = None
        if self._reader is not None:
            self._reader.join(timeout=5)
            if self._reader.is_alive():
                errors.append("stdout reader did not exit in 5s")
            self._reader = None
        errors.extend(self._drain_errors)
        self._drain_errors = []
        if self.log is not None:
            try:
                self.log.close()
            except Exception as e:
                errors.append(f"log close: {e}")
            self.log = None
        if errors:
            raise RuntimeError("OwnedApp cleanup: " + "; ".join(errors))


# ---------------------------------------------------------------------------
# Memory sampling — NtQuerySystemInformation(SystemProcessInformation).
# Offsets below are the documented 64-bit layout (VM_COUNTERS embedded).
# ---------------------------------------------------------------------------

SPI_NEXT = 0x00
SPI_WS_PRIVATE = 0x08
SPI_IMAGE_NAME = 0x38  # UNICODE_STRING: u16 len, u16 max, pad, ptr@+8
SPI_PID = 0x50
SPI_PEAK_WS = 0x88
SPI_WS = 0x90
SPI_PRIVATE_BYTES = 0xC8

def filetime_now() -> int:
    """Current time as FILETIME ticks (100ns since 1601-01-01 UTC) —
    the clock ETW rows, GetProcessTimes and the memory sampler share.

    GetSystemTimePreciseAsFileTime, the clock those sources stamp with:
    `time.time()` reads the coarse system tick on CPython < 3.13 and
    trails it by up to a tick, which would schedule the drive late
    against the first-present anchor."""
    ft = ctypes.c_ulonglong()
    kernel32.GetSystemTimePreciseAsFileTime(ctypes.byref(ft))
    return ft.value


def process_memory_snapshot(
        buffer_bytes: int = 4 * 1024 * 1024) -> dict[int, dict]:
    """SystemProcessInformation for every live process, keyed by pid.

    The buffer is sized from the ReturnLength the first call reports —
    STATUS_BUFFER_TOO_SMALL (0xC0000023) is retried once at the reported
    size plus headroom; every other NTSTATUS failure raises instead of
    silently yielding an empty snapshot (a zero-sample rep used to count
    as a success).
    """
    if ntdll is None:
        raise RuntimeError("process_memory_snapshot requires Windows")
    size = buffer_bytes
    for _attempt in range(2):
        buf = create_string_buffer(size)
        ret = c_ulong(0)
        status = ntdll.NtQuerySystemInformation(
            5, buf, size, byref(ret))
        if status == 0:
            break
        if (status & 0xFFFFFFFF) in (0xC0000023, 0x80000005) \
                and ret.value > size:
            size = ret.value + 256 * 1024
            continue
        raise OSError(
            f"NtQuerySystemInformation failed: NTSTATUS "
            f"0x{status & 0xFFFFFFFF:08X}")
    else:
        raise OSError(
            "NtQuerySystemInformation still reports "
            "STATUS_BUFFER_TOO_SMALL after resizing")
    out = {}
    off = 0
    while True:
        nxt = struct.unpack_from("<I", buf, off + SPI_NEXT)[0]
        pid = struct.unpack_from("<Q", buf, off + SPI_PID)[0]
        ilen = struct.unpack_from("<H", buf, off + SPI_IMAGE_NAME)[0]
        iptr = struct.unpack_from("<Q", buf, off + SPI_IMAGE_NAME + 8)[0]
        name = ""
        if ilen and iptr:
            try:
                name = wstring_at(iptr, ilen // 2)
            except OSError:
                name = ""
        out[pid] = {
            "name": name,
            "ws_private": struct.unpack_from("<q", buf, off + SPI_WS_PRIVATE)[0],
            "peak_ws": struct.unpack_from("<q", buf, off + SPI_PEAK_WS)[0],
            "ws": struct.unpack_from("<q", buf, off + SPI_WS)[0],
            "private_bytes": struct.unpack_from("<q", buf, off + SPI_PRIVATE_BYTES)[0],
        }
        if nxt == 0:
            break
        off += nxt
        if off + 0xD0 > size:
            raise OSError("SystemProcessInformation walk overran buffer")
    return out


class MemorySampler(threading.Thread):
    """Samples the owned process tree every `interval` seconds.

    `tree_pids` is a callable returning the pids the measured process
    tree currently owns (the Job Object's process-id list on Windows).
    Per sample: FILETIME, and the sums of private working set and of
    private bytes across exactly those pids.
    """

    def __init__(self, tree_pids, interval: float):
        super().__init__(daemon=True)
        self.tree_pids = tree_pids
        self.interval = interval
        self._done = threading.Event()
        self.samples: list[tuple[int, int, int]] = []
        self.max_processes = 0
        # a native query failing inside the thread lands here and must
        # reach the attempt record — never silently truncate the sample
        self.error: Exception | None = None

    def stop(self) -> None:
        self._done.set()

    def run(self) -> None:
        try:
            while not self._done.is_set():
                snap = process_memory_snapshot()
                owned = self.tree_pids()
                mine = {p: m for p, m in snap.items() if p in owned}
                if mine:
                    # FILETIME ticks — the same clock the ETW dump's
                    # clock column carries, so samples trim to the
                    # measurement window (first present + warmup)
                    self.samples.append(
                        (filetime_now(),
                         sum(m["ws_private"] for m in mine.values()),
                         sum(m["private_bytes"] for m in mine.values())))
                    self.max_processes = max(self.max_processes, len(mine))
                self._done.wait(self.interval)
        except Exception as e:
            self.error = e

    def summarise(self, window: tuple[int, int]) -> dict:
        """Steady (median) and peak (max) of the tree's private working
        set, and its peak private bytes, over the samples inside `window`
        = [first present + warmup, +capture] in FILETIME ticks — startup
        and the processes' lifetime peak counters never enter."""
        in_window = [(ws, pb) for t, ws, pb in self.samples
                     if window[0] <= t <= window[1]]
        ws_mb = [ws / 1e6 for ws, _ in in_window]
        return {
            "steady_private_ws_mb": statistics.median(ws_mb)
            if ws_mb else None,
            "peak_private_ws_mb": max(ws_mb) if ws_mb else None,
            "private_bytes_mb": max(pb for _, pb in in_window) / 1e6
            if in_window else None,
            "process_count": self.max_processes,
            "samples_taken": len(in_window),
            "samples_total": len(self.samples),
        }


# ---------------------------------------------------------------------------
# ETW capture + parse (uniform frame-timing source for every contestant).
# ---------------------------------------------------------------------------

DXGI = "Microsoft-Windows-DXGI"
DXGKRNL = "Microsoft-Windows-DxgKrnl"
KPROC = "Microsoft-Windows-Kernel-Process"


def write_providers_file(path: Path) -> None:
    path.write_text(
        # DXGI: all keywords — composition-path presents carry keyword 0.
        "Microsoft-Windows-DXGI\t0xffffffffffffffff\t0xff\n"
        # DxgKrnl keywords 0x2 (per-frame present events) | 0x20 (CDD driver
        # events, e.g. DxgkCddDrvBitBlt — Chromium's frame path on WARP).
        "Microsoft-Windows-DxgKrnl\t0x22\t0xff\n"
        # pid bookkeeping for process-create timestamps.
        "Microsoft-Windows-Kernel-Process\t0x10\t0x4\n"
    )


def start_trace(name: str, etl: Path, providers_file: Path) -> None:
    # clear a same-named session left over from a crashed run, then create
    subprocess.run(
        ["logman", "delete", name, "-ets"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    if etl.exists():
        etl.unlink()
    out = subprocess.run(
        [
            "logman", "create", "trace", name, "-ets",
            "-o", str(etl), "-pf", str(providers_file),
        ],
        capture_output=True,
        text=True,
    )
    if out.returncode != 0:
        raise RuntimeError(f"logman create failed: {out.stdout} {out.stderr}")


def stop_trace(name: str) -> None:
    subprocess.run(
        ["logman", "stop", name, "-ets"],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def etl_to_csv(etl: Path, csv_path: Path) -> None:
    subprocess.run(
        ["tracerpt", str(etl), "-o", str(csv_path), "-of", "CSV", "-y"],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


# ---------------------------------------------------------------------------
# Real-time first-present consumer — the event the drive is scheduled on.
# A second, real-time-only ETW session over the same DXGI provider the
# file trace records; ctypes over the documented StartTraceW /
# EnableTraceEx2 / OpenTraceW / ProcessTrace / ControlTraceW API. All
# structures use fixed-width fields (the documented x64 layouts), so their
# sizes are checked by the CPU self-test on any host.
# ---------------------------------------------------------------------------

from ctypes import Structure as _S, c_int64 as _i64, c_uint8 as _u8, \
    c_uint16 as _u16, c_uint32 as _u32, c_uint64 as _u64, c_void_p as _ptr


class _EtwGuid(_S):
    _fields_ = [("Data1", _u32), ("Data2", _u16), ("Data3", _u16),
                ("Data4", _u8 * 8)]

    @classmethod
    def parse(cls, text: str) -> "_EtwGuid":
        h = text.strip("{}").replace("-", "")
        g = cls(int(h[0:8], 16), int(h[8:12], 16), int(h[12:16], 16))
        for i in range(8):
            g.Data4[i] = int(h[16 + 2 * i:18 + 2 * i], 16)
        return g

    def key(self) -> tuple:
        return (self.Data1, self.Data2, self.Data3, bytes(self.Data4))


class _WnodeHeader(_S):
    _fields_ = [("BufferSize", _u32), ("ProviderId", _u32),
                ("HistoricalContext", _u64), ("TimeStamp", _i64),
                ("Guid", _EtwGuid), ("ClientContext", _u32),
                ("Flags", _u32)]


class _EventTraceProperties(_S):
    _fields_ = [("Wnode", _WnodeHeader), ("BufferSize", _u32),
                ("MinimumBuffers", _u32), ("MaximumBuffers", _u32),
                ("MaximumFileSize", _u32), ("LogFileMode", _u32),
                ("FlushTimer", _u32), ("EnableFlags", _u32),
                ("AgeLimit", _u32), ("NumberOfBuffers", _u32),
                ("FreeBuffers", _u32), ("EventsLost", _u32),
                ("BuffersWritten", _u32), ("LogBuffersLost", _u32),
                ("RealTimeBuffersLost", _u32), ("LoggerThreadId", _ptr),
                ("LogFileNameOffset", _u32), ("LoggerNameOffset", _u32)]


class _EventTraceHeader(_S):
    _fields_ = [("Size", _u16), ("FieldTypeFlags", _u16),
                ("Version", _u32), ("ThreadId", _u32), ("ProcessId", _u32),
                ("TimeStamp", _i64), ("Guid", _EtwGuid),
                ("ProcessorTime", _u64)]


class _EventTrace(_S):
    _fields_ = [("Header", _EventTraceHeader), ("InstanceId", _u32),
                ("ParentInstanceId", _u32), ("ParentGuid", _EtwGuid),
                ("MofData", _ptr), ("MofLength", _u32),
                ("ClientContext", _u32)]


class _SystemTime(_S):
    _fields_ = [(n, _u16) for n in ("wYear", "wMonth", "wDayOfWeek", "wDay",
                                     "wHour", "wMinute", "wSecond",
                                     "wMilliseconds")]


class _TimeZoneInformation(_S):
    _fields_ = [("Bias", _u32), ("StandardName", _u16 * 32),
                ("StandardDate", _SystemTime), ("StandardBias", _u32),
                ("DaylightName", _u16 * 32), ("DaylightDate", _SystemTime),
                ("DaylightBias", _u32)]


class _TraceLogfileHeader(_S):
    _fields_ = [("BufferSize", _u32), ("Version", _u32),
                ("ProviderVersion", _u32), ("NumberOfProcessors", _u32),
                ("EndTime", _i64), ("TimerResolution", _u32),
                ("MaximumFileSize", _u32), ("LogFileMode", _u32),
                ("BuffersWritten", _u32), ("LogInstanceGuid", _EtwGuid),
                ("LoggerName", _ptr), ("LogFileName", _ptr),
                ("TimeZone", _TimeZoneInformation), ("BootTime", _i64),
                ("PerfFreq", _i64), ("StartTime", _i64),
                ("ReservedFlags", _u32), ("BuffersLost", _u32)]


class _EventDescriptor(_S):
    _fields_ = [("Id", _u16), ("Version", _u8), ("Channel", _u8),
                ("Level", _u8), ("Opcode", _u8), ("Task", _u16),
                ("Keyword", _u64)]


class _EventHeader(_S):
    _fields_ = [("Size", _u16), ("HeaderType", _u16), ("Flags", _u16),
                ("EventProperty", _u16), ("ThreadId", _u32),
                ("ProcessId", _u32), ("TimeStamp", _i64),
                ("ProviderId", _EtwGuid),
                ("EventDescriptor", _EventDescriptor),
                ("ProcessorTime", _u64), ("ActivityId", _EtwGuid)]


class _EventRecord(_S):
    _fields_ = [("EventHeader", _EventHeader), ("BufferContext", _u32),
                ("ExtendedDataCount", _u16), ("UserDataLength", _u16),
                ("ExtendedData", _ptr), ("UserData", _ptr),
                ("UserContext", _ptr)]


class _EventTraceLogfileW(_S):
    _fields_ = [("LogFileName", _ptr), ("LoggerName", ctypes.c_void_p),
                ("CurrentTime", _i64), ("BuffersRead", _u32),
                ("ProcessTraceMode", _u32), ("CurrentEvent", _EventTrace),
                ("LogfileHeader", _TraceLogfileHeader),
                ("BufferCallback", _ptr), ("BufferSize", _u32),
                ("Filled", _u32), ("EventsLost", _u32),
                ("EventRecordCallback", _ptr), ("IsKernelTrace", _u32),
                ("Context", _ptr)]


# documented x64 sizes (evntrace.h / evntcons.h)
ETW_STRUCT_SIZES = {_WnodeHeader: 48, _EventTraceProperties: 120,
                    _EventTrace: 88, _TraceLogfileHeader: 280,
                    _EventHeader: 80, _EventRecord: 112,
                    _EventTraceLogfileW: 448}

DXGI_PROVIDER_GUID = "{CA11C036-0102-4A2D-A6AD-F03CFED5D3C9}"  # Microsoft-Windows-DXGI
WIN_START_OPCODE = 1
# the DXGI present events the file trace's frame sources A and B are made
# of: event id -> source letter (parse_frames)
DXGI_PRESENT_EVENTS = {42: "A", 144: "B"}
WNODE_FLAG_TRACED_GUID = 0x00020000
EVENT_TRACE_REAL_TIME_MODE = 0x00000100
EVENT_TRACE_USE_MS_FLUSH_TIMER = 0x00000010
PROCESS_TRACE_MODE_REAL_TIME = 0x00000100
PROCESS_TRACE_MODE_EVENT_RECORD = 0x10000000
EVENT_CONTROL_CODE_ENABLE_PROVIDER = 1
EVENT_TRACE_CONTROL_STOP = 1
INVALID_PROCESSTRACE_HANDLE = 0xFFFFFFFFFFFFFFFF
# real-time buffers are delivered at least this often, so the drive is
# scheduled within milliseconds of the first present, not a 1 s flush
RT_FLUSH_MS = 10


def first_present_of(provider: tuple, event_id: int, opcode: int, pid: int,
                     owned) -> str | None:
    """The frame source letter when one real-time event is an owned DXGI
    present Start, else None. `owned(pid)` answers ownership."""
    if provider != _EtwGuid.parse(DXGI_PROVIDER_GUID).key() \
            or opcode != WIN_START_OPCODE \
            or event_id not in DXGI_PRESENT_EVENTS:
        return None
    return DXGI_PRESENT_EVENTS[event_id] if owned(pid) else None


class FirstPresentConsumer:
    """Real-time consumer of the DXGI present events of the owned process
    tree: `wait()` returns (FILETIME, source) of the tree's first present
    — the event the measurement window anchors on, delivered within
    RT_FLUSH_MS. Timestamps are system time (FILETIME), the clock of the
    file trace's dump. The session is stopped as soon as the first
    present is in, so it adds no load during the capture."""

    def __init__(self, name: str, tree_pids):
        self.name = name
        self.tree_pids = tree_pids
        self._known: set[int] = set()
        self.first: tuple[int, str] | None = None
        self._got = threading.Event()
        self._advapi = windll.advapi32
        self._advapi.OpenTraceW.restype = _u64
        self._session = _u64(0)
        self._props_buf = None
        self._consumer = None
        self._thread = None
        self._error: list[str] = []
        self._start_session()

    def _props(self):
        name_bytes = (len(self.name) + 1) * 2
        size = ctypes.sizeof(_EventTraceProperties) + name_bytes
        buf = ctypes.create_string_buffer(size)
        props = _EventTraceProperties.from_buffer(buf)
        props.Wnode.BufferSize = size
        props.Wnode.Flags = WNODE_FLAG_TRACED_GUID
        props.Wnode.ClientContext = 1  # QPC
        props.LogFileMode = (EVENT_TRACE_REAL_TIME_MODE
                             | EVENT_TRACE_USE_MS_FLUSH_TIMER)
        props.FlushTimer = RT_FLUSH_MS
        props.LoggerNameOffset = ctypes.sizeof(_EventTraceProperties)
        return buf, props

    def _start_session(self) -> None:
        self._props_buf, props = self._props()
        rc = self._advapi.StartTraceW(byref(self._session),
                                      ctypes.c_wchar_p(self.name),
                                      byref(props))
        if rc != 0:
            raise RuntimeError(f"StartTraceW({self.name}) failed: {rc}")
        try:
            guid = _EtwGuid.parse(DXGI_PROVIDER_GUID)
            rc = self._advapi.EnableTraceEx2(
                self._session, byref(guid),
                _u32(EVENT_CONTROL_CODE_ENABLE_PROVIDER), _u8(0xFF),
                _u64(0xFFFFFFFFFFFFFFFF), _u64(0), _u32(0), None)
            if rc != 0:
                raise RuntimeError(f"EnableTraceEx2(DXGI) failed: {rc}")
            self._open_consumer()
        except BaseException:
            self.stop()
            raise

    def _open_consumer(self) -> None:
        cb_type = WINFUNCTYPE(None, ctypes.POINTER(_EventRecord))
        self._cb = cb_type(self._on_event)
        self._logname = ctypes.create_unicode_buffer(self.name)
        lf = _EventTraceLogfileW()
        lf.LoggerName = ctypes.cast(self._logname, _ptr)
        lf.ProcessTraceMode = (PROCESS_TRACE_MODE_REAL_TIME
                               | PROCESS_TRACE_MODE_EVENT_RECORD)
        lf.EventRecordCallback = ctypes.cast(self._cb, _ptr)
        self._logfile = lf
        handle = self._advapi.OpenTraceW(byref(lf))
        if handle == INVALID_PROCESSTRACE_HANDLE:
            raise RuntimeError(
                f"OpenTraceW({self.name}) failed: "
                f"winerror {kernel32.GetLastError()}")
        self._consumer = _u64(handle)

        def _pump() -> None:
            rc = self._advapi.ProcessTrace(byref(self._consumer), _u32(1),
                                           None, None)
            # ERROR_CANCELLED (1223) is the stop this consumer asked for
            if rc not in (0, 1223):
                self._error.append(f"ProcessTrace returned {rc}")
                self._got.set()
        self._thread = threading.Thread(target=_pump, daemon=True)
        self._thread.start()

    def _owned(self, pid: int) -> bool:
        if pid not in self._known:
            self._known = set(self.tree_pids())
        return pid in self._known

    def _on_event(self, rec_p) -> None:
        if self.first is not None:
            return
        h = rec_p.contents.EventHeader
        src = first_present_of(h.ProviderId.key(), h.EventDescriptor.Id,
                               h.EventDescriptor.Opcode, h.ProcessId,
                               self._owned)
        if src is not None:
            self.first = (h.TimeStamp, src)
            self._got.set()

    def wait(self, budget_s: float) -> tuple[int, str]:
        if not self._got.wait(budget_s):
            raise RuntimeError(
                f"no owned DXGI present within {budget_s} s of launch")
        if self._error:
            raise RuntimeError(self._error[0])
        return self.first

    def stop(self) -> None:
        """Stop the real-time session; ProcessTrace then returns."""
        errors = []
        if self._session.value:
            _buf, props = self._props()
            rc = self._advapi.ControlTraceW(self._session, None,
                                            byref(props),
                                            _u32(EVENT_TRACE_CONTROL_STOP))
            if rc != 0:
                errors.append(f"ControlTraceW(stop {self.name}) failed: {rc}")
            self._session = _u64(0)
        if self._consumer is not None:
            rc = self._advapi.CloseTrace(self._consumer)
            # ERROR_CTX_CLOSE_PENDING: ProcessTrace is still draining
            if rc not in (0, 7007):
                errors.append(f"CloseTrace failed: {rc}")
            self._consumer = None
        if self._thread is not None:
            self._thread.join(timeout=5)
            if self._thread.is_alive():
                errors.append("real-time ETW consumer did not stop")
            self._thread = None
        if errors:
            raise RuntimeError("; ".join(errors))


SOURCE_LABELS = {
    "A": "dxgi_present (IDXGISwapChain::Present — the same events PresentMon reports)",
    "B": "dxgi_composition_present (DXGI event 144 — emitted by WinUI 3's compositor; below PresentMon's keyword mask)",
    "C": "dxgkrnl_cdd_blit (per-frame DxgkCddDrvBitBlt — Electron's submission path on this adapter)",
    "D": "dxgkrnl_present_history (PresentHistory_Start fallback)",
}


def parse_frames(csv_path: Path, app_pids: set[int]) -> dict:
    """Frame-submission timestamps for the owned pid set from the dump.

    Streams tried in order (identical logic for every contestant):
      A. DXGI "Present" task Start events — classic swapchain presenters
         (waterui/vello, flutter); identical to what PresentMon reports.
      B. DXGI event 144 task 63 — composition-path present marker emitted by
         WinUI 3's compositor (PresentMon's keyword mask filters it out).
      C. DxgKrnl event 107 "DxgkCddDrvBitBlt" — per-frame CDD blit; the only
         frame signal Chromium/Electron produces on the Basic Render Driver.
      D. DxgKrnl PresentHistory task (6) event 171 — fallback.
    """
    frame_rows: list[tuple[int, int, str]] = []
    proc_starts: dict[int, int] = {}
    owned = set(app_pids)
    with open(csv_path, newline="", errors="replace") as f:
        for r in csv.reader(f):
            if len(r) < 20 or "Event" in r[0][:12]:
                continue
            prov, typ = r[0].strip(), r[1].strip()
            try:
                pid = int(r[9].strip(), 16)
                clock = int(r[16].strip())
            except ValueError:
                continue
            if prov == KPROC:
                if typ == "Start":
                    try:
                        child = int(r[19].split(",")[0].strip())
                        proc_starts[child] = clock
                    except (ValueError, IndexError):
                        pass
                continue
            eid = r[2].strip()
            if prov == DXGI and typ == "Start" and eid == "42":
                frame_rows.append((pid, clock, "A"))
            elif prov == DXGI and typ == "Start" and eid == "144":
                frame_rows.append((pid, clock, "B"))
            elif (
                prov == DXGKRNL
                and typ == "Start"
                and eid == "107"
                and "DxgkCddDrvBitBlt" in r[19]
            ):
                frame_rows.append((pid, clock, "C"))
            elif (
                prov == DXGKRNL
                and typ == "Start"
                and r[7].strip() == "6"
                and eid == "171"
            ):
                frame_rows.append((pid, clock, "D"))

    app_pids = owned
    chosen = [(pid, clock, src) for pid, clock, src in frame_rows if pid in app_pids]
    by_src: dict[str, list[int]] = {}
    for _, clock, s in chosen:
        by_src.setdefault(s, []).append(clock)

    for src in ("A", "B", "C", "D"):
        if len(by_src.get(src, ())) >= 4:
            return {
                "present_events": len(by_src[src]),
                "source": src,
                "timestamps_100ns": sorted(by_src[src]),
                "proc_starts": proc_starts,
                "app_pids": sorted(app_pids),
            }
    if by_src:
        src = max(by_src, key=lambda k: len(by_src[k]))
        return {
            "present_events": len(by_src[src]),
            "source": src,
            "timestamps_100ns": sorted(by_src[src]),
            "proc_starts": proc_starts,
            "app_pids": sorted(app_pids),
        }
    return {
        "present_events": 0,
        "source": None,
        "timestamps_100ns": [],
        "proc_starts": proc_starts,
        "app_pids": sorted(app_pids),
    }


# ---------------------------------------------------------------------------
# Measurement loop
# ---------------------------------------------------------------------------


def package_size(dist_dir: Path) -> dict:
    total = sum(f.stat().st_size for f in dist_dir.rglob("*") if f.is_file())
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for f in sorted(dist_dir.rglob("*")):
            if f.is_file():
                z.write(f, f.relative_to(dist_dir))
    return {"uncompressed_bytes": total, "compressed_bytes": buf.tell()}


_ADAPTER_RE = re.compile(
    r"selected wgpu adapter.*name:\s*\"(?P<name>[^\"]+)\".*?"
    r"device_type:\s*(?P<dt>[A-Za-z]+).*?backend:\s*(?P<backend>[A-Za-z0-9]+)",
    re.DOTALL,
)


def adapter_from_log(log_path: Path) -> dict | None:
    """Parse hydrolysis's `selected wgpu adapter` log line.

    Returns {"name", "device_type", "backend"} or None when the app never
    reached adapter selection in this run's log.
    """
    try:
        text = log_path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    m = _ADAPTER_RE.search(text)
    if not m:
        return None
    return {
        "name": m.group("name"),
        "device_type": m.group("dt"),
        "backend": m.group("backend"),
    }


def adapter_label(c: dict, detected: dict | None) -> str:
    """'Name (backend device_type)' from the run's own renderer evidence."""
    if detected:
        label = (
            f"{detected['name']} ({detected['backend']}, "
            f"{detected['device_type']})"
        )
        if detected.get("mechanism"):
            label += f" [{detected['mechanism']}]"
        return label
    return c["adapter"]


# ---------------------------------------------------------------------------
# Renderer evidence — which adapter the MEASURED process actually used.
# Host inventory (Win32_VideoController, DXGI enumeration) proves only
# availability. Every contestant row must carry one of:
#   * hydrolysis's own 'selected wgpu adapter' log line (waterui)
#   * Electron's app.getGPUInfo('complete') from the measured process
#   * the owned pid set's GPU-engine perf counters attributed to a
#     physical adapter LUID resolved through DXGI
# Missing or ambiguous evidence fails the attempt — it never silently
# passes as a measurement.
# ---------------------------------------------------------------------------


class RendererEvidenceError(RuntimeError):
    """No trustworthy adapter evidence for this attempt."""


class SoftwareRendererError(RuntimeError):
    """The measured tree rendered on a software adapter — refuse."""


_GPUINFO_LINE = re.compile(r"BENCH_GPUINFO (\{.*\})")


def gpuinfo_from_log(log_path: Path) -> dict | None:
    """Electron's own app.getGPUInfo('complete') report — vendor/device
    ids plus auxAttributes.glRenderer for the adapter in use."""
    try:
        text = log_path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    m = _GPUINFO_LINE.search(text)
    if not m:
        return None
    try:
        info = json.loads(m.group(1))
    except ValueError:
        return None
    aux = info.get("auxAttributes") or {}
    device = next(
        (d for d in info.get("gpuDevice", []) if d.get("active")),
        info.get("gpuDevice", [{}])[0] if info.get("gpuDevice") else {},
    )
    return {
        "gl_renderer": aux.get("glRenderer") or "",
        "vendor_id": device.get("vendorId"),
        "device_id": device.get("deviceId"),
        "raw": info,
    }


_ENGINE_INSTANCE = re.compile(
    r"pid_(\d+)_luid_0x([0-9a-fA-F]+)_0x([0-9a-fA-F]+)")


def gpu_engine_counters() -> list[tuple[int, tuple[int, int], float]]:
    """(pid, (luid_low, luid_high), utilisation) from the documented
    '\\GPU Engine(*)\\Utilization Percentage' perf counters — the OS's
    own attribution of GPU work to a pid on a specific adapter LUID.
    The instance encodes `luid_0x<HighPart>_0x<LowPart>` — HighPart
    comes FIRST in the name."""
    out = subprocess.run(
        ["powershell", "-NoProfile", "-Command",
         "(Get-Counter '\\GPU Engine(*)\\Utilization Percentage' "
         "-SampleInterval 1 -MaxSamples 3).CounterSamples | "
         "ForEach-Object { $_.Path + '|' + $_.CookedValue }"],
        capture_output=True, text=True)
    hits: list[tuple[int, tuple[int, int], float]] = []
    for line in out.stdout.splitlines():
        path, _, val = line.rpartition("|")
        m = _ENGINE_INSTANCE.search(path)
        if not m:
            continue
        try:
            util = float(val.strip())
        except ValueError:
            continue
        if util <= 0:
            continue
        hits.append((int(m.group(1)),
                     (int(m.group(3), 16), int(m.group(2), 16)), util))
    return hits


class _LUID(ctypes.Structure):
    _fields_ = [("LowPart", ctypes.c_ulong),
                ("HighPart", ctypes.c_long)]


class _DXGI_ADAPTER_DESC(ctypes.Structure):
    _fields_ = [
        ("Description", ctypes.c_wchar * 128),
        ("VendorId", ctypes.c_uint),
        ("DeviceId", ctypes.c_uint),
        ("SubSysId", ctypes.c_uint),
        ("Revision", ctypes.c_uint),
        ("DedicatedVideoMemory", ctypes.c_size_t),
        ("DedicatedSystemMemory", ctypes.c_size_t),
        ("SharedSystemMemory", ctypes.c_size_t),
        ("AdapterLuid", _LUID),
    ]


# DXGI interfaces declared with comtypes — the maintained COM binding
# computes each method's vtable slot from the official interface
# inheritance chain, so no hand-indexed vtable calls remain. Inherited
# methods we never call are declared as slots (no argument spec) purely
# to keep the indices of the ones we do call honest:
#   IDXGIAdapter::GetDesc        = IUnknown(3) + IDXGIObject(4) + 1 = 8
#   IDXGIFactory1::EnumAdapters1 = 3 + 4 + IDXGIFactory(5)        = 12
if comtypes is not None:
    class IDXGIObject(IUnknown):
        _iid_ = GUID("{aec22fb8-76f3-4639-9be0-28eb43a67a2e}")
        _methods_ = [
            COMMETHOD([], HRESULT, "SetPrivateData"),
            COMMETHOD([], HRESULT, "SetPrivateDataInterface"),
            COMMETHOD([], HRESULT, "GetPrivateData"),
            COMMETHOD([], HRESULT, "GetParent"),
        ]

    class IDXGIAdapter(IDXGIObject):
        _iid_ = GUID("{2411e7e1-12ac-4ccf-bd14-9798e8534dc0}")
        _methods_ = [
            COMMETHOD([], HRESULT, "EnumOutputs", (
                ["in"], ctypes.c_uint, "Output"), (
                ["out"], POINTER(c_void_p), "ppOutput")),
            COMMETHOD([], HRESULT, "GetDesc", (
                ["out"], POINTER(_DXGI_ADAPTER_DESC), "pDesc")),
            COMMETHOD([], HRESULT, "CheckInterfaceSupport"),
        ]

    class IDXGIFactory(IDXGIObject):
        _iid_ = GUID("{7b7166ec-21c7-44ae-b21a-c9ae321ae369}")
        _methods_ = [
            COMMETHOD([], HRESULT, "EnumAdapters", (
                ["in"], ctypes.c_uint, "Adapter"), (
                ["out"], POINTER(POINTER(IDXGIAdapter)), "ppAdapter")),
            COMMETHOD([], HRESULT, "MakeWindowAssociation"),
            COMMETHOD([], HRESULT, "GetWindowAssociation"),
            COMMETHOD([], HRESULT, "CreateSwapChain"),
            COMMETHOD([], HRESULT, "CreateSoftwareAdapter"),
        ]

    class IDXGIFactory1(IDXGIFactory):
        _iid_ = GUID("{770aae78-f26f-4dba-a829-253c83d1b387}")
        _methods_ = [
            COMMETHOD([], HRESULT, "EnumAdapters1", (
                ["in"], ctypes.c_uint, "Adapter"), (
                ["out"], POINTER(POINTER(IDXGIAdapter)), "ppAdapter")),
            # IsCurrent returns BOOL, not HRESULT — declaring it
            # HRESULT would misread the return
            COMMETHOD([], wintypes.BOOL, "IsCurrent"),
        ]
else:
    IDXGIObject = IDXGIAdapter = IDXGIFactory = IDXGIFactory1 = None

# DXGI_ERROR_NOT_FOUND — the only documented end-of-enumeration marker;
# every other HRESULT failure propagates
DXGI_ERROR_NOT_FOUND = 0x887A0002


def dxgi_adapters() -> dict[tuple[int, int], dict]:
    """adapter LUID → {name, vendor_id, device_id} via
    IDXGIFactory1::EnumAdapters1 + IDXGIAdapter::GetDesc (DXGI 1.1),
    dispatched through the declared comtypes interfaces.

    CreateDXGIFactory1 is prototyped as documented —
    (REFIID, POINTER(POINTER(IDXGIFactory1))) — and the returned
    comtypes pointers own their refcounts: no manual Release() (that
    would double-release against comtypes' __del__).
    """
    if comtypes is None or windll is None:
        raise RuntimeError("dxgi_adapters requires a Windows host")
    create_factory = windll.dxgi.CreateDXGIFactory1
    create_factory.restype = HRESULT
    create_factory.argtypes = [
        POINTER(GUID), POINTER(POINTER(IDXGIFactory1))]
    comtypes.CoInitialize()
    try:
        factory = POINTER(IDXGIFactory1)()
        hr = create_factory(byref(IDXGIFactory1._iid_), byref(factory))
        if hr != 0 or not factory:
            raise OSError(
                "CreateDXGIFactory1 failed: "
                f"HRESULT 0x{hr & 0xFFFFFFFF:08X}")
        out: dict[tuple[int, int], dict] = {}
        i = 0
        while True:
            try:
                adapter = factory.EnumAdapters1(i)
            except comtypes.COMError as e:
                if (e.hresult & 0xFFFFFFFF) == DXGI_ERROR_NOT_FOUND:
                    break
                raise
            desc = adapter.GetDesc()
            if hasattr(desc, "contents"):
                desc = desc.contents
            luid = desc.AdapterLuid
            out[(luid.LowPart, luid.HighPart & 0xFFFFFFFF)] = {
                "name": desc.Description,
                "vendor_id": desc.VendorId,
                "device_id": desc.DeviceId,
            }
            i += 1
        return out
    finally:
        comtypes.CoUninitialize()


def renderer_evidence(c_key: str, c: dict, owned_pids: set[int],
                      log_path: Path | None,
                      counter_samples=None,
                      luid_map: dict | None = None) -> dict:
    """The adapter THIS rep actually rendered on — required per attempt.

    Returns {"name", "device_type", "backend", "mechanism"}. Raises
    RendererEvidenceError when evidence is missing or ambiguous (recorded
    as a failed attempt) and SoftwareRendererError when the owned tree
    provably rendered on a software adapter (refuses the measurement).
    """
    if c.get("adapter_from_log"):
        det = adapter_from_log(log_path) if log_path else None
        if not det:
            raise RendererEvidenceError(
                f"{c_key}: no 'selected wgpu adapter' line in the run's "
                "own output — adapter unproven")
        if det["device_type"] == "Cpu" or is_software_gpu(det["name"]):
            raise SoftwareRendererError(
                f"{det['name']} [{det['device_type']}]")
        return {**det, "mechanism": "hydrolysis adapter log"}

    if c.get("gpuinfo_log"):
        info = gpuinfo_from_log(log_path) if log_path else None
        if info is None:
            raise RendererEvidenceError(
                f"{c_key}: no BENCH_GPUINFO report from the measured "
                "process — renderer unproven")
        if is_software_gpu(info["gl_renderer"]):
            raise SoftwareRendererError(info["gl_renderer"])
        name = info["gl_renderer"] or (
            f"vendor {info['vendor_id']!r} device {info['device_id']!r}")
        if not info["gl_renderer"]:
            raise RendererEvidenceError(
                f"{c_key}: getGPUInfo reported no glRenderer string")
        return {"name": name, "device_type": "Gpu",
                "backend": "chromium", "mechanism": "app.getGPUInfo"}

    if c.get("gpu_engine_evidence"):
        samples = (
            counter_samples if counter_samples is not None
            else gpu_engine_counters()
        )
        hits: dict[tuple[int, int], float] = {}
        for pid, luid, util in samples:
            if pid in owned_pids:
                hits[luid] = hits.get(luid, 0.0) + util
        if not hits:
            raise RendererEvidenceError(
                f"{c_key}: no GPU-engine activity attributed to the "
                f"owned pid set {sorted(owned_pids)} — renderer unproven")
        adapters = luid_map if luid_map is not None else dxgi_adapters()
        used = []
        for luid in hits:
            a = adapters.get(luid)
            if a is None:
                raise RendererEvidenceError(
                    f"{c_key}: GPU engine used LUID {luid} which does not "
                    "map to any enumerated adapter — attribution ambiguous")
            used.append(a)
        soft = [a["name"] for a in used if is_software_gpu(a["name"])]
        if soft:
            raise SoftwareRendererError("; ".join(soft))
        return {
            "name": "; ".join(sorted({a["name"] for a in used})),
            "device_type": "Gpu",
            "backend": "d3d",
            "mechanism": "gpu-engine counter → DXGI LUID",
            "luids": [f"{lo:#x}/{hi:#x}" for lo, hi in hits],
        }
    raise RendererEvidenceError(
        f"{c_key}: no renderer-evidence mechanism configured")


def wait_for_ready(app, budget_s: float) -> tuple:
    """Block until the owned process is input-idle AND owns a visible
    window; returns (hwnd, rect) of its largest visible top-level window.

    Both stages block on the events themselves: WaitForInputIdle on the
    owned root handle, then an EVENT_OBJECT_SHOW WinEvent hook. The hook
    is installed FIRST and only then is the window list snapshotted, so a
    window that becomes visible at any moment is either in the snapshot
    or delivered to the hook — never lost between the two. `budget_s`
    bounds the total wait; raises when it expires without readiness.
    Readiness only locates the window: the measurement window anchors on
    the first owned present (FirstPresentConsumer)."""
    deadline = time.monotonic() + budget_s
    hproc = getattr(app, "hproc", None)
    if hproc is not None and win32event is not None:
        # documented input-idle wait on the owned root handle —
        # WaitForInputIdle lives on win32event, not win32process
        rc = win32event.WaitForInputIdle(hproc, int(budget_s * 1000))
        if rc != 0:
            raise RuntimeError(f"WaitForInputIdle returned {rc}")

    got = threading.Event()
    installed = threading.Event()
    hook_error: list[str] = []
    hits: list[tuple[int, tuple, int]] = []
    EVENT_OBJECT_SHOW = 0x8002
    OBJID_WINDOW = 0
    WINEVENT_OUTOFCONTEXT = 0

    @WINFUNCTYPE(None, c_void_p, c_ulong, c_void_p,
                 c_long, c_long, c_ulong, c_ulong)
    def _on_show(_hook, event, hwnd, id_object, id_child, _tid, _t):
        if event != EVENT_OBJECT_SHOW \
                or id_object != OBJID_WINDOW or id_child != 0:
            return
        pid = c_ulong(0)
        user32.GetWindowThreadProcessId(hwnd, byref(pid))
        if pid.value not in app.pids() or not user32.IsWindowVisible(hwnd):
            return
        rect = (c_int * 4)()
        user32.GetWindowRect(hwnd, rect)
        area = max(0, rect[2] - rect[0]) * max(0, rect[3] - rect[1])
        if area:
            hits.append((hwnd, tuple(rect), area))
            got.set()

    # out-of-context hooks are delivered to the installing thread's
    # message queue — the hook thread must pump messages
    def _hook_loop() -> None:
        hook = user32.SetWinEventHook(
            EVENT_OBJECT_SHOW, EVENT_OBJECT_SHOW, None, _on_show,
            0, 0, WINEVENT_OUTOFCONTEXT)
        if not hook:
            hook_error.append(
                f"SetWinEventHook failed: winerror {kernel32.GetLastError()}")
            installed.set()
            return
        installed.set()
        try:
            msg = ctypes.wintypes.MSG()
            while user32.GetMessageW(byref(msg), None, 0, 0) != 0:
                user32.TranslateMessage(byref(msg))
                user32.DispatchMessageW(byref(msg))
        finally:
            user32.UnhookWinEvent(hook)

    WM_QUIT = 0x0012
    hook_thread = threading.Thread(target=_hook_loop, daemon=True)
    hook_thread.start()
    try:
        if not installed.wait(max(0.0, deadline - time.monotonic())):
            raise RuntimeError("WinEvent hook thread did not start")
        if hook_error:
            raise RuntimeError(hook_error[0])
        # the hook is live: a window already visible is in this snapshot,
        # one shown from now on reaches the hook
        snap = window_for_pids(app.pids())
        if snap:
            return snap
        if not got.wait(max(0.0, deadline - time.monotonic())):
            raise RuntimeError(
                "no owned visible window before readiness deadline")
    finally:
        user32.PostThreadMessageW(hook_thread.native_id, WM_QUIT, 0, 0)
    # a later SHOW can still land bigger — take the largest recorded
    return max(hits, key=lambda r: r[2])[:2]


# the real-time and the file session stamp the same event from one QPC
# read, each converting it to system time — they agree to well within this
ANCHOR_TOLERANCE_100NS = 10_000  # 1 ms


def require_same_anchor(trace_first: int, trace_source: str | None,
                        rt_first: int, rt_source: str) -> None:
    """The present the drive was scheduled on (real-time consumer) and the
    present the window anchors on (the dump's first owned present of the
    chosen source) must be one event; otherwise the drive did not start
    at window start."""
    if trace_source != rt_source \
            or abs(trace_first - rt_first) > ANCHOR_TOLERANCE_100NS:
        raise RuntimeError(
            f"drive anchor mismatch: scheduled on source {rt_source} at "
            f"{rt_first}, the trace's window anchors on source "
            f"{trace_source} at {trace_first} "
            f"({(trace_first - rt_first) / 1e4:+.1f} ms)")


def measure_run(c_key: str, workload: str, rep: int, cfg,
                providers_file: Path,
                owned_launch=None, owned_first_present=None) -> dict:
    """One rep — ALWAYS returns an attempt record.

    On success the record carries the metrics; on any launch/capture/
    parser failure it carries {"run", "error", "detail"} and the rep
    loop preserves it. Nothing retries an attempt behind the scenes.
    Trace stop, sampler stop, process-tree termination, log close and
    desktop restoration all run in `finally`, so a failure anywhere in
    the rep still leaves the machine clean — and the adapter-refusal
    SystemExit (a BaseException) still propagates past the record path.

    `owned_launch` and `owned_first_present` are injectable for
    off-Windows tests of this control logic; on Windows they default to
    OwnedApp (Job Object) and FirstPresentConsumer (real-time ETW).
    """
    c = CONTESTANTS[c_key]
    minimized: list[int] = []
    app = None
    sampler = None
    first_present_rt = None
    trace_name = f"bench1262_{os.getpid()}_{rep}"
    trace_started = False
    etl = TRACE_DIR / f"{c_key}_{workload}_{rep}.etl"
    dump = etl.with_suffix(".csv")
    log_path = etl.with_suffix(".log")
    rec: dict | None = None
    refuse: SoftwareRendererError | None = None
    cleanup_errors: list[str] = []

    def _cleanup() -> None:
        # every cleanup runs on every path — success, attempt failure,
        # or the software-renderer refusal — and a failure in one step
        # never skips the rest; failures are collected, not suppressed
        if first_present_rt is not None:
            try:
                first_present_rt.stop()
            except Exception as e:
                cleanup_errors.append(f"first-present consumer: {e}")
        if trace_started:
            try:
                stop_trace(trace_name)
            except Exception as e:
                cleanup_errors.append(f"stop_trace: {e}")
        if sampler is not None:
            try:
                sampler.stop()
                sampler.join(timeout=5)
                if sampler.error is not None:
                    cleanup_errors.append(
                        f"sampler: {sampler.error}")
            except Exception as e:
                cleanup_errors.append(f"sampler: {e}")
        if app is not None:
            try:
                app.terminate()
            except Exception as e:
                cleanup_errors.append(f"app terminate: {e}")
        try:
            restore_windows(minimized)
        except Exception as e:
            cleanup_errors.append(f"restore: {e}")

    try:
        minimized = minimize_other_windows()
        start_trace(trace_name, etl, providers_file)
        trace_started = True
        # the real-time consumer is live before the launch, so the owned
        # tree's first present cannot pass unseen
        first_present_rt = (owned_first_present or FirstPresentConsumer)(
            f"{trace_name}_rt",
            lambda: app.pids() if app is not None else set())

        # every contestant's output is captured — evidence mechanisms
        # read it (hydrolysis adapter line, Electron BENCH_GPUINFO)
        app = (owned_launch or OwnedApp)(c, workload, log_path)
        root_pid = app.pid
        # a missing startup timestamp is a failed attempt, not a quiet
        # None in the row — GetProcessTimes is on the owned handle
        create_ft = app.create_filetime()

        key = {"w2": "capture_seconds_w2", "w3": "capture_seconds_w3"}.get(
            workload, "capture_seconds_static"
        )
        capture_s = cfg["runner"][key]
        warmup_s = cfg["runner"]["warmup_seconds"]
        if warmup_s <= 0:
            raise RuntimeError(
                "runner.warmup_seconds must be > 0 — the warmup is "
                "declared in the manifest and is never zero (METHOD)")

        # readiness locates the window (event-driven, bounded by
        # ready_timeout_seconds); the window itself anchors on the first
        # owned present the real-time consumer reports
        hwnd_info = wait_for_ready(
            app, cfg["runner"]["ready_timeout_seconds"])
        rt_first_ft, rt_source = first_present_rt.wait(
            cfg["runner"]["ready_timeout_seconds"])
        first_present_rt.stop()
        first_present_rt = None

        owned = app.pids()
        sampler = MemorySampler(
            app.pids, cfg["runner"]["memory_sample_interval_ms"] / 1000
        )
        sampler.start()
        bring_to_foreground(hwnd_info[0])

        # The measurement window is [first owned present + warmup,
        # +capture] and the drive program starts at window start
        # (METHOD): both are deadlines on the first-present event's own
        # timestamp, on the FILETIME clock the dump and the sampler share.
        rt_win_start = rt_first_ft + int(warmup_s * 10_000_000)
        rt_win_end = rt_win_start + int(capture_s * 10_000_000)
        lead = (rt_win_start - filetime_now()) / 1e7
        if lead < 0:
            raise RuntimeError(
                f"the window opened {-lead * 1000:.0f} ms before the "
                "runner could start the drive — first-present delivery "
                "or foreground activation outlasted the declared warmup")
        time.sleep(lead)
        if workload in SCROLL_WORKLOADS:
            fling_window(hwnd_info[0], hwnd_info[1],
                         (rt_win_end - filetime_now()) / 1e7, cfg["runner"])
        rest = (rt_win_end - filetime_now()) / 1e7
        if rest > 0:
            time.sleep(rest)

        # GPU-engine attribution while the app is still alive — the
        # counters only exist for active engines
        engine_samples = (
            gpu_engine_counters()
            if c.get("gpu_engine_evidence") else None
        )

        sampler.stop()
        sampler.join()
        if sampler.error is not None:
            # a failed native PID-list/memory query inside the sampler
            # thread reaches the record as an attempt failure
            raise RuntimeError(
                f"memory sampler failed: {sampler.error}")
        owned |= app.pids()  # catch descendants spawned late in the run

        stop_trace(trace_name)
        trace_started = False
        app.terminate()
        app = None

        etl_to_csv(etl, dump)
        frames = parse_frames(dump, owned)
        if not frames["timestamps_100ns"]:
            raise RuntimeError(
                "no owned present events — the measurement window has "
                "no first present to anchor on")
        # measurement window: first owned present + declared warmup,
        # capture_s wide — startup frames before it are trimmed, and
        # memory samples (FILETIME-stamped) trim to the same window
        first_present = min(frames["timestamps_100ns"])
        # the drive was scheduled on the real-time first present: it must
        # be the very event the trace anchors the window on
        require_same_anchor(first_present, frames["source"],
                            rt_first_ft, rt_source)
        win_start = first_present + int(warmup_s * 10_000_000)
        win_end = win_start + int(capture_s * 10_000_000)
        windowed_ts = [t for t in frames["timestamps_100ns"]
                       if win_start <= t <= win_end]
        frames["timestamps_100ns"] = windowed_ts
        frames["measurement_window_100ns"] = [win_start, win_end]
        frames["first_present_100ns"] = first_present
        mem = sampler.summarise(window=(win_start, win_end))
        if mem["samples_taken"] == 0:
            raise RuntimeError(
                "zero memory samples inside the measurement window")
        # Evidence precedes metrics: a rep with no proven renderer is a
        # failed attempt, and a proven software renderer refuses the
        # measurement outright — under --development as well (no
        # software-GPU measurement route exists in this runner).
        adapter = renderer_evidence(
            c_key, c, owned, log_path,
            counter_samples=engine_samples)
        # one frame-statistics definition for every leg (METHOD):
        # windowed presents in ms; >100 ms gaps end a run (excluded);
        # missed vsyncs = round(interval/period) - 1 over 1.5 periods
        stats = lib_frames.frame_statistics(
            [t / 10000.0 for t in windowed_ts],
            window_start_ms=win_start / 10000.0,
            capture_ms=capture_s * 1000.0,
            refresh_ms=cfg["measurement"]["vsync_budget_ms"],
        )

        # startup is launch→first owned present — never from windowed
        # data (first_present anchors the window; it precedes it)
        startup_ms = None
        create = frames["proc_starts"].get(root_pid, create_ft)
        if create:
            startup_ms = (first_present - create) / 10000.0

        rec = {
            "run": rep,
            "startup_ms": startup_ms,
            "adapter_detected": adapter,
            "memory": mem,
            "frame_rate": {
                **stats,
                "present_events": frames["present_events"],
                "source": frames["source"],
                "source_label": SOURCE_LABELS.get(frames["source"]),
            },
        }
    except SoftwareRendererError as sre:
        refuse = sre
    except Exception as e:
        rec = {
            "run": rep,
            "error": f"{type(e).__name__}: {e}",
            "detail": traceback.format_exc()[-1500:],
        }
    finally:
        _cleanup()

    if refuse is not None:
        # the owned tree provably rendered on a software adapter —
        # refuse the measurement (exit 2); never a per-rep flake and
        # never permitted under --development
        detail = ("; cleanup: " + "; ".join(cleanup_errors)
                  if cleanup_errors else "")
        print(
            "refusing to measure: "
            f"{c_key} rendered on a software adapter ({refuse}) — "
            "frame-time and memory numbers would not be hardware "
            f"evidence{detail}",
            file=sys.stderr,
        )
        raise SystemExit(2)
    if cleanup_errors:
        # cleanup diagnostics always reach the record; a cleanup failure
        # on an otherwise measured attempt degrades it to a failed
        # attempt — its numbers are not trustworthy once teardown broke
        joined = "; ".join(cleanup_errors)
        if rec is not None and "error" not in rec:
            rec = {"run": rep,
                   "error": f"cleanup failure: {joined}",
                   "metrics_discarded": rec}
        elif rec is not None:
            rec["error"] += f" | cleanup: {joined}"
        else:
            rec = {"run": rep, "error": f"cleanup failure: {joined}"}
        rec["cleanup_errors"] = cleanup_errors
    return rec


# ---------------------------------------------------------------------------
# Aggregation
# ---------------------------------------------------------------------------


def stats_of(samples: list) -> dict | None:
    vals = [s for s in samples if s is not None]
    if not vals:
        return None
    return {
        "median": statistics.median(vals),
        "min": min(vals),
        "max": max(vals),
        "samples": vals,
        "n": len(vals),
    }


def repetition_check(name: str, wl: str, good: list, reps: int,
                     attempted: int) -> str | None:
    """A cell counts only with >= reps successful attempts."""
    if len(good) < reps:
        return (f"{name}/{wl}: {len(good)}/{reps} successful "
                f"({attempted} attempted)")
    return None


SOFTWARE_GPU_MARKERS = (
    "basic render",        # Microsoft Basic Render Driver (WARP-class)
    "basic display",       # Microsoft Basic Display Adapter
    "warp",
    "swiftshader",
    "llvmpipe",
    "lavapipe",
    "softpipe",
)


def is_software_gpu(name: str) -> bool:
    n = name.lower()
    return any(m in n for m in SOFTWARE_GPU_MARKERS)


def machine_spec() -> dict:
    def ps(cmd: str) -> str:
        out = subprocess.run(
            ["powershell", "-NoProfile", "-Command", cmd],
            capture_output=True,
            text=True,
        )
        return out.stdout.strip()

    if platform.system() != "Windows":
        raise SystemExit("run.py measures on Windows hosts only")

    gpus = [
        g.strip()
        for g in ps(
            "(Get-CimInstance Win32_VideoController | "
            "Select-Object -ExpandProperty Name) -join ';'"
        ).split(";")
        if g.strip()
    ]
    gpu = "; ".join(gpus) if gpus else "unknown"
    return {
        "cpu": ps("(Get-CimInstance Win32_Processor).Name"),
        "ram_gb": ps(
            "[math]::Round((Get-CimInstance Win32_ComputerSystem)"
            ".TotalPhysicalMemory/1GB,1)"
        ),
        "gpu": gpu,
        "gpu_software": all(is_software_gpu(g) for g in gpus) if gpus else True,
        "os": f"Windows build {platform.version()}",
        "python": platform.python_version(),
        "display": ps(
            "(Get-CimInstance Win32_DesktopMonitor | "
            "Select-Object -First 1 -ExpandProperty Name)"
        ) or "unknown",
    }


def _self_test() -> None:
    """CPU-only validation of the runner's own control logic.

    measure_run runs for real — the ONLY substitutions are at the OS
    boundary (window manager, ETW, process launch): a fake owned app
    standing in for OwnedApp and recorders for trace/window calls. Fault
    injection drives the failure paths; native Windows acceptance still
    happens on a Windows host.
    """
    events: list[str] = []
    TRACE_DIR.mkdir(parents=True, exist_ok=True)
    cfg = {
        "runner": {"ready_timeout_seconds": 0,
                   # warmup trims startup frames; the capture window is
                   # intentionally wider than a rep so fixture samples
                   # always land inside it
                   "warmup_seconds": 0.1,
                   "capture_seconds_static": 0.3,
                   "capture_seconds_w2": 0.3, "capture_seconds_w3": 0.3,
                                      "memory_sample_interval_ms": 10,
                   "fling_down": 8, "fling_up": 2, "fling_detents": 12,
                   "fling_duration_ms": 250, "fling_pause_ms": 350},
        "measurement": {"vsync_budget_ms": 16.7},
    }

    # per-rep ETW-clock (FILETIME 100ns) anchor — the measurement
    # window is derived from the first present, and memory samples
    # stamp filetime_now(), so the fixture clock is reset at each
    # launch to keep the sampler inside the window
    # the fixture clock stands in for GetSystemTimePreciseAsFileTime so
    # the self-test runs on any host; it is installed as filetime_now
    # below for the whole rep
    def fixture_filetime() -> int:
        return int((time.time() + 11644473600) * 10_000_000)

    ts = {"t0": fixture_filetime()}

    class FakeApp:
        """Same contract as OwnedApp: pid, pids(), create_filetime(),
        terminate(), log."""
        def __init__(self, c, workload, log_path):
            events.append("launch")
            ts["t0"] = fixture_filetime()
            self.pid = os.getpid()
            self.hproc = None
            self.terminated = False
            if log_path:
                self.log = open(log_path, "w")
        def create_filetime(self):
            return ts["t0"] - 100_000
        def pids(self):
            return {os.getpid()}
        def terminate(self):
            self.terminated = True
            if getattr(self, "log", None):
                self.log.close()
            events.append("terminate")

    # the real-time first-present consumer at the OS boundary: it
    # reports the rep's launch-clock first present as source A
    rt = {"first": None}

    class FakeFirstPresent:
        def __init__(self, name, tree_pids):
            events.append("rt_start")
        def wait(self, budget_s):
            return rt["first"] or (ts["t0"], "A")
        def stop(self):
            events.append("rt_stop")

    saved = {k: globals()[k] for k in (
        "minimize_other_windows", "restore_windows", "start_trace",
        "stop_trace", "etl_to_csv", "parse_frames", "window_for_pids",
        "process_memory_snapshot", "adapter_from_log",
        "renderer_evidence", "bring_to_foreground", "wait_for_ready",
        "FirstPresentConsumer", "filetime_now")}
    try:
        globals()["filetime_now"] = fixture_filetime
        globals()["FirstPresentConsumer"] = FakeFirstPresent
        globals()["wait_for_ready"] = \
            lambda app, budget: (42, (10, 10, 800, 600))
        globals()["minimize_other_windows"] = \
            lambda: events.append("minimize") or [42]
        globals()["restore_windows"] = \
            lambda hw: events.append(f"restore{hw}")
        globals()["start_trace"] = \
            lambda n, e, p: events.append("trace_start")
        globals()["stop_trace"] = \
            lambda n: events.append("trace_stop")
        globals()["window_for_pids"] = \
            lambda pids: (42, 10, 10, 800, 600)
        globals()["bring_to_foreground"] = \
            lambda hwnd: events.append("foreground")
        globals()["adapter_from_log"] = lambda p: None
        globals()["renderer_evidence"] = lambda *a, **kw: {
            "name": "Fixture RTX", "device_type": "Gpu",
            "backend": "d3d", "mechanism": "fixture"}

        def _fixture_frames(d, pids):
            # anchored at the rep's launch clock — memory samples stamp
            # filetime_now() during the rep, so the fixture must share
            # that clock or the measurement-window trim empties it
            t0 = ts["t0"]
            return {
                "present_events": 6, "source": "A",
                "timestamps_100ns": [
                    t0,                  # first owned present → anchor
                    t0 + 500_000,        # inside warmup → trimmed
                    t0 + 1_100_000,      # window [t0+warmup, +capture]
                    t0 + 2_000_000,
                    t0 + 2_900_000,
                    t0 + 3_500_000,
                ],
                "proc_starts": {os.getpid(): t0 - 100_000},
                "app_pids": sorted(pids),
            }
        globals()["parse_frames"] = _fixture_frames
        globals()["etl_to_csv"] = lambda e, d: events.append("etl_csv")
        globals()["process_memory_snapshot"] = lambda: {
            os.getpid(): {"name": "fake.exe", "ws_private": 100 << 20,
                          "peak_ws": 110 << 20,
                          "private_bytes": 90 << 20}}

        # -- success path: cleanup order is minimize→…→trace_stop→
        #    terminate→restore, and restore receives exactly the hwnds
        #    minimize returned
        rec = measure_run("waterui", "w1", 0, cfg, Path("prov.txt"),
                          owned_launch=FakeApp)
        assert "error" not in rec, rec
        assert rec["memory"]["steady_private_ws_mb"] == \
            (100 << 20) / 1e6
        assert rec["memory"]["samples_taken"] >= 1
        # the measurement window trimmed the startup/warmup frames —
        # stats come from the windowed series, not all 6 presents
        assert rec["frame_rate"]["present_events"] == 6
        assert rec["frame_rate"]["presents"] == 4
        # the windowed presents form one active run; 90/90/60 ms
        # intervals each exceed 1.5 periods → missed-vsync counting
        # follows lib/frame_stats.py (round(i/period)-1 each)
        assert rec["frame_rate"]["runs"] == 1
        assert rec["frame_rate"]["missed_vsyncs"] == \
            round(90 / 16.7) - 1 + round(90 / 16.7) - 1 + \
            round(60 / 16.7) - 1
        order = [e for e in events if e in (
            "minimize", "trace_start", "rt_start", "launch", "rt_stop",
            "foreground", "etl_csv", "trace_stop", "terminate",
            "restore[42]")]
        assert order == ["minimize", "trace_start", "rt_start", "launch",
                         "rt_stop", "foreground", "trace_stop",
                         "terminate", "etl_csv", "restore[42]"], order

        # -- the drive was scheduled on the real-time first present: a
        #    trace whose window anchors on another event fails the rep,
        #    and so does a first present delivered after the window
        #    already opened
        rt["first"] = "B-source"
        events.clear()
        orig_wait = FakeFirstPresent.wait
        FakeFirstPresent.wait = lambda self, b: (ts["t0"], "B")
        rec = measure_run("waterui", "w1", 0, cfg, Path("prov.txt"),
                          owned_launch=FakeApp)
        assert "drive anchor mismatch" in rec["error"], rec
        FakeFirstPresent.wait = lambda self, b: (ts["t0"] - 10_000_000, "A")
        rec = measure_run("waterui", "w1", 0, cfg, Path("prov.txt"),
                          owned_launch=FakeApp)
        assert "before the runner could start the drive" in rec["error"], rec
        assert "rt_stop" in events and "restore[42]" in events
        FakeFirstPresent.wait = orig_wait
        rt["first"] = None

        # -- capture failure (etl_to_csv raises): attempt record keeps
        #    the error AND the finally still stops trace, terminates the
        #    app and restores the desktop
        events.clear()
        globals()["etl_to_csv"] = (
            lambda e, d: (_ for _ in ()).throw(
                RuntimeError("tracerpt exploded")))
        rec = measure_run("waterui", "w1", 1, cfg, Path("prov.txt"),
                          owned_launch=FakeApp)
        assert rec["error"].startswith("RuntimeError: tracerpt"), rec
        assert "detail" in rec and rec["run"] == 1
        for ev in ("trace_stop", "terminate", "restore[42]"):
            assert ev in events, events
        assert events.index("trace_stop") < events.index("terminate")

        # -- launch failure: no app to terminate but trace/desktop still
        #    cleaned
        events.clear()
        def boom(c, w, lp):
            events.append("launch")
            raise OSError("exe missing")
        rec = measure_run("waterui", "w1", 2, cfg, Path("prov.txt"),
                          owned_launch=boom)
        assert rec["error"].startswith("OSError: exe missing"), rec
        assert "trace_stop" in events and "restore[42]" in events
        assert "terminate" not in events

        # -- missing renderer evidence is a failed attempt, not a
        #    measurement
        events.clear()
        globals()["etl_to_csv"] = lambda e, d: events.append("etl_csv")
        globals()["renderer_evidence"] = (
            lambda *a, **kw: (_ for _ in ()).throw(
                RendererEvidenceError("no proof")))
        rec = measure_run("waterui", "w1", 3, cfg, Path("prov.txt"),
                          owned_launch=FakeApp)
        assert rec["error"].startswith(
            "RendererEvidenceError: no proof"), rec
        assert "terminate" in events and "restore[42]" in events

        # -- a proven software renderer refuses the measurement — it
        #    propagates as SystemExit, not an attempt record
        globals()["renderer_evidence"] = (
            lambda *a, **kw: (_ for _ in ()).throw(
                SoftwareRendererError("SwiftShader")))
        try:
            measure_run("waterui", "w1", 4, cfg, Path("prov.txt"),
                        owned_launch=FakeApp)
            raise AssertionError("software renderer accepted")
        except SystemExit:
            pass
        assert "terminate" in events and "restore[42]" in events

        globals()["renderer_evidence"] = lambda *a, **kw: {
            "name": "Fixture RTX", "device_type": "Gpu",
            "backend": "d3d", "mechanism": "fixture"}

        # -- a failing cleanup step still runs the rest of the
        #    teardown: stop_trace raising skips neither terminate nor
        #    restore, and the record keeps BOTH the operation error and
        #    the cleanup failure
        events.clear()
        globals()["stop_trace"] = (
            lambda n: (_ for _ in ()).throw(
                RuntimeError("etl stop broke")))
        rec = measure_run("waterui", "w1", 5, cfg, Path("prov.txt"),
                          owned_launch=FakeApp)
        assert "etl stop broke" in rec["error"], rec
        assert "cleanup: stop_trace" in rec["error"], rec
        for ev in ("terminate", "restore[42]"):
            assert ev in events, events
        globals()["stop_trace"] = \
            lambda n: events.append("trace_stop")
        # -- and a cleanup failure on an otherwise measured attempt
        #    degrades it to a recorded failure — its numbers are not
        #    trustworthy once teardown broke
        events.clear()
        globals()["restore_windows"] = (
            lambda hw: (_ for _ in ()).throw(
                RuntimeError("desktop restore broke")))
        rec = measure_run("waterui", "w1", 5, cfg, Path("prov.txt"),
                          owned_launch=FakeApp)
        assert "cleanup failure" in rec["error"], rec
        assert "metrics_discarded" in rec and "cleanup_errors" in rec
        assert "terminate" in events
        globals()["stop_trace"] = \
            lambda n: events.append("trace_stop")
        globals()["restore_windows"] = \
            lambda hw: events.append(f"restore{hw}")

        # -- a native query failing inside the sampler thread reaches
        #    the attempt record as a failure, not a silent shortfall
        events.clear()

        class BadPidApp(FakeApp):
            def pids(self):
                if (threading.current_thread()
                        is not threading.main_thread()):
                    raise RuntimeError("job pid query failed")
                return {os.getpid()}

        rec = measure_run("waterui", "w1", 6, cfg, Path("prov.txt"),
                          owned_launch=BadPidApp)
        assert "memory sampler failed" in rec["error"], rec
        assert "terminate" in events and "restore[42]" in events

        # -- a missing startup timestamp is a failed attempt (M1): the
        #    GetProcessTimes call on the owned handle raising propagates
        #    as an error record, not a quiet None
        events.clear()

        class NoClockApp(FakeApp):
            def create_filetime(self):
                raise OSError("GetProcessTimes failed")

        rec = measure_run("waterui", "w1", 7, cfg, Path("prov.txt"),
                          owned_launch=NoClockApp)
        assert "GetProcessTimes" in rec["error"], rec

        # -- zero memory samples inside the measurement window is a
        #    failed attempt (M2), not a steady=None success
        events.clear()
        saved_snap = globals()["process_memory_snapshot"]
        globals()["process_memory_snapshot"] = lambda: {}
        rec = measure_run("waterui", "w1", 8, cfg, Path("prov.txt"),
                          owned_launch=FakeApp)
        globals()["process_memory_snapshot"] = saved_snap
        assert "zero memory samples" in rec["error"], rec

        # -- zero owned presents: no first present exists to anchor the
        #    window on → failed attempt (M3)
        events.clear()
        globals()["parse_frames"] = lambda d, pids: {
            "present_events": 0, "source": None,
            "timestamps_100ns": [], "proc_starts": {},
            "app_pids": sorted(pids)}
        rec = measure_run("waterui", "w1", 9, cfg, Path("prov.txt"),
                          owned_launch=FakeApp)
        assert "no owned present events" in rec["error"], rec
        globals()["parse_frames"] = _fixture_frames

        # -- renderer_evidence fixtures: real parser/dispatch, fixture
        #    data at the OS boundary — restore the real function first
        globals()["renderer_evidence"] = saved["renderer_evidence"]
        globals()["adapter_from_log"] = saved["adapter_from_log"]
        globals()["FirstPresentConsumer"] = saved["FirstPresentConsumer"]
        globals()["wait_for_ready"] = saved["wait_for_ready"]

        # -- the real-time ETW structures match the documented x64
        #    layouts, and only an owned DXGI present Start anchors
        for etw_struct, size in ETW_STRUCT_SIZES.items():
            assert ctypes.sizeof(etw_struct) == size, (
                etw_struct.__name__, ctypes.sizeof(etw_struct))
        dxgi = _EtwGuid.parse(DXGI_PROVIDER_GUID).key()
        assert dxgi == (0xCA11C036, 0x0102, 0x4A2D,
                        bytes.fromhex("A6ADF03CFED5D3C9"))
        mine = {7}.__contains__
        assert first_present_of(dxgi, 42, 1, 7, mine) == "A"
        assert first_present_of(dxgi, 144, 1, 7, mine) == "B"
        assert first_present_of(dxgi, 42, 2, 7, mine) is None   # Stop
        assert first_present_of(dxgi, 42, 1, 8, mine) is None   # not owned
        assert first_present_of(dxgi, 43, 1, 7, mine) is None
        other = _EtwGuid.parse("{802EC45A-1E99-4B83-9920-87C98277BA9D}")
        assert first_present_of(other.key(), 42, 1, 7, mine) is None
        require_same_anchor(1_000_000, "A", 1_005_000, "A")
        for args in ((1_000_000, "A", 1_020_000, "A"),
                     (1_000_000, "C", 1_000_000, "A")):
            try:
                require_same_anchor(*args)
            except RuntimeError:
                pass
            else:
                raise AssertionError(f"anchor mismatch accepted: {args}")
        import tempfile
        hw = {"name": "NVIDIA GeForce RTX 4090", "device_type": "Gpu",
              "backend": "Vulkan"}
        with tempfile.NamedTemporaryFile(
                "w", suffix=".log", delete=False) as lf:
            lf.write('selected wgpu adapter: name: "NVIDIA GeForce RTX '
                     '4090", device_type: Gpu, backend: Vulkan')
            hydro_ok = lf.name
        ev = renderer_evidence("waterui", {"adapter_from_log": True},
                               {1}, Path(hydro_ok))
        assert ev["name"] == hw["name"], ev
        with tempfile.NamedTemporaryFile(
                "w", suffix=".log", delete=False) as lf:
            lf.write("no adapter line here")
            empty = lf.name
        try:
            renderer_evidence("waterui", {"adapter_from_log": True},
                              {1}, Path(empty))
            raise AssertionError("missing log accepted")
        except RendererEvidenceError:
            pass
        with tempfile.NamedTemporaryFile(
                "w", suffix=".log", delete=False) as lf:
            lf.write('selected wgpu adapter: name: "llvmpipe", '
                     'device_type: Cpu, backend: Vulkan')
            soft = lf.name
        try:
            renderer_evidence("waterui", {"adapter_from_log": True},
                              {1}, Path(soft))
            raise AssertionError("software adapter accepted")
        except SoftwareRendererError:
            pass
        # electron: gpuinfo from the process's own log
        info = {"gpuDevice": [{"vendorId": 4318, "deviceId": 9973,
                               "active": True}],
                "auxAttributes": {"glRenderer":
                                  "ANGLE (NVIDIA RTX Direct3D11)"}}
        with tempfile.NamedTemporaryFile(
                "w", suffix=".log", delete=False) as lf:
            lf.write("BENCH_GPUINFO " + json.dumps(info))
            elog = lf.name
        ev = renderer_evidence("electron", {"gpuinfo_log": True},
                               {1}, Path(elog))
        assert "NVIDIA" in ev["name"]
        info["auxAttributes"]["glRenderer"] = "Google SwiftShader"
        with tempfile.NamedTemporaryFile(
                "w", suffix=".log", delete=False) as lf:
            lf.write("BENCH_GPUINFO " + json.dumps(info))
            esoft = lf.name
        try:
            renderer_evidence("electron", {"gpuinfo_log": True},
                              {1}, Path(esoft))
            raise AssertionError("swiftshader accepted")
        except SoftwareRendererError:
            pass
        try:
            renderer_evidence("electron", {"gpuinfo_log": True},
                              {1}, Path(empty))
            raise AssertionError("missing gpuinfo accepted")
        except RendererEvidenceError:
            pass
        # gpu-engine → DXGI LUID attribution (flutter/winui3 path)
        adapters = {(0x1000, 0x0): {"name": "AMD Radeon RX 7900 XTX",
                                    "vendor_id": 0x1002},
                    (0x2000, 0x0): {"name": "Microsoft Basic Render "
                                    "Driver", "vendor_id": 0x1414}}
        ev = renderer_evidence(
            "flutter", {"gpu_engine_evidence": True}, {1234},
            None,
            counter_samples=[(1234, (0x1000, 0), 12.5)],
            luid_map=adapters)
        assert ev["name"] == "AMD Radeon RX 7900 XTX"
        # unrelated pid only → unproven
        try:
            renderer_evidence(
                "flutter", {"gpu_engine_evidence": True}, {1234}, None,
                counter_samples=[(9999, (0x1000, 0), 50.0)],
                luid_map=adapters)
            raise AssertionError("foreign pid engine accepted")
        except RendererEvidenceError:
            pass
        # unmapped luid → ambiguous → failed attempt
        try:
            renderer_evidence(
                "flutter", {"gpu_engine_evidence": True}, {1234}, None,
                counter_samples=[(1234, (0x9999, 0), 5.0)],
                luid_map=adapters)
            raise AssertionError("unmapped luid accepted")
        except RendererEvidenceError:
            pass
        # mixed adapters incl. software → refusal
        try:
            renderer_evidence(
                "winui3", {"gpu_engine_evidence": True}, {1234}, None,
                counter_samples=[(1234, (0x1000, 0), 8.0),
                                 (1234, (0x2000, 0), 3.0)],
                luid_map=adapters)
            raise AssertionError("software-engine mix accepted")
        except SoftwareRendererError:
            pass

        # -- OwnedApp constructor: real __init__ code over fake pywin32
        #    modules — every failure path terminates/releases exactly
        #    what it acquired
        import inspect
        src = inspect.getsource(OwnedApp.__init__)
        # subprocess.CREATE_SUSPENDED does not exist on pinned
        # CPython 3.12 — the launch must go through pywin32
        assert "subprocess.CREATE_SUSPENDED" not in src
        assert "win32con.CREATE_SUSPENDED" in src
        assert "win32job.AssignProcessToJobObject" in src

        # One fake object PER pywin32 module — a call into the wrong
        # module (e.g. win32process.WaitForInputIdle) fails loudly
        # instead of silently hitting a shared stand-in.
        class _Bus:
            events: list = []
            fail: set = set()

        class FakeWin32job:
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x2000
            JobObjectExtendedLimitInformation = 9
            JobObjectBasicProcessIdList = 3

            def CreateJobObject(self, sa, name):
                if "job" in _Bus.fail:
                    raise OSError("CreateJobObject denied")
                _Bus.events.append("job")
                return "job-h"

            def QueryInformationJobObject(self, j, cls):
                if cls == self.JobObjectBasicProcessIdList:
                    return [4242]
                return {"BasicLimitInformation": {"LimitFlags": 0}}

            def SetInformationJobObject(self, j, cls, info):
                _Bus.events.append("setinfo")

            def AssignProcessToJobObject(self, j, p):
                if "assign" in _Bus.fail:
                    raise OSError("assign denied")
                _Bus.events.append("assign")

            def TerminateJobObject(self, j, code):
                _Bus.events.append("job_kill")

        class FakeWin32process:
            class STARTUPINFO:
                def __init__(self):
                    self.dwFlags = 0
                    self.hStdOutput = None
                    self.hStdError = None

            def CreateProcess(self, *a):
                if "create" in _Bus.fail:
                    raise OSError("spawn denied")
                _Bus.events.append("spawn")
                return "ph", "th", 4242, 99

            def ResumeThread(self, t):
                if "resume" in _Bus.fail:
                    raise OSError("resume denied")
                _Bus.events.append("resume")
                return 0

            def GetProcessTimes(self, h):
                # real pywin32 returns PyTime values carrying
                # .timestamp() (epoch seconds)
                class _PyTime:
                    def timestamp(self):
                        return 1_700_000_000.0
                return {"CreationTime": _PyTime()}

            # deliberately NO WaitForInputIdle — that function lives on
            # win32event; a call to it here is the C1 regression

        class FakeWin32event:
            WAIT_TIMEOUT = 0x102

            def WaitForSingleObject(self, h, ms):
                return 0

            def WaitForInputIdle(self, h, ms):
                return 0

        class FakeWin32pipe:
            def CreatePipe(self, sa, sz):
                _Bus.events.append("pipe")
                return "rp", "wp"

        class FakeWin32file:
            # deliberately NO HANDLE_FLAG_INHERIT — the constant lives
            # on win32con in real pywin32 (D1); keeping it here would
            # let the regression pass under the fakes
            class _WinError(Exception):
                winerror = 109  # ERROR_BROKEN_PIPE

            def ReadFile(self, h, n):
                # the child's write end is already closed — end of
                # stream arrives as a broken-pipe error, not b"" (D2)
                raise self._WinError("broken pipe")

        class FakeWin32api:
            def SetHandleInformation(self, h, mask, flags):
                _Bus.events.append("setinherit")

            def TerminateProcess(self, p, code):
                _Bus.events.append("term_root")

            def CloseHandle(self, h):
                _Bus.events.append(f"close:{h}")

        class FakeWin32con:
            CREATE_SUSPENDED = 0x4
            STARTF_USESTDHANDLES = 0x100
            HANDLE_FLAG_INHERIT = 0x1

        class FakeWin32security:
            class SECURITY_ATTRIBUTES:
                def __init__(self):
                    self.bInheritHandle = False

        w32_modules = {
            "win32api": FakeWin32api(), "win32con": FakeWin32con(),
            "win32event": FakeWin32event(), "win32file": FakeWin32file(),
            "win32job": FakeWin32job(), "win32pipe": FakeWin32pipe(),
            "win32process": FakeWin32process(),
            "win32security": FakeWin32security(),
        }
        w32_saved = {n: globals()[n] for n in w32_modules}
        globals().update(w32_modules)
        try:
            c_stub = {"exe_dir": Path("."), "exe": "app.exe", "env": {}}
            import tempfile
            with tempfile.TemporaryDirectory() as td:
                lp = Path(td) / "launch.log"

                # success: suspended spawn -> assign -> resume; the job
                # owns the pid set and kills the tree on terminate
                _Bus.events.clear()
                app = OwnedApp(c_stub, "w1", lp)
                assert app.pid == 4242 and app.pids() == {4242}
                assert app.create_filetime() == int(
                    (1_700_000_000.0 + 11644473600) * 10_000_000)
                seq = [e for e in _Bus.events
                       if e in ("job", "setinfo", "spawn", "assign",
                                "resume")]
                assert seq == ["job", "setinfo", "spawn", "assign",
                               "resume"], seq
                # the fake pipe ends as ERROR_BROKEN_PIPE — D2: the
                # drain treats it as end of stream, not a drain error
                app._reader.join(timeout=5)
                assert app._drain_errors == [], app._drain_errors
                app.terminate()
                assert "job_kill" in _Bus.events
                assert app.log is None

                # CreateJobObject failure: nothing launched, job/log
                # released
                _Bus.fail.add("job")
                _Bus.events.clear()
                try:
                    OwnedApp(c_stub, "w1", lp)
                    raise AssertionError("job failure accepted")
                except OSError:
                    pass
                assert "spawn" not in _Bus.events
                _Bus.fail.discard("job")

                # assign failure: the suspended root is terminated by
                # handle even though it never entered the job, and
                # every acquired handle is released
                _Bus.fail.add("assign")
                _Bus.events.clear()
                try:
                    OwnedApp(c_stub, "w1", lp)
                    raise AssertionError("assign failure accepted")
                except OSError:
                    pass
                for ev in ("term_root", "close:job-h", "close:ph",
                           "close:th"):
                    assert ev in _Bus.events, _Bus.events
                _Bus.fail.discard("assign")

                # resume failure: same ownership contract
                _Bus.fail.add("resume")
                _Bus.events.clear()
                try:
                    OwnedApp(c_stub, "w1", lp)
                    raise AssertionError("resume failure accepted")
                except OSError:
                    pass
                assert "term_root" in _Bus.events
                _Bus.fail.discard("resume")

                # spawn failure: no root to kill, job + pipe released
                _Bus.fail.add("create")
                _Bus.events.clear()
                try:
                    OwnedApp(c_stub, "w1", lp)
                    raise AssertionError("spawn failure accepted")
                except OSError:
                    pass
                assert "term_root" not in _Bus.events
                assert "close:job-h" in _Bus.events
                _Bus.fail.discard("create")
        finally:
            globals().update(w32_saved)

        # -- DXGI interface declarations carry the OFFICIAL IIDs and
        #    the comtypes-computed vtable layout; IsCurrent is declared
        #    BOOL, not HRESULT
        if IDXGIFactory1 is not None:
            assert str(IDXGIObject._iid_).lower() == \
                "{aec22fb8-76f3-4639-9be0-28eb43a67a2e}"
            assert str(IDXGIAdapter._iid_).lower() == \
                "{2411e7e1-12ac-4ccf-bd14-9798e8534dc0}"
            assert str(IDXGIFactory1._iid_).lower() == \
                "{770aae78-f26f-4dba-a829-253c83d1b387}"
            assert len(IDXGIObject._methods_) == 4
            assert len(IDXGIFactory._methods_) == 5
            assert len(IDXGIAdapter._methods_) == 3
            assert len(IDXGIFactory1._methods_) == 2
            names = [m.name for m in IDXGIAdapter._methods_]
            assert names[1] == "GetDesc", names  # vtable slot 8
            assert IDXGIFactory1._methods_[0].name == \
                "EnumAdapters1"  # vtable slot 12

        # -- GPU Engine instance names carry luid_0x<High>_0x<Low> —
        #    HighPart first (H1); the parse must map a fixture row to
        #    the (low, high) key dxgi_adapters() produces
        saved_run = subprocess.run
        try:
            def fake_run(cmd, **kw):
                class R:
                    stdout = (
                        "\\\\x\\gpu engine(pid_4242_luid_0x00000001_"
                        "0x00002000_engtype_3d)\\utilization percentage"
                        "|12.5\n")
                return R()
            subprocess.run = fake_run
            hits = gpu_engine_counters()
            assert hits == [(4242, (0x2000, 0x1), 12.5)], hits
        finally:
            subprocess.run = saved_run

        # -- attempt accounting + rep floor
        good = [{"run": i} for i in range(3)]
        assert repetition_check("x", "w1", good, 5, 5) is not None
        assert repetition_check(
            "x", "w1", [{"run": i} for i in range(5)], 5, 6) is None
        st = stats_of([10, 11, 12, None])
        assert st["n"] == 3 and st["median"] == 11

        # -- report refuses incomplete data, renders complete data
        import importlib.util
        spec = importlib.util.spec_from_file_location(
            "report", ROOT / "report.py")
        rep_mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(rep_mod)
        cell_data = {"startup_ms": st, "memory": {}, "frame_rate": {},
                     "runs_succeeded": 3, "runs_attempted": 5,
                     "failures": [{"run": 3, "error": "boom"}]}
        inc = {"repetitions": 5, "repetitions_met": False,
               "results": {"waterui": {"workloads": {"w1": cell_data}}}}
        try:
            rep_mod.generate(inc)
            raise AssertionError("incomplete report accepted")
        except SystemExit:
            pass
        ok = {"repetitions": 5, "repetitions_met": True,
              "results": {"waterui": {
                  "workloads": {"w1": {**cell_data, "runs_succeeded": 5,
                                       "failures": []}}}},
              "contestants": {"waterui": {}}, "machine": {}}
        out = rep_mod.generate(ok)
        assert "5/5" in out
    finally:
        globals().update(saved)
    print("run.py self-test OK")


def _native_check() -> int:
    """CPU-boundary native checks on a real Windows host — the parts of
    the runner that must work natively, exercised against a small owned
    test process tree. No contestant build, no measurement, no GPU
    required; the software-GPU-refusing measurement path is never run.
    Prints PASS/FAIL/SKIP per item and exits non-zero on any FAIL.

    Covers the native-verification checklist from the ab90 review:
    1 pywin32 attribute surface; 2 OwnedApp suspended→job→resume with a
      real grandchild, stdout capture, tree termination, forced-assign
      failure; 3 WaitForInputIdle on a GUI-subsystem exe; 4 Electron GPU
      process inside the job (SKIPs without a build); 5 dxgi_adapters
      enumeration stability; 6 GPU Engine LUID ↔ DXGI AdapterLuid;
      7 create_filetime FILETIME ordering vs the ETW clock;
      8 process_memory_snapshot NTSTATUS/buffer handling vs psutil;
      9 logman/tracerpt dump columns + owned-pid rows; 10 minimize/
      restore round-trip under injected cleanup failure.
    """
    if platform.system() != "Windows":
        print("--native-check needs a Windows host", file=sys.stderr)
        return 2
    results: list[tuple[int, str, str]] = []

    def rec(item: int, status: str, detail: str = "") -> None:
        results.append((item, status, detail))
        print(f"[{status:>4}] item {item}: {detail}")

    def guard(item: int, fn) -> None:
        try:
            fn()
        except Skip as s:
            rec(item, "SKIP", str(s))
        except Exception as e:
            rec(item, "FAIL", f"{type(e).__name__}: {e}")

    class Skip(Exception):
        pass

    # 1 — pywin32 attribute surface the runner calls
    def i1():
        for mod, names in (
            (win32con, ["CREATE_SUSPENDED", "STARTF_USESTDHANDLES",
                        "HANDLE_FLAG_INHERIT"]),
            (win32event, ["WaitForInputIdle", "WaitForSingleObject",
                          "WAIT_TIMEOUT"]),
            (win32file, ["ReadFile"]),
            (win32pipe, ["CreatePipe"]),
            (win32api, ["CloseHandle", "TerminateProcess",
                        "SetHandleInformation"]),
            (win32security, ["SECURITY_ATTRIBUTES"]),
            (win32process, ["CreateProcess", "STARTUPINFO",
                            "ResumeThread", "GetProcessTimes"]),
            (win32job, ["CreateJobObject", "QueryInformationJobObject",
                        "SetInformationJobObject",
                        "AssignProcessToJobObject", "TerminateJobObject",
                        "JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE",
                        "JobObjectExtendedLimitInformation",
                        "JobObjectBasicProcessIdList"]),
        ):
            for n in names:
                if getattr(mod, n, None) is None:
                    raise AssertionError(f"{mod.__name__}.{n} missing")
        rec(1, "PASS", "all pywin32 attributes resolve")

    td = Path(tempfile.mkdtemp(prefix="bench-native-"))

    def spawn_test_app(log: Path) -> OwnedApp:
        """The real OwnedApp over a scripted test process: python
        spawning a sleeping grandchild and printing a ready line."""
        script = td / "child.py"
        script.write_text(
            "import subprocess,sys,time\n"
            "subprocess.Popen([sys.executable, '-c', "
            "'import time;time.sleep(300)'])\n"
            "print('CHILD-READY', flush=True)\n"
            "time.sleep(300)\n")
        return OwnedApp(
            {"exe_dir": Path(sys.executable).parent,
             "exe": Path(sys.executable).name,
             "args": f'"{script}"', "env": {}},
            "w1", log)

    # 2 — OwnedApp real lifecycle: suspended→assign→resume, grandchild
    #     inside the job's pid set, stdout through the pipe, terminate
    #     kills the whole tree, forced assign failure leaves no root
    def i2():
        log = td / "child.log"
        app = spawn_test_app(log)
        try:
            deadline = time.monotonic() + 10
            pids = set()
            while time.monotonic() < deadline:
                pids = app.pids()
                if len(pids) >= 2:
                    break
                time.sleep(0.2)
            assert app.pid in pids, (app.pid, pids)
            assert len(pids) >= 2, f"grandchild not in job: {pids}"
            app.create_filetime()
        finally:
            app.terminate()
        for p in pids:
            try:
                psutil.Process(p)
            except psutil.NoSuchProcess:
                continue
            raise AssertionError(f"pid {p} survived job termination")
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if "CHILD-READY" in log.read_text(errors="replace"):
                break
            time.sleep(0.1)
        else:
            raise AssertionError("child stdout never reached the pipe")
        # forced assign failure: a process already inside another job
        # (non-nested hosts) makes AssignProcessToJobObject raise — the
        # constructor's terminate-the-root path depends on it raising
        other = win32job.CreateJobObject(None, "bench-native-other")
        spawn = subprocess.Popen(
            [sys.executable, "-c", "import time;time.sleep(60)"])
        try:
            try:
                win32job.AssignProcessToJobObject(
                    other, int(spawn._handle))
                win32job.AssignProcessToJobObject(
                    other, int(spawn._handle))
            except Exception as e:
                rec(2, "PASS",
                    f"tree owned {sorted(pids)}, terminate killed it; "
                    f"second-job assign raises: {e}")
                return
            rec(2, "PASS",
                f"job nesting permitted on this host; tree owned "
                f"{sorted(pids)} and terminate killed it")
        finally:
            try:
                win32job.TerminateJobObject(other, 1)
                win32api.CloseHandle(other)
            except Exception:
                pass
            spawn.kill()

    # 3 — WaitForInputIdle on an owned process that creates a real GUI
    #     test window and pumps a message loop (input-idle is signalled
    #     only once the process blocks in its message loop — a sleeping
    #     process never reaches it). The console-subsystem case is kept
    #     as an expected failure: WaitForInputIdle legitimately returns
    #     WAIT_FAILED on a process with no message loop.
    GUI_PUMP = """
import ctypes
import ctypes.wintypes as W

u = ctypes.windll.user32
k = ctypes.windll.kernel32
WNDPROC = ctypes.WINFUNCTYPE(ctypes.c_long, W.HWND, W.UINT,
                             W.WPARAM, W.LPARAM)
u.DefWindowProcW.restype = ctypes.c_long
def _proc(hwnd, msg, wparam, lparam):
    return u.DefWindowProcW(hwnd, msg, wparam, lparam)
_proc_c = WNDPROC(_proc)
class WNDCLASSW(ctypes.Structure):
    _fields_ = [
        ("style", W.UINT), ("lpfnWndProc", WNDPROC),
        ("cbClsExtra", ctypes.c_int), ("cbWndExtra", ctypes.c_int),
        ("hInstance", W.HINSTANCE), ("hIcon", W.HANDLE),
        ("hCursor", W.HANDLE), ("hbrBackground", W.HANDLE),
        ("lpszMenuName", W.LPCWSTR), ("lpszClassName", W.LPCWSTR),
    ]
cls = WNDCLASSW(0, _proc_c, 0, 0, k.GetModuleHandleW(None),
                None, None, None, None, "BenchGuiProbe")
u.RegisterClassW(ctypes.byref(cls))
hwnd = u.CreateWindowExW(0, "BenchGuiProbe", "probe", 0xCF0000,
                         0, 0, 100, 100, None, None, cls.hInstance, None)
u.ShowWindow(hwnd, 5)
u.UpdateWindow(hwnd)
msg = W.MSG()
while u.GetMessageW(ctypes.byref(msg), None, 0, 0) != 0:
    u.TranslateMessage(ctypes.byref(msg))
    u.DispatchMessageW(ctypes.byref(msg))
"""

    def i3():
        script = td / "gui_pump.py"
        script.write_text(GUI_PUMP)
        # pythonw.exe is the GUI-subsystem interpreter: WaitForInputIdle
        # refuses (1471) any console-subsystem image, message loop or not
        app = OwnedApp(
            {"exe_dir": Path(sys.executable).parent,
             "exe": "pythonw.exe",
             "args": f'"{script}"', "env": {}},
            "w1", td / "gui.log")
        try:
            rc = win32event.WaitForInputIdle(app.hproc, 15000)
            if rc != 0:
                raise AssertionError(
                    "WaitForInputIdle on an owned message-loop process "
                    f"returned {rc}")
        finally:
            app.terminate()
        # console-subsystem expected failure: a process that never
        # pumps a message loop must NOT report input-idle — pywin32
        # raises error 1471 rather than returning WAIT_FAILED
        sleeping = OwnedApp(
            {"exe_dir": Path(sys.executable).parent,
             "exe": Path(sys.executable).name,
             "args": '-c "import time;time.sleep(20)"', "env": {}},
            "w1", td / "console.log")
        try:
            try:
                win32event.WaitForInputIdle(sleeping.hproc, 10000)
            except Exception as e:
                if getattr(e, "winerror", None) != 1471:
                    raise AssertionError(
                        "WaitForInputIdle on the console sleeper raised "
                        f"an unexpected error: {e!r}") from e
            else:
                raise AssertionError(
                    "console-subsystem sleeper unexpectedly reported "
                    "input-idle")
        finally:
            sleeping.terminate()
        rec(3, "PASS",
            "WaitForInputIdle signalled on owned GUI test window; "
            "console sleeper raised error 1471 as expected")

    # 4 — Electron renderer/GPU helpers inside the job + GPUINFO log
    def i4():
        cand = (ROOT.parent / "apps" / "electron" / "dist"
                / "bench-electron-win32-x64" / "bench-electron.exe")
        if not cand.exists():
            raise Skip("no Electron build on this host")
        app = OwnedApp(
            {"exe_dir": cand.parent, "exe": cand.name, "env": {}},
            "w1", td / "electron.log")
        try:
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                if len(app.pids()) >= 3:
                    break
                time.sleep(0.5)
            assert len(app.pids()) >= 3, (
                f"renderer/GPU helpers escaped the job: {app.pids()}")
            if "BENCH_GPUINFO" not in (td / "electron.log").read_text(
                    errors="replace"):
                raise AssertionError("no BENCH_GPUINFO on stdout")
        finally:
            app.terminate()
        rec(4, "PASS", "electron helpers in job; GPUINFO logged")

    # 5 — dxgi_adapters stability over 100 enumerations
    def i5():
        first = dxgi_adapters()
        for _ in range(100):
            now = dxgi_adapters()
            assert set(now) == set(first), "adapter set changed"
        names = "; ".join(a["name"] for a in first.values())
        assert "Basic Render Driver" in names or any(
            not is_software_gpu(a["name"]) for a in first.values()), names
        rec(5, "PASS", f"100 enumerations stable: {names}")

    # 6 — GPU Engine instance LUID ↔ DXGI AdapterLuid (H1 order)
    def i6():
        adapters = dxgi_adapters()
        hits = gpu_engine_counters()
        if not hits:
            raise Skip("no live GPU engines to attribute")
        unknown = [l for _, l, _ in hits if l not in adapters]
        assert not unknown, f"engine LUIDs unmapped: {unknown}"
        rec(6, "PASS",
            f"{len(hits)} engine instances map to enumerated adapters")

    # 7 — create_filetime FILETIME ordering vs the ETW clock
    def i7():
        app = spawn_test_app(td / "clk.log")
        try:
            ft = app.create_filetime()
            now = filetime_now()
            assert 0 < now - ft < 60 * 10_000_000, (
                f"creation FILETIME off by {now - ft} ticks")
        finally:
            app.terminate()
        rec(7, "PASS", "create_filetime is plausible FILETIME")

    # 8 — process_memory_snapshot NTSTATUS/buffer handling vs psutil
    def i8():
        snap = process_memory_snapshot()
        assert os.getpid() in snap, "own pid missing from snapshot"
        mine_ps = psutil.Process().memory_info().private
        mine_nt = snap[os.getpid()]["private_bytes"]
        assert abs(mine_nt - mine_ps) < 64 << 20, (
            f"nt {mine_nt} vs psutil {mine_ps}")
        try:
            process_memory_snapshot(buffer_bytes=64)
        except OSError:
            pass
        else:
            raise AssertionError("tiny buffer silently accepted")
        rec(8, "PASS", "NTSTATUS clean; matches psutil; small-buffer "
                       "errors propagate")

    # 9 — logman/tracerpt dump columns + owned-pid process rows
    def i9():
        etl = td / "nc.etl"
        prov = td / "providers.txt"
        write_providers_file(prov)
        name = f"bench1262_native_{os.getpid()}"
        start_trace(name, etl, prov)
        app = spawn_test_app(td / "etw.log")
        try:
            time.sleep(2)
        finally:
            app.terminate()
        stop_trace(name)
        dump = td / "nc.csv"
        etl_to_csv(etl, dump)
        frames = parse_frames(dump, set())
        assert dump.exists() and dump.stat().st_size > 0
        assert frames["proc_starts"], "no Kernel-Process rows parsed"
        rec(9, "PASS", "trace dump parses; process-start rows present")

    # 10 — minimize/restore round-trip only touches owned changes
    def i10():
        before = minimize_other_windows()
        restore_windows(before)
        rec(10, "PASS",
            f"minimized {len(before)} windows and restored them")

    guard(1, i1)
    for item, fn in ((2, i2), (3, i3), (4, i4), (5, i5), (6, i6),
                     (7, i7), (8, i8), (9, i9), (10, i10)):
        guard(item, fn)


    fails = [r for r in results if r[1] == "FAIL"]
    print(f"native-check: {len(results)} items, "
          f"{sum(1 for r in results if r[1] == 'PASS')} pass, "
          f"{sum(1 for r in results if r[1] == 'SKIP')} skip, "
          f"{len(fails)} fail")
    return 1 if fails else 0


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--reps", type=int, default=None)
    ap.add_argument("--apps", nargs="*", default=list(CONTESTANTS))
    ap.add_argument("--workloads", nargs="*", default=["w1", "w2", "w3", "w4"])
    ap.add_argument("--skip-build", action="store_true")
    ap.add_argument("--out", default=None)
    ap.add_argument("--self-test", action="store_true",
                    help="run the CPU-only control-logic checks and exit")
    ap.add_argument("--native-check", action="store_true",
                    help="run the Windows-native boundary checks "
                    "(pywin32/comtypes/ETW) against a test process tree "
                    "and exit — no GPU or contestant build needed")
    ap.add_argument("--development", action="store_true",
                    help="mark the run development-only in its output "
                    "(software-GPU measurement is refused regardless)")
    args = ap.parse_args()
    if args.self_test:
        _self_test()
        return
    if args.native_check:
        raise SystemExit(_native_check())

    manifest = tomllib.loads((ROOT / "manifest.toml").read_text())
    reps = args.reps or manifest["runner"]["repetitions"]
    TRACE_DIR.mkdir(parents=True, exist_ok=True)
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    providers_file = TRACE_DIR / "etw_providers.txt"
    write_providers_file(providers_file)

    out = Path(args.out) if args.out else RESULTS_DIR / (
        f"windows-{datetime.now(timezone.utc).strftime('%Y%m%d-%H%M%S')}.json"
    )

    # Resume: if the output file already exists, keep its earlier results so a
    # failed run can be continued with --apps <remaining>. An unreadable
    # file is refused, not silently dropped — the run it came from was
    # evidence.
    previous_results = {}
    previous_meta: dict = {}
    if args.out and out.exists():
        try:
            previous_meta = json.loads(out.read_text())
            previous_results = previous_meta.get("results", {})
        except Exception as e:
            raise SystemExit(
                f"cannot resume from {out}: unreadable ({e}); fix or "
                "remove the file — dropping it silently would erase "
                "evidence of the earlier run")

    machine = machine_spec()
    if previous_meta:
        # A resumed file merges cells into this run — they must carry
        # the same machine and the same source identity, never an
        # older HEAD's or another host's numbers
        if previous_meta.get("machine") != machine or (
                previous_meta.get("waterui_head")
                != toolchain.checkout_head()):
            raise SystemExit(
                f"cannot merge {out}: it was measured on a different "
                "machine or a different checkout HEAD — keep the files "
                "separate or re-measure")
    if machine["gpu_software"]:
        # there is no software-GPU measurement route: WARP/SwiftShader/
        # Basic Render numbers are not evidence under any flag
        print(
            f"refusing to measure: the GPU adapter is a software "
            f"rasterizer ({machine['gpu']}), so frame-time and memory "
            "numbers would not be evidence. Re-run on a host with a "
            "hardware GPU.",
            file=sys.stderr,
        )
        sys.exit(2)
    # every rep brings its contestant to the foreground — a host that
    # would refuse SetForegroundWindow fails here, before any cell
    require_foreground_eligible()

    limitations = [
        "PresentMon 2.5.1 captures no presents for WinUI 3 (composition-"
        "path presents sit below its keyword mask) or Electron (CDD blit "
        "path on this adapter); the runner consumes the underlying "
        "DXGI/DxgKrnl ETW events directly, uniformly for all four.",
        "Adapter identity per row is per-run evidence, not host "
        "inventory: hydrolysis's own adapter log (WaterUI), Electron's "
        "app.getGPUInfo from the measured process, or the owned pid "
        "set's GPU-engine counters resolved to adapter LUIDs through "
        "DXGI. A rep with no such evidence is recorded as a failed "
        "attempt and never counted.",
    ]

    results = {
        "schema_version": 1,
        "issue": manifest["meta"]["issue"],
        "platform": "windows",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "development_only": bool(args.development),
        "renderer_evidence": {
            k: ("hydrolysis adapter log" if v.get("adapter_from_log")
                else "app.getGPUInfo" if v.get("gpuinfo_log")
                else "gpu-engine → DXGI LUID")
            for k, v in CONTESTANTS.items()
        },
        "repetitions": reps,
        "machine": machine,
        "toolchain": manifest["toolchain"],
        "frameworks": manifest["frameworks"],
        "waterui_head": toolchain.checkout_head(),
        "measurement_method": manifest["measurement"],
        "contestants": {
            k: {
                "title": v["title"],
                "exe": v["exe"],
                "adapter": v["adapter"],
                "env": v["env"],
                **(
                    {"unsupported_workloads": v["unsupported_workloads"]}
                    if v.get("unsupported_workloads")
                    else {}
                ),
            }
            for k, v in CONTESTANTS.items()
        },
        "workloads": WORKLOAD_NAMES,
        "results": dict(previous_results),
        "limitations": limitations,
    }

    def dump() -> None:
        out.write_text(json.dumps(results, indent=2))

    for key in args.apps:
        c = CONTESTANTS[key]
        if not args.skip_build:
            print(f"=== building {key} ===", flush=True)
            # every bootstrap step (flutter create, npm ci, water fetch)
            # runs on a clean tree and must leave it clean
            with toolchain.tracked_tree_unchanged(f"windows build {key}"):
                builders(manifest)[key]()
        cres = results["results"].setdefault(key, {})
        cres["package_size"] = package_size(c["exe_dir"])
        cres["workloads"] = previous_results.get(key, {}).get("workloads", {})
        for wl in args.workloads:
            unsupported = c.get("unsupported_workloads", {}).get(wl)
            if unsupported:
                cres["workloads"][wl] = {"unsupported": unsupported}
                print(f"=== {key} {wl} skipped: unsupported ===", flush=True)
                continue
            attempts = []
            for rep in range(reps):
                print(f"=== {key} {wl} rep {rep + 1}/{reps} ===", flush=True)
                # measure_run always returns an attempt record — the
                # metrics, or the error it failed with; no rep is
                # silently retried or dropped
                attempts.append(
                    measure_run(key, wl, rep, manifest, providers_file))
            runs = [r for r in attempts if "error" not in r]
            failures = [r for r in attempts if "error" in r]
            for f in failures:
                print(f"    rep {f['run']} failed: {f['error']}",
                      flush=True)
            detected = next(
                (r.get("adapter_detected") for r in runs
                 if r.get("adapter_detected")),
                None,
            )
            if detected is not None:
                # Label the row from what hydrolysis actually selected — a
                # hardware-GPU host labels itself through the same field.
                results["contestants"][key]["adapter"] = adapter_label(
                    c, detected
                )
                results["contestants"][key]["adapter_detected"] = detected
            shortfall = repetition_check(key, wl, runs, reps,
                                         len(attempts))
            cres["workloads"][wl] = {
                "runs_succeeded": len(runs),
                "runs_attempted": len(attempts),
                "failures": [
                    {"run": f["run"], "error": f["error"],
                     "detail": f.get("detail")} for f in failures
                ],
                "startup_ms": stats_of([r["startup_ms"] for r in runs]),
                "memory": {
                    "steady_private_ws_mb": stats_of(
                        [r["memory"]["steady_private_ws_mb"] for r in runs]
                    ),
                    "peak_private_ws_mb": stats_of(
                        [r["memory"]["peak_private_ws_mb"] for r in runs]
                    ),
                    "private_bytes_mb": stats_of(
                        [r["memory"]["private_bytes_mb"] for r in runs]
                    ),
                    "process_count": max(
                        (r["memory"]["process_count"] or 0) for r in runs
                    )
                    if runs
                    else 0,
                },
                "frame_rate": {
                    "fps": stats_of([r["frame_rate"].get("fps") for r in runs]),
                    "frame_ms_p50": stats_of(
                        [r["frame_rate"].get("frame_ms_p50") for r in runs]
                    ),
                    "frame_ms_p90": stats_of(
                        [r["frame_rate"].get("frame_ms_p90") for r in runs]
                    ),
                    "frame_ms_p99": stats_of(
                        [r["frame_rate"].get("frame_ms_p99") for r in runs]
                    ),
                    "missed_vsyncs": stats_of(
                        [r["frame_rate"].get("missed_vsyncs") for r in runs]
                    ),
                    "present_events": [
                        r["frame_rate"]["present_events"] for r in runs
                    ],
                    "source": [r["frame_rate"]["source"] for r in runs],
                    "source_label": [
                        r["frame_rate"]["source_label"] for r in runs
                    ],
                },
            }
            if shortfall:
                # a cell below the rep floor rejects the measurement —
                # the attempt records are kept in the results file
                results["repetitions_met"] = False
                out_inc = out.with_name(
                    out.stem + "-INCOMPLETE" + out.suffix)
                out_inc.write_text(json.dumps(results, indent=2))
                print(
                    "repetition requirement not met: "
                    f"{shortfall} — records kept in {out_inc}; "
                    "refusing to emit a measurement",
                    file=sys.stderr,
                )
                sys.exit(2)
        dump()  # save partial results after every contestant

    # cells carried over from a resumed run are checked against THIS
    # run's floor — a stale cell below the rep count cannot ride along
    stale = [
        f"{name}/{wl}"
        for name, entry in results["results"].items()
        for wl, w in (entry.get("workloads") or {}).items()
        if "unsupported" not in w
        and (w.get("runs_succeeded") or 0) < reps
    ]
    if stale:
        results["repetitions_met"] = False
        out_inc = out.with_name(out.stem + "-INCOMPLETE" + out.suffix)
        out_inc.write_text(json.dumps(results, indent=2))
        print(
            "carried cells below the rep floor: " + ", ".join(stale)
            + f" — records kept in {out_inc}; refusing to emit a "
            "measurement",
            file=sys.stderr,
        )
        sys.exit(2)
    results["repetitions_met"] = True
    dump()

    out.write_text(json.dumps(results, indent=2))
    print(f"results -> {out}", flush=True)


if __name__ == "__main__":
    main()
