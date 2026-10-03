#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = ["tomli-w==1.2.0"]
# ///
"""Measurement driver for water-rs/apple-backend#282: Swift backend vs the
merged Rust/objc2 backend.

Reads benches/issue-282/manifest.json. Commands:

    setup        prepare reusable pinned checkouts, verified CLI installs and
                 a compatible owned simulator; no run inputs are frozen.
    finalize-inputs
                 before scaffold, reconcile owned clean new-side inputs
                 and verified CLI receipts, then freeze run inputs.
    scaffold     <side>: create the fresh app, stage the identical form app,
                 pin manifests, materialize + fingerprint lockfiles.
    parity       hard gate: identical view source both sides, lockfiles
                 materialized.
    measure      <side> <subject> <leg> --platform P --sample N: one bounded
                 leg; appends one JSON record to results/<side>__<subject>.jsonl.
                 Exits nonzero on any failure or diagnosis.
    plan         ordered command list for a full protocol pass.
    report       aggregate into report.json + report.md; emits a ratio only
                 when both sides have exactly samples_per_leg valid samples
                 and inputs are coherently locked. Exits nonzero otherwise.

Scope/build honesty: the old CLI's `water build` compiles only the Rust
library (native link deferred to package/run); the new CLI's builds the
managed native backend project too. Records label this via build_scope so
the two legs are never compared as the same operation. Package legs always
include the full native build. Every subprocess is bounded (<= 30 min/step),
readiness is event-driven on the log-stream pipe, and the launch marker is
scoped to the measured PID — no cross-process log fallback.
"""

import argparse
import copy
import difflib
import glob
import hashlib
import json
import math
import os
import pwd
import re
import select
import shutil
import signal
import shlex
import statistics
import subprocess
import sys
import time
import tomllib
import tomli_w
from contextlib import ExitStack
from dataclasses import dataclass
from datetime import datetime
from zoneinfo import ZoneInfo
from pathlib import Path

HERE = Path(__file__).resolve().parent
MANIFEST_PATH = HERE / "manifest.json"
# ROOT must live inside the dedicated user's home — validated at runtime.
ROOT = Path(os.environ.get("BENCH282_ROOT", HERE / ".run")).resolve()
RESULTS = ROOT / "results"
LOGS = ROOT / "logs"
STATE_PATH = ROOT / "state.json"
MAX_STEP_S = 1800  # the <=30-minute bound applies to every step


class BenchError(RuntimeError):
    pass


def die(msg):
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(2)


def load_manifest():
    return json.loads(MANIFEST_PATH.read_text())


def load_state():
    return json.loads(STATE_PATH.read_text()) if STATE_PATH.exists() else {}


def save_state(state):
    STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
    STATE_PATH.write_text(json.dumps(state, indent=2) + "\n")


def now_eastern():
    return datetime.now(ZoneInfo("America/New_York")).isoformat()


def bounded(timeout_s, step):
    if not 0 < timeout_s <= MAX_STEP_S:
        raise BenchError(f"{step}: timeout {timeout_s}s exceeds {MAX_STEP_S}s")
    return timeout_s


def shlex_join(argv):
    return shlex.join([str(a) for a in argv])


def run(argv, timeout_s, env=None, cwd=None, log_file=None, capture=False):
    """Run argv bounded by timeout_s; kill the whole process group on expiry.
    Returns a record fragment; nonzero exit is data, not an exception."""
    timeout_s = bounded(timeout_s, str(argv[0]))
    started_wall = now_eastern()
    t0 = time.monotonic_ns()
    log_fh = open(log_file, "w") if log_file else subprocess.DEVNULL
    try:
        proc = subprocess.Popen(
            argv, cwd=cwd, env=env,
            stdout=subprocess.PIPE if capture else log_fh,
            stderr=subprocess.STDOUT if capture else log_fh,
            start_new_session=True, text=True)
        timed_out = False
        try:
            out, _ = proc.communicate(timeout=timeout_s)
        except subprocess.TimeoutExpired:
            timed_out = True
            stop_process(proc)
            out, _ = proc.communicate()
        finally:
            stop_process(proc)
    finally:
        if log_file:
            log_fh.close()
    wall_ms = (time.monotonic_ns() - t0) // 1_000_000
    return {
        "argv": argv, "cwd": str(cwd) if cwd else None,
        "started_at": started_wall, "wall_ms": wall_ms,
        "exit_code": proc.returncode, "timed_out": timed_out,
        "timeout_s": timeout_s,
        "log_file": str(log_file) if log_file else None,
        "stdout": out if capture else None,
    }


def run_or_die(argv, timeout_s, step, env=None, cwd=None, log_file=None):
    rec = run(argv, timeout_s, env=env, cwd=cwd, log_file=log_file)
    if rec["timed_out"] or rec["exit_code"] != 0:
        raise BenchError(
            f"{step}: timed_out={rec['timed_out']} exit={rec['exit_code']} "
            f"({shlex_join(argv)})\n{log_tail(rec['log_file'])}")
    return rec


def checked_output(argv, timeout_s=60):
    rec = run(argv, timeout_s, capture=True)
    if not record_ok(rec):
        raise BenchError(f"command failed: {shlex_join(argv)}\n{rec['stdout']}")
    return (rec["stdout"] or "").strip()


def log_tail(log_file, lines=40):
    if not log_file or not Path(log_file).exists():
        return ""
    return "\n".join(Path(log_file).read_text(errors="replace")
                     .splitlines()[-lines:])


def timeout_for(manifest, leg):
    env_key = f"BENCH282_TIMEOUT_{leg.upper().replace('-', '_')}"
    if env_key in os.environ:
        return bounded(int(os.environ[env_key]), leg)
    return bounded(manifest["legs"].get(leg, {}).get(
        "timeout_s", manifest["timeouts"]["default_leg"]), leg)


def measured_env(manifest):
    env = dict(os.environ)
    for key, value in manifest["measured_env_overrides"].items():
        if value is None:
            env.pop(key, None)
        else:
            env[key] = value
    return env


def probe_toolchain(manifest):
    snap = {}
    for argv in manifest["toolchain_probes"]:
        label = " ".join(argv[:1])
        snap[label] = checked_output(argv)
    return snap


def resolve_pin(side_cfg, name, side):
    """Exact commit for one input: env override wins, then the manifest.
    A null pin is a required input — never a moving ref."""
    pin = side_cfg[name]
    env_key = pin.get("sha_env") or f"BENCH282_{side.upper()}_{name.upper()}_SHA"
    sha = os.environ.get(env_key, pin.get("sha"))
    if sha is None:
        raise BenchError(
            f"{side}.{name}: no pinned SHA — {pin.get('pending', 'required input')}. "
            f"Supply it via manifest or env {env_key} (40-hex).")
    if not re.fullmatch(r"[0-9a-fA-F]{40}", sha):
        raise BenchError(f"{side}.{name}: {sha!r} is not a 40-hex commit")
    if pin.get("sha") and sha.lower() != pin["sha"]:
        raise BenchError(f"{side}.{name}: fixed manifest pin cannot be overridden")
    if sha.lower() in pin.get("rejected_shas", []):
        raise BenchError(f"{side}.{name}: rejected placeholder SHA {sha}")
    return sha.lower()


def expand(template, project=None, scheme=None, home=None):
    s = str(template).replace("{home}", str(home or Path.home()))
    if project is not None:
        s = s.replace("{project}", str(project))
        s = s.replace("{project_slug}", str(project).lstrip("/"))
    if scheme is not None:
        s = s.replace("{scheme}", scheme)
    return s


def du_bytes(path):
    """Allocated bytes via du -sk. Missing path -> 0; failed measurement
    -> error, never a silent zero."""
    if not Path(path).exists():
        return 0
    rec = run(["du", "-sk", str(path)], 300, capture=True)
    if rec["timed_out"] or rec["exit_code"] != 0:
        raise BenchError(f"du -sk {path}: timed_out={rec['timed_out']} "
                         f"exit={rec['exit_code']}")
    try:
        return int((rec["stdout"] or "").split()[0]) * 1024
    except (IndexError, ValueError) as exc:
        raise BenchError(
            f"du -sk {path}: unparseable {rec['stdout']!r}") from exc


def water_bin(side):
    return ROOT / "toolchains" / side / "bin" / "water"


def waterui_dir(side):
    return ROOT / "checkouts" / side / "waterui"


def backend_dir(side):
    return ROOT / "checkouts" / side / "apple-backend"


def project_dir(manifest, side, subject_name):
    subject = manifest["subjects"][subject_name]
    return ROOT / "apps" / side / subject["folder_name"]


def record_path(side, subject_name):
    RESULTS.mkdir(parents=True, exist_ok=True)
    return RESULTS / f"{side}__{subject_name}.jsonl"


def write_record(side, subject_name, record):
    path = record_path(side, subject_name)
    with open(path, "a") as fh:
        fh.write(json.dumps(record) + "\n")
    print(f"recorded -> {path}")


def iter_records():
    for path in sorted(RESULTS.glob("*.jsonl")):
        for line in path.read_text().splitlines():
            yield json.loads(line)


def record_ok(rec):
    """A sample counts only when the run completed cleanly: no timeout, no
    pre-run failure, zero exit, and no recorded diagnosis."""
    return (not rec.get("timed_out")
            and not rec.get("failed_before_run")
            and rec.get("exit_code") == 0
            and not rec.get("diagnosis"))


def require_predecessor(state, side, subject, platform, sample, leg):
    expected = [side, subject, leg, platform, sample]
    if state.get("last_success") != expected:
        raise BenchError(f"requires immediately preceding successful {expected}")


# ---------------------------------------------------------------------------
# dedicated OS user — the fresh-user mechanism, not a HOME trick
# ---------------------------------------------------------------------------

def dedicated_ctx(manifest):
    """Validate that the driver itself runs as the dedicated user. HOME is
    then the account's real home (getpwuid-backed), so every per-user cache
    under it is measurement-owned — including the shared cargo target's
    dependency units, which is what makes the cold build truly cold."""
    du = manifest["dedicated_user"]
    try:
        pw = pwd.getpwnam(du["name"])
    except KeyError:
        raise BenchError(
            f"dedicated user {du['name']!r} does not exist; create it first "
            "(see README)")
    if os.getuid() != pw.pw_uid:
        raise BenchError(
            f"must run as dedicated user {du['name']} "
            f"(sudo -Hu {du['name']} python3 drive.py ...); "
            f"current uid={os.getuid()}")
    home = Path(pw.pw_dir).resolve()
    real_home = Path.home().resolve()
    if real_home != home or home != Path(du["expected_home"]).resolve():
        raise BenchError(
            f"dedicated home mismatch: getpwuid={home} HOME={real_home} "
            f"expected under {du['expected_home']}")
    if home.stat().st_uid != pw.pw_uid:
        raise BenchError("dedicated account does not own its home")
    if not str(ROOT).startswith(str(home) + os.sep):
        raise BenchError(
            f"BENCH282_ROOT {ROOT} is not inside the dedicated home {home}")
    if ROOT.exists() and ROOT.stat().st_uid != pw.pw_uid:
        raise BenchError(f"{ROOT} is not owned by {du['name']}")
    state = load_state()
    water_home = home / ".water"
    if not state.get("dedicated_cache_claimed"):
        leftovers = list(water_home.iterdir()) if water_home.exists() else []
        if leftovers:
            raise BenchError(
                f"{water_home} already holds {len(leftovers)} entries the "
                "driver did not create — refusing to inherit or erase "
                "foreign cache. Clear it explicitly or prove it is stale.")
        state["dedicated_cache_claimed"] = True
        save_state(state)
    for path in (water_home, home / "Library", home / ".cargo", home / ".rustup"):
        assert_owned({"home": home, "uid": pw.pw_uid}, path)
    for key in ("CARGO_HOME", "RUSTUP_HOME", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR"):
        if key in os.environ:
            assert_owned({"home": home, "uid": pw.pw_uid}, os.environ[key])
    return {"uid": pw.pw_uid, "home": home, "name": du["name"]}


def assert_owned(ctx, path):
    """Every path deleted must resolve inside the dedicated home and be
    owned by the dedicated uid — checked before each cold cleanup."""
    real = Path(path).resolve()
    if not str(real).startswith(str(ctx["home"]) + os.sep):
        raise BenchError(f"refusing to touch {real} — outside dedicated home")
    p = Path(path)
    if p.exists() and p.stat().st_uid != ctx["uid"]:
        raise BenchError(f"refusing to touch {real} — owned by uid "
                         f"{p.stat().st_uid}, not {ctx['uid']}")


# ---------------------------------------------------------------------------
# cold state — under the dedicated user the whole per-user cache is owned,
# so the wipe covers dep units too: true cold
# ---------------------------------------------------------------------------

def cold_clean(manifest, ctx, side, subject, project):
    log = LOGS / f"cold-clean-{side}-{time.monotonic_ns()}.log"
    cache_root = ctx["home"] / ".water" / "build_cache"
    scope = {
        "cli_commands": [], "deleted": {},
        "cache_root_bytes_before": du_bytes(cache_root),
    }
    for tpl in (manifest["cold_clean"]["owned_delete_globs"]
                + manifest["cold_clean"]["wipe_whole_dirs"]):
        for path in glob.glob(expand(tpl, project=project,
                                     scheme=subject["scheme"], home=ctx["home"])):
            assert_owned(ctx, path)
    for argv_tpl in manifest["cold_clean"]["cli_commands"]:
        argv = [str(water_bin(side)) if a == "water" else a for a in argv_tpl]
        argv = [expand(a, project=project, scheme=subject["scheme"],
                       home=ctx["home"]) for a in argv]
        rec = run(argv, manifest["timeouts"]["clean_step"],
                  env=measured_env(manifest), log_file=log)
        scope["cli_commands"].append({
            "argv": argv, "exit_code": rec["exit_code"],
            "timed_out": rec["timed_out"], "wall_ms": rec["wall_ms"],
            "log_file": str(log)})
        if not record_ok(rec):
            raise BenchError(f"cold clean failed: {shlex_join(argv)}\n{log_tail(log)}")
    for tpl in (manifest["cold_clean"]["owned_delete_globs"]
                + manifest["cold_clean"]["wipe_whole_dirs"]):
        for path in glob.glob(expand(tpl, project=project,
                                     scheme=subject["scheme"],
                                     home=ctx["home"])):
            assert_owned(ctx, path)
            scope["deleted"][path] = du_bytes(path)
            p = Path(path)
            if p.is_dir() and not p.is_symlink():
                shutil.rmtree(p)
            else:
                p.unlink(missing_ok=True)
    scope["cache_root_bytes_after"] = du_bytes(cache_root)
    scope["scope"] = ("full per-user cache under dedicated uid "
                      f"{ctx['uid']} ({ctx['home']}) — dep units included")
    return scope


def disk_usage(manifest, subject, project, home):
    usage = {}
    for tpl in manifest["disk_usage_paths"]:
        for path in glob.glob(expand(tpl, project=project,
                                     scheme=subject["scheme"], home=home)):
            usage[path] = du_bytes(path)
    return usage


def require_disk_evidence(record, usage):
    """A successful build/package must have produced measurable artifacts;
    a missing required path is an error, not an empty dict."""
    if record.get("exit_code") == 0 and not record.get("timed_out") \
            and not usage:
        record["diagnosis"] = (
            "leg succeeded but no disk-usage path materialized — "
            "manifest.disk_usage_paths do not cover the written artifacts")
        record["disk_evidence_missing"] = True


# ---------------------------------------------------------------------------
# setup
# ---------------------------------------------------------------------------

def checkout_commit(url, sha, dest):
    dest.parent.mkdir(parents=True, exist_ok=True)
    log = LOGS / f"git-{dest.name}-{sha[:8]}.log"
    for argv in (
        ["git", "init", "-q", str(dest)],
        ["git", "-C", str(dest), "remote", "add", "origin", url],
        ["git", "-C", str(dest), "fetch", "-q", "--depth", "1", "origin", sha],
        ["git", "-C", str(dest), "checkout", "-q", "--detach", "FETCH_HEAD"],
    ):
        preparation_command(argv, manifest_timeouts()["clone"], sha, log_file=log)
    head = checked_output(["git", "-C", str(dest), "rev-parse", "HEAD"])
    if head != sha:
        raise BenchError(
            f"provenance failed for {dest}: HEAD {head} != pinned {sha}")
    return head


def manifest_timeouts():
    return load_manifest()["timeouts"]


def requested_pins(manifest):
    return {side: {name: resolve_pin(pins, name, side)
                   for name in ("apple_backend", "waterui", "cli")}
            for side, pins in manifest["sides"].items()}


def run_started(state):
    return (bool(state.get("run_started")) or (ROOT / "apps").exists()
            or bool(state.get("parity_passed")) or any(RESULTS.glob("*.jsonl")))


def require_finalized(manifest, state):
    if not state.get("inputs_finalized"):
        raise BenchError("run inputs are not finalized; run finalize-inputs before scaffold")
    if requested_pins(manifest) != state.get("resolved_pins"):
        raise BenchError("requested pins differ from immutable finalized run inputs")
    for side in manifest["sides"]:
        receipt = state.get("tools", {}).get(side, {})
        if receipt.get("source_sha") != state["resolved_pins"][side]["cli"]:
            raise BenchError(f"{side}: finalized CLI provenance is missing")
        if file_sha256(water_bin(side)) != receipt.get("binary_sha256"):
            raise BenchError(f"{side}: installed CLI changed after finalization")


def file_sha256(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def preparation_command(argv, timeout_s, source_sha, **kwargs):
    """Preparatory timing ledger is separate from measurement records."""
    rec = run(argv, timeout_s, **kwargs)
    LOGS.mkdir(parents=True, exist_ok=True)
    with (LOGS / "preparation.jsonl").open("a") as output:
        output.write(json.dumps({"source_sha": source_sha, **rec}) + "\n")
    if not record_ok(rec):
        raise BenchError(f"preparation failed: {shlex_join(argv)}; {rec.get('stdout')}")
    return rec


def require_clean_checkout(path, backend=None):
    """Reject tracked changes and untracked source before trusting a pin."""
    if backend is not None:
        link = path / "backends" / "apple"
        if not link.is_symlink() or link.resolve() != backend.resolve():
            raise BenchError(f"{link}: expected the harness-owned backend link")
    status = checked_output([
        "git", "-C", str(path), "status", "--porcelain", "--untracked-files=all"])
    changes = [line for line in status.splitlines()
               if not (backend is not None and line == "?? backends/apple")]
    if changes:
        raise BenchError(f"{path}: dirty checkout (tracked changes or untracked inputs); refusing source provenance")


def owned_checkout(ctx, path, url, backend=None):
    """A standalone, clean checkout at our exact path, never another worktree."""
    assert_owned(ctx, path)
    if path.is_symlink() or not (path / ".git").is_dir() or (path / ".git").is_symlink():
        raise BenchError(f"{path}: expected our standalone checkout, not a linked worktree")
    git = ["git", "-C", str(path)]
    if Path(checked_output([*git, "rev-parse", "--show-toplevel"])).resolve() != path.resolve():
        raise BenchError(f"{path}: foreign repository root")
    if Path(checked_output([*git, "rev-parse", "--absolute-git-dir"])).resolve() != (path / ".git").resolve():
        raise BenchError(f"{path}: foreign Git directory")
    if checked_output([*git, "remote", "get-url", "origin"]) != url:
        raise BenchError(f"{path}: unexpected origin")
    worktrees = checked_output([*git, "worktree", "list", "--porcelain"])
    if sum(line.startswith("worktree ") for line in worktrees.splitlines()) != 1:
        raise BenchError(f"{path}: repository has other worktrees; refusing input replacement")
    require_clean_checkout(path, backend)
    return checked_output([*git, "rev-parse", "HEAD"])


@dataclass(frozen=True)
class PreparedCheckout:
    side: str
    name: str
    path: Path
    url: str
    previous_sha: str | None
    source_sha: str
    backend_link: Path | None


@dataclass(frozen=True)
class PreparedCLI:
    side: str
    source_sha: str
    receipt: dict | None


@dataclass(frozen=True)
class PreparedInputs:
    checkouts: tuple[PreparedCheckout, ...]
    tools: tuple[PreparedCLI, ...]


def cli_receipt(ctx, state, side, sha, supplied, finalize):
    stored = state.get("tools", {}).get(side)
    explicit = supplied.get(side)
    receipt = explicit if explicit is not None else stored
    if stored and explicit and stored != explicit:
        if not finalize or side != "new" or run_started(state):
            raise BenchError(f"{side}: CLI receipt replacement requires unstarted new-side finalization")
    if stored and stored.get("source_sha") != sha and explicit is None:
        raise BenchError(f"{side}: new CLI pin requires an explicit coordinator receipt")
    binary = water_bin(side)
    if binary.exists():
        assert_owned(ctx, binary)
        if not receipt or receipt.get("source_sha") != sha or receipt.get("binary_sha256") != file_sha256(binary):
            raise BenchError(f"{side}: installed CLI needs matching source-SHA/binary-SHA256 provenance")
    elif receipt:
        raise BenchError(f"{side}: CLI receipt exists but binary is missing")
    return copy.deepcopy(receipt)


def preflight_inputs(manifest, ctx, state, requested, finalize, supplied):
    if run_started(state):
        raise BenchError("started run inputs are immutable")
    previous = state.get("prepared_pins") or state.get("resolved_pins", {})
    for side, pins in previous.items():
        for name, sha in pins.items():
            if sha != requested[side][name] and (side != "new" or not finalize):
                raise BenchError(f"{side}/{name}: prepared pin change requires new-side finalization")
    checkouts = []
    for side, pins in manifest["sides"].items():
        link = waterui_dir(side) / "backends" / "apple"
        if link.is_symlink():
            if link.resolve() != backend_dir(side).resolve():
                raise BenchError(f"{link}: foreign backend link")
        elif link.exists():
            raise BenchError(f"{link}: expected our backend link")
        for name, folder in (("apple_backend", "apple-backend"), ("waterui", "waterui"), ("cli", "cli")):
            path = ROOT / "checkouts" / side / folder
            head = None
            backend = backend_dir(side) if name == "waterui" and link.is_symlink() else None
            if path.exists():
                head = owned_checkout(ctx, path, pins[name]["repo"], backend)
                if head != requested[side][name]:
                    if not finalize or side != "new":
                        raise BenchError(f"{side}/{name}: checkout pin change requires new-side finalization")
            checkouts.append(PreparedCheckout(side, name, path, pins[name]["repo"], head,
                                              requested[side][name], backend))
    tools = tuple(PreparedCLI(side, requested[side]["cli"],
                             cli_receipt(ctx, state, side, requested[side]["cli"], supplied, finalize))
                  for side in manifest["sides"])
    return PreparedInputs(tuple(checkouts), tools)


def prepare_checkouts(manifest, ctx, state, requested, plan):
    for source in plan.checkouts:
        if source.previous_sha is None:
            checkout_commit(source.url, source.source_sha, source.path)
        elif source.previous_sha != source.source_sha:
            git = ["git", "-C", str(source.path)]
            preparation_command([*git, "fetch", "--depth", "1", "origin", source.source_sha],
                                manifest["timeouts"]["clone"], source.source_sha)
            preparation_command([*git, "checkout", "--detach", source.source_sha], 60, source.source_sha)
            if owned_checkout(ctx, source.path, source.url, source.backend_link) != source.source_sha:
                raise BenchError("input replacement did not reach the requested exact SHA")
    for side in manifest["sides"]:
        link = waterui_dir(side) / "backends" / "apple"
        link.parent.mkdir(parents=True, exist_ok=True)
        if link.is_symlink():
            if link.resolve() != backend_dir(side).resolve():
                raise BenchError(f"{link}: foreign backend link")
        elif link.exists():
            raise BenchError(f"{link}: expected our backend link")
        else:
            link.symlink_to(backend_dir(side), target_is_directory=True)
    state["prepared_pins"] = requested
    save_state(state)


def prepare_cli(manifest, ctx, state, side, sha, supplied, finalize=False):
    binary = water_bin(side)
    source = ROOT / "checkouts" / side / "cli"
    if checked_output(["git", "-C", str(source), "rev-parse", "HEAD"]) != sha:
        raise BenchError(f"{side}: CLI source SHA mismatch")
    require_clean_checkout(source)
    receipt = cli_receipt(ctx, state, side, sha, supplied, finalize)
    if not binary.exists():
        # --path uses this checkout's ordinary target; never set a parallel target
        # or copy a foreign user's caches. Reusing installed tools skips this entirely.
        rec = preparation_command(
            ["cargo", "install", "--locked", "--path", str(source),
             "--root", str(ROOT / "toolchains" / side)],
            manifest["timeouts"]["cargo_install"], sha, cwd=source,
            log_file=LOGS / f"cargo-install-{side}.log")
        receipt = {"source_sha": sha, "binary_sha256": file_sha256(binary),
                   "install": rec}
    previous = state.get("tools", {}).get(side)
    if previous and previous != receipt:
        state.setdefault("tool_history", {}).setdefault(side, []).append(copy.deepcopy(previous))
    state.setdefault("tools", {})[side] = receipt
    save_state(state)
    return preparation_command([str(binary), "--version"], 60, sha, capture=True)["stdout"].strip()


def compatible_devices(runtimes):
    """Compatibility comes only from CoreSimulator's per-runtime evidence."""
    pairs = []
    for runtime in runtimes:
        if not runtime.get("isAvailable") or ".iOS-" not in runtime["identifier"]:
            continue
        supported = runtime.get("supportedDeviceTypes")
        if not isinstance(supported, list) or not supported:
            raise BenchError(f"{runtime['identifier']}: missing supportedDeviceTypes compatibility evidence from simctl list runtimes --json")
        version = tuple(int(part) for part in runtime["version"].split("."))
        for device in supported:
            if not isinstance(device, dict) or "identifier" not in device:
                raise BenchError("supportedDeviceTypes must contain device objects with identifiers")
            if device.get("productFamily") == "iPhone":
                pairs.append((version, runtime["identifier"], device["identifier"]))
    if not pairs:
        raise BenchError("no available iOS runtime advertises a supported iPhone device type")
    return sorted(pairs)


def prepare_simulator(state):
    runtimes = json.loads(preparation_command(
        ["xcrun", "simctl", "list", "runtimes", "--json"], 60, None, capture=True)["stdout"])["runtimes"]
    pairs = compatible_devices(runtimes)
    owned = state.get("owned_simulator")
    if owned:
        if not any((runtime, device) == (owned["runtime"], owned["device_type"])
                   for _, runtime, device in pairs):
            raise BenchError("owned simulator is no longer compatible with an available runtime")
        devices = json.loads(preparation_command(
            ["xcrun", "simctl", "list", "devices", "--json"], 60, None, capture=True)["stdout"])["devices"]
        matches = [device for device in devices.get(owned["runtime"], [])
                   if device["udid"] == owned["udid"]]
        if len(matches) != 1 or not matches[0].get("isAvailable") or matches[0].get("deviceTypeIdentifier") != owned["device_type"]:
            raise BenchError("recorded owned simulator identity is missing or incompatible")
    else:
        _, runtime, device = pairs[-1]
        # Ignore names/precreated devices. Only the identifier created here is owned.
        udid = preparation_command(["xcrun", "simctl", "create", "bench-282", device, runtime],
                                   60, None, capture=True)["stdout"].strip()
        owned = {"udid": udid, "runtime": runtime, "device_type": device}
        state["owned_simulator"] = owned
        state["simulator_udid"] = udid
        save_state(state)
    preparation_command(["xcrun", "simctl", "bootstatus", owned["udid"], "-b"],
                        300, None, capture=True)
    state["simulator_udid"] = owned["udid"]


def cmd_setup(manifest, finalize=False, provenance_path=None):
    ctx = dedicated_ctx(manifest)
    LOGS.mkdir(parents=True, exist_ok=True)
    state = load_state()
    requested = requested_pins(manifest)
    if run_started(state):
        require_finalized(manifest, state)
        raise BenchError("run already started; preparation/finalization cannot mutate its inputs")
    supplied = json.loads(Path(provenance_path).read_text()) if provenance_path else {}
    plan = preflight_inputs(manifest, ctx, state, requested, finalize, supplied)
    state.setdefault("input_history", []).append({
        "at": now_eastern(), "previous": copy.deepcopy({key: state.get(key) for key in (
            "prepared_pins", "resolved_pins", "tools", "inputs_finalized", "lockfile_sha256",
            "form_src_sha256", "source_sha256", "parity_passed", "last_success", "packaged")}),
        "requested": copy.deepcopy(requested), "status": "preflight-passed"})
    # A preparation update cannot leave an earlier finalization valid when a
    # later step fails. The reusable tool receipts remain independently valid.
    state.pop("inputs_finalized", None)
    save_state(state)
    prepare_checkouts(manifest, ctx, state, requested, plan)
    approved = {tool.side: tool.receipt for tool in plan.tools if tool.receipt is not None}
    versions = {tool.side: prepare_cli(manifest, ctx, state, tool.side, tool.source_sha, approved, finalize)
                for tool in plan.tools}
    prepare_simulator(state)
    # Preparation remains reusable. Only explicit finalization freezes run inputs.
    state["water_versions"] = versions
    if finalize:
        for key in ("lockfile_sha256", "form_src_sha256", "source_sha256",
                    "parity_passed", "last_success", "packaged"):
            state.pop(key, None)
        state["resolved_pins"] = requested
        state["toolchain"] = probe_toolchain(manifest)
        state["inputs_finalized"] = True
    state["input_history"][-1]["status"] = "finalized" if finalize else "prepared"
    save_state(state)
    print("[finalize-inputs] ready for scaffold" if finalize else "[setup] tools prepared; run finalize-inputs")


# ---------------------------------------------------------------------------
# scaffold — identical view source + real app manifests on both sides
# ---------------------------------------------------------------------------

def write_toml(path, data):
    path.write_text(tomli_w.dumps(data))
    if tomllib.loads(path.read_text()) != data:
        raise BenchError(f"TOML round-trip failed: {path}")


def ensure_backend_path(water_toml, backend, scheme, side):
    """Pin the manifest's backend binding for the side's exact CLI schema.
    The old CLI persists `[backends.apple] backend_path` (+ `scheme`). The new
    CLI records no `backends` table at all — `water create` already recorded
    `waterui_path`, and the backend comes from the harness-owned
    `backends/apple` link under it, so a new-side manifest is validated, never
    written here."""
    if side == "old":
        data = tomllib.loads(water_toml.read_text())
        apple = data.setdefault("backends", {}).setdefault("apple", {})
        apple["backend_path"] = str(backend)
        apple["scheme"] = scheme
        write_toml(water_toml, data)
    validate_backend_path(water_toml, backend, side)


def validate_backend_path(water_toml, backend, side):
    data = tomllib.loads(water_toml.read_text())
    package = data.get("package", {})
    if side == "new":
        if "type" in package or "backends" in data:
            raise BenchError(f"{water_toml}: removed app-mode keys on new side")
        waterui = data.get("waterui_path")
        if not isinstance(waterui, str) or \
                Path(waterui).resolve() != waterui_dir(side).resolve():
            raise BenchError(f"{water_toml}: new side requires exact waterui_path")
        link = Path(waterui) / "backends" / "apple"
        if not link.is_symlink() or link.resolve() != Path(backend).resolve():
            raise BenchError(
                f"{water_toml}: new side requires the harness-owned "
                f"backends/apple link to {backend}")
        return
    apple = data.get("backends", {}).get("apple", {})
    if apple.get("backend_path") != str(backend):
        raise BenchError(f"{water_toml}: exact [backends.apple].backend_path mismatch")
    if package.get("type") != "app" or not apple.get("scheme"):
        raise BenchError(f"{water_toml}: old app mode requires type and scheme")


def set_source_variant(manifest, side, subject_name, changed):
    """Toggle an actual rendered string; no unused const or invalid Rust edit."""
    cfg = manifest["subjects"][subject_name]["incremental_edit"]
    lib = project_dir(manifest, side, subject_name) / "src" / "lib.rs"
    text = lib.read_text()
    before, after = cfg["before"], cfg["after"]
    if text.count(before) + text.count(after) != 1:
        raise BenchError(f"{lib}: expected exactly one rendered incremental label")
    source, target = (before, after) if changed else (after, before)
    if changed and source not in text:
        raise BenchError(f"{lib}: incremental change already applied")
    if source in text:
        lib.write_text(text.replace(source, target))
    return {"file": str(lib), "before": source, "after": target}


def materialize_lockfile(manifest, proj):
    """Dependency resolution is pinned setup, not measured work — failure
    is fatal, never a warning before a measured leg."""
    lock = proj / "Cargo.lock"
    if not lock.exists():
        run_or_die(["cargo", "generate-lockfile"],
                   manifest["timeouts"]["lockfile"],
                   f"cargo generate-lockfile in {proj}",
                   cwd=proj, log_file=LOGS / f"lockfile-{proj.name}-{proj.parent.name}.log")
    return hashlib.sha256(lock.read_bytes()).hexdigest()


def tree_sha256(root):
    h = hashlib.sha256()
    for p in sorted(root.rglob("*")):
        if p.is_file():
            h.update(str(p.relative_to(root)).encode())
            h.update(p.read_bytes())
    return h.hexdigest()


def stage_form(manifest, side):
    """Stage the ONE pinned form source as a real app project. Both sides
    get byte-identical view source; manifests differ only by side schema —
    the old `[backends.apple]` table versus the new `waterui_path` binding —
    and the side-local checkout paths."""
    src_ref = manifest["subjects"]["form"]["source"]
    if src_ref["sha"] != manifest["sides"]["old"]["waterui"]["sha"]:
        raise BenchError("form source pin must equal the staged old framework pin")
    origin = (ROOT / "checkouts" / "old" / "waterui" / src_ref["path"])
    if not origin.exists():
        raise BenchError(
            f"{origin} missing — run setup first; form source is pinned to "
            f"waterui {src_ref['sha']}")
    dest = project_dir(manifest, side, "form")
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    shutil.copytree(origin / "src", dest / "src")
    for extra in ("assets", "Assets", "web"):
        if (origin / extra).exists():
            shutil.copytree(origin / extra, dest / extra)
    package = {"name": "Form Example", "bundle_identifier": manifest["subjects"]["form"]["bundle_id"]}
    water_toml = {"waterui_path": str(waterui_dir(side)), "package": package}
    if side == "old":
        package["type"] = "app"
        water_toml["backends"] = {"apple": {
            "backend_path": str(backend_dir(side)),
            "scheme": manifest["subjects"]["form"]["scheme"]}}
    write_toml(dest / "Water.toml", water_toml)
    write_toml(dest / "Cargo.toml", {
        "package": {"name": "form_example", "version": "0.1.0", "edition": "2024", "publish": False},
        "features": {"dev": ["waterui/dynamic_linking"]},
        "dependencies": {"waterui": {"path": str(waterui_dir(side))}}})
    validate_backend_path(dest / "Water.toml", backend_dir(side), side)
    return dest


def cmd_scaffold(manifest, side):
    dedicated_ctx(manifest)
    state = load_state()
    require_finalized(manifest, state)
    if any(RESULTS.glob("*.jsonl")) or state.get("parity_passed"):
        raise BenchError("cannot scaffold after parity or measurement has begun")
    state["run_started"] = True
    save_state(state)
    pins = manifest["sides"][side]
    fresh = manifest["subjects"]["fresh"]
    apps_parent = ROOT / "apps" / side
    apps_parent.mkdir(parents=True, exist_ok=True)

    fresh_dir = project_dir(manifest, side, "fresh")
    if not fresh_dir.exists():
        argv = [str(water_bin(side)), "create", fresh["display_name"],
                "--bundle-id", fresh["bundle_id"],
                "--waterui-path", str(waterui_dir(side)),
                *pins["create_extra_args"]]
        run_or_die(argv, manifest["timeouts"]["scaffold"],
                   f"water create {side}", cwd=apps_parent,
                   log_file=LOGS / f"create-{side}.log")
    ensure_backend_path(fresh_dir / "Water.toml",
                        backend_dir(side), fresh["scheme"], side)

    form_dir = stage_form(manifest, side)

    state = load_state()
    state.pop("parity_passed", None)
    state.setdefault("lockfile_sha256", {})
    state.setdefault("form_src_sha256", {})
    state["lockfile_sha256"][f"{side}/fresh"] = materialize_lockfile(
        manifest, fresh_dir)
    state["lockfile_sha256"][f"{side}/form"] = materialize_lockfile(
        manifest, form_dir)
    state["form_src_sha256"][side] = tree_sha256(form_dir / "src")
    save_state(state)
    print(f"[scaffold] {side}: fresh-lock="
          f"{state['lockfile_sha256'][f'{side}/fresh'][:12]} "
          f"form-src={state['form_src_sha256'][side][:12]}")


def normalized_lib_rs(path):
    return path.read_text()


def cmd_parity(manifest):
    dedicated_ctx(manifest)
    require_finalized(manifest, load_state())
    failures = []
    old_lib = project_dir(manifest, "old", "fresh") / "src" / "lib.rs"
    new_lib = project_dir(manifest, "new", "fresh") / "src" / "lib.rs"
    if not (old_lib.exists() and new_lib.exists()):
        failures.append("fresh app missing on a side; run scaffold both")
    else:
        old_n, new_n = normalized_lib_rs(old_lib), normalized_lib_rs(new_lib)
        if old_n != new_n:
            diff = "\n".join(difflib.unified_diff(
                old_n.splitlines(), new_n.splitlines(),
                "old/src/lib.rs", "new/src/lib.rs", lineterm=""))
            (LOGS / "parity-fresh.diff").write_text(diff + "\n")
            failures.append("fresh src/lib.rs diverged "
                            "(logs/parity-fresh.diff)")
    state = load_state()
    src_hashes = {side: tree_sha256(project_dir(manifest, side, "form") / "src")
                  for side in manifest["sides"]}
    if src_hashes != state.get("form_src_sha256", {}):
        failures.append("form sources changed after scaffold")
    if set(src_hashes) != set(manifest["sides"]) or len(set(src_hashes.values())) != 1:
        failures.append(f"form src trees differ: {src_hashes}")
    missing = [k for k in ("old/fresh", "old/form", "new/fresh", "new/form")
               if k not in state.get("lockfile_sha256", {})]
    if missing:
        failures.append(f"lockfiles not materialized: {missing}")
    for entry in failures:
        print(f"parity: FAIL — {entry}", file=sys.stderr)
    if failures:
        sys.exit(1)
    for side in manifest["sides"]:
        verify_checkouts(state, side)
        for subject in manifest["subjects"]:
            project = project_dir(manifest, side, subject)
            validate_backend_path(project / "Water.toml", backend_dir(side), side)
            if hashlib.sha256((project / "Cargo.lock").read_bytes()).hexdigest() != state["lockfile_sha256"][f"{side}/{subject}"]:
                raise BenchError(f"{project}: lockfile changed after scaffold")
    state["parity_passed"] = True
    state["source_sha256"] = {
        f"{side}/{subject}": source_fingerprint(manifest, side, subject)
        for side in manifest["sides"] for subject in manifest["subjects"]}
    save_state(state)
    print("parity: identical view source; lockfiles pinned")


# ---------------------------------------------------------------------------
# legs
# ---------------------------------------------------------------------------

def toolchain_guard(manifest, record):
    """Mismatch against the setup toolchain snapshot FAILS the sample —
    paired ratios across toolchains are meaningless."""
    expected = load_state().get("toolchain")
    if not expected:
        raise BenchError("no setup toolchain snapshot; run setup first")
    current = probe_toolchain(manifest)
    record["toolchain"] = current
    mismatched = {k: (expected.get(k), current.get(k))
                  for k in expected if expected.get(k) != current.get(k)}
    if mismatched:
        raise BenchError(f"host toolchain changed since setup: {mismatched}")
    record["toolchain_match"] = True


def verify_checkouts(state, side):
    for name, folder in (("apple_backend", "apple-backend"), ("waterui", "waterui"), ("cli", "cli")):
        path = ROOT / "checkouts" / side / folder
        head = checked_output(["git", "-C", str(path), "rev-parse", "HEAD"])
        if head != state["resolved_pins"][side][name]:
            raise BenchError(f"{path}: checkout no longer matches resolved pin")
        require_clean_checkout(path, backend_dir(side) if name == "waterui" else None)


def source_fingerprint(manifest, side, subject):
    root = project_dir(manifest, side, subject) / "src"
    edit = manifest["subjects"][subject]["incremental_edit"]
    digest = hashlib.sha256()
    for path in sorted(root.rglob("*")):
        if path.is_file():
            data = path.read_bytes()
            if path == root / "lib.rs":
                data = data.replace(edit["after"].encode(), edit["before"].encode())
            digest.update(str(path.relative_to(root)).encode())
            digest.update(data)
    return digest.hexdigest()


def app_features(project):
    """The enabled feature surface, recorded verbatim — the issue requires
    reporting which features the app enables."""
    cargo = project / "Cargo.toml"
    data = tomllib.loads(cargo.read_text())
    return {"features": data.get("features", {}),
            "dependencies": data.get("dependencies", {})}


def leg_build(manifest, ctx, side, subject_name, platform, sample, cold):
    subject = manifest["subjects"][subject_name]
    project = project_dir(manifest, side, subject_name)
    leg = "cold-build" if cold else "incremental-build"
    record = {"record_version": 3, "side": side, "subject": subject_name,
              "leg": leg, "platform": platform, "sample": sample,
              "build_scope": manifest["sides"][side]["build_scope"],
              "app_features": app_features(project)}
    toolchain_guard(manifest, record)
    if cold:
        set_source_variant(manifest, side, subject_name, False)
        record["cold_clean"] = cold_clean(manifest, ctx, side, subject, project)
    else:
        record["incremental_change"] = set_source_variant(
            manifest, side, subject_name, True)

    argv = [str(water_bin(side)), "build", "--platform", platform,
            "--path", str(project)]
    rec = run(argv, timeout_for(manifest, leg), env=measured_env(manifest),
              log_file=LOGS / f"{leg}-{side}-{subject_name}-{platform}-s{sample}.log")
    record.update(rec)
    record["env_overrides"] = manifest["measured_env_overrides"]
    record["metrics"] = {"wall_ms": rec["wall_ms"]}
    usage = disk_usage(manifest, subject, project, ctx["home"])
    record["disk_bytes"] = usage
    record["metrics"]["build_disk_bytes"] = sum(usage.values())
    if rec["exit_code"] != 0 or rec["timed_out"]:
        record["diagnosis"] = log_tail(rec["log_file"])
    else:
        require_disk_evidence(record, usage)
    return record


def leg_preview(manifest, ctx, side, subject_name, sample, cold):
    subject = manifest["subjects"][subject_name]
    if not subject["preview_target"]:
        raise BenchError(f"subject {subject_name} has no preview target")
    project = project_dir(manifest, side, subject_name)
    leg = "preview-cold" if cold else "preview-warm"
    out_png = RESULTS / (f"preview-{side}-{subject_name}-s{sample}-"
                         + ("cold" if cold else "warm") + ".png")
    record = {"record_version": 3, "side": side, "subject": subject_name,
              "leg": leg, "platform": "macos", "sample": sample,
              "app_features": app_features(project)}
    toolchain_guard(manifest, record)
    if cold:
        set_source_variant(manifest, side, subject_name, False)
        record["cold_clean"] = cold_clean(manifest, ctx, side, subject, project)
    out_png.unlink(missing_ok=True)
    argv = [str(water_bin(side)), "preview", subject["preview_target"],
            "--platform", "macos", "--output", str(out_png),
            "--path", str(project)]
    rec = run(argv, timeout_for(manifest, leg), env=measured_env(manifest),
              log_file=LOGS / f"{leg}-{side}-{subject_name}-s{sample}.log")
    record.update(rec)
    record["env_overrides"] = manifest["measured_env_overrides"]
    png = out_png if out_png.exists() else None
    record["metrics"] = {"wall_ms": rec["wall_ms"],
                         "png_bytes": png.stat().st_size if png else None}
    if rec["exit_code"] != 0 or rec["timed_out"]:
        record["diagnosis"] = log_tail(rec["log_file"])
    elif png is None:
        record["diagnosis"] = f"preview exited 0 but no PNG at {out_png}"
    return record


def read_bundle_executable(app_path, platform):
    """The executable name comes from the app's own Info.plist, never from
    the bundle filename."""
    plist = (app_path / "Contents" / "Info.plist" if platform == "macos"
             else app_path / "Info.plist")
    rec = run(["plutil", "-extract", "CFBundleExecutable", "raw",
               str(plist)], 60, capture=True)
    if rec["exit_code"] != 0 or not (rec["stdout"] or "").strip():
        raise BenchError(f"CFBundleExecutable unreadable in {plist}")
    return rec["stdout"].strip()


def leg_package(manifest, ctx, side, subject_name, platform, sample):
    subject = manifest["subjects"][subject_name]
    project = project_dir(manifest, side, subject_name)
    record = {"record_version": 3, "side": side, "subject": subject_name,
              "leg": "package", "platform": platform, "sample": sample,
              "includes_native_build": True,
              "app_features": app_features(project)}
    toolchain_guard(manifest, record)
    set_source_variant(manifest, side, subject_name, False)
    record["cold_clean"] = cold_clean(manifest, ctx, side, subject, project)
    extra = manifest["legs"]["package"].get(
        "platform_extra_args", {}).get(platform, [])
    argv = [str(water_bin(side)), "package", "--platform", platform,
            "--backend", "apple", "--release", *extra,
            "--path", str(project)]
    rec = run(argv, timeout_for(manifest, "package"),
              env=measured_env(manifest),
              log_file=LOGS / f"package-{side}-{subject_name}-{platform}-s{sample}.log")
    record.update(rec)
    record["env_overrides"] = manifest["measured_env_overrides"]
    usage = disk_usage(manifest, subject, project, ctx["home"])
    record["disk_bytes"] = usage

    # Exact artifact path only: the CLI's own "Packaged at" line. No glob
    # fallback — an unlocated artifact is a failed leg, not a guess.
    app_path = None
    log_text = Path(rec["log_file"]).read_text(errors="replace") \
        if rec["log_file"] else ""
    hits = re.findall(r"(?m)Packaged at ([^\r\n]+)",
                      re.sub(r"\x1b\[[0-9;]*m", "", log_text))
    if hits:
        candidate = Path(hits[-1].strip())
        if candidate.is_dir() and candidate.suffix == ".app":
            app_path = candidate
    if rec["exit_code"] == 0 and not rec["timed_out"] and app_path is None:
        record["diagnosis"] = ("no usable 'Packaged at <path>' line in the "
                               "package log; artifact unlocated")
        return record
    if app_path is not None:
        exe_name = read_bundle_executable(app_path, platform)
        exe = (app_path / "Contents" / "MacOS" / exe_name
               if platform == "macos" else app_path / exe_name)
        if not exe.exists():
            record["diagnosis"] = f"declared executable missing: {exe}"
            return record
        record["metrics"] = {
            "wall_ms": rec["wall_ms"],
            "app_bytes": sum(f.stat().st_size for f in app_path.rglob("*")
                             if f.is_file()),
            "executable_bytes": exe.stat().st_size}
        record["artifacts"] = {"app_path": str(app_path),
                               "executable": exe_name}
        state = load_state()
        state.setdefault("packaged", {})[
            f"{side}|{subject_name}|{platform}|{sample}"] = {
            "app": str(app_path), "exe": exe_name}
        save_state(state)
    if rec["exit_code"] != 0 or rec["timed_out"]:
        record["diagnosis"] = log_tail(rec["log_file"])
    else:
        require_disk_evidence(record, usage)
    return record


# ---------------------------------------------------------------------------
# launch — marker scoped to the measured PID, event-driven, no log replay
# ---------------------------------------------------------------------------

def stop_process(proc):
    """Reap every owned child, including on exceptions and interruption."""
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    proc.wait(timeout=10)


class StructuredLogStream:
    """A live NDJSON stream: attach first, buffer events, select PID later."""

    def __init__(self, proc):
        self.proc = proc
        self.buffer = bytearray()
        self.events = []
        self.attached = False

    def receive(self, deadline):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise BenchError("log stream deadline expired")
        ready, _, _ = select.select([self.proc.stdout], [], [], remaining)
        if not ready:
            raise BenchError("log stream deadline expired")
        chunk = os.read(self.proc.stdout.fileno(), 65536)
        if not chunk:
            raise BenchError("log stream closed before required event")
        self.buffer.extend(chunk)
        while b"\n" in self.buffer:
            line, _, rest = self.buffer.partition(b"\n")
            self.buffer = bytearray(rest)
            if not line.strip():
                continue
            if line.startswith(b"Filtering the log data"):
                self.attached = True
                continue
            try:
                event = json.loads(line)
            except (ValueError, UnicodeDecodeError) as exc:
                raise BenchError(f"unexpected log stream output: {line!r}") from exc
            if not isinstance(event, dict):
                raise BenchError("log stream event is not a JSON object")
            self.events.append(event)

    def await_attach(self, deadline):
        while not self.attached:
            self.receive(deadline)

    def first_paint(self, pid, cfg, deadline):
        pattern = re.compile(re.escape(cfg["marker"]) + r"(\d+)\b")
        while True:
            events, self.events = self.events, []
            for event in events:
                if event.get("processID") != pid or event.get("subsystem") != cfg["log_subsystem"]:
                    continue
                match = pattern.search(event.get("eventMessage", ""))
                if match:
                    return int(match.group(1))
            self.receive(deadline)


def rss_sample(pid, samples, interval_s, deadline):
    """The only timed interval is the specified RSS sampling cadence."""
    values = []
    for index in range(samples):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise BenchError("launch deadline expired during RSS sampling")
        rec = run(["ps", "-o", "rss=", "-p", str(pid)],
                  min(10, remaining), capture=True)
        if not record_ok(rec) or not (rec["stdout"] or "").strip():
            raise BenchError(f"pid {pid} exited during RSS window at sample {index + 1}")
        try:
            value = int(rec["stdout"].strip()) * 1024
        except ValueError as exc:
            raise BenchError("invalid ps RSS output") from exc
        if value <= 0:
            raise BenchError("non-positive RSS sample")
        values.append(value)
        if index + 1 < samples:
            if time.monotonic() + interval_s >= deadline:
                raise BenchError("launch deadline cannot cover remaining RSS interval")
            time.sleep(interval_s)
    return values


def sim_cleanup(udid, bundle, operation):
    rec = run(["xcrun", "simctl", operation, udid, bundle], 30, capture=True)
    # Terminating after a failed launch can report "not running"; uninstall
    # remains mandatory and ExitStack executes it even if this callback raises.
    if not record_ok(rec) and not (
            operation == "terminate" and "not running" in (rec["stdout"] or "").lower()):
        raise BenchError(f"simctl {operation} failed: {rec['stdout']}")


def leg_launch(manifest, ctx, side, subject_name, platform, sample):
    subject = manifest["subjects"][subject_name]
    cfg = manifest["legs"]["launch"]
    state = load_state()
    packaged = state.get("packaged", {}).get(
        f"{side}|{subject_name}|{platform}|{sample}")
    if not packaged or not Path(packaged["app"]).is_dir():
        raise BenchError("no surviving package for this side/subject/platform/sample")
    app_path = Path(packaged["app"])
    record = {"record_version": 4, "side": side, "subject": subject_name,
              "leg": "launch", "platform": platform, "sample": sample,
              "artifacts": {"app_path": str(app_path)}, "exit_code": 0,
              "app_features": app_features(project_dir(manifest, side, subject_name))}
    toolchain_guard(manifest, record)
    deadline = time.monotonic() + timeout_for(manifest, "launch")

    def remaining(limit):
        return bounded(min(limit, deadline - time.monotonic()), "launch step")

    with ExitStack() as owned:
        prefix = []
        if platform == "ios-simulator":
            udid = state["simulator_udid"]
            boot = run(["xcrun", "simctl", "bootstatus", udid, "-b"],
                       remaining(120), capture=True)
            if not record_ok(boot):
                raise BenchError(f"simulator bootstatus failed: {boot['stdout']}")
            owned.callback(sim_cleanup, udid, subject["bundle_id"], "uninstall")
            inst = run(["xcrun", "simctl", "install", udid, str(app_path)],
                       remaining(120), capture=True)
            if not record_ok(inst):
                raise BenchError(f"simulator install failed: {inst['stdout']}")
            prefix = ["xcrun", "simctl", "spawn", udid]

        stream_argv = [*prefix, "log", "stream", "--level", "info",
                       "--predicate", f'subsystem == "{cfg["log_subsystem"]}"',
                       "--style", "ndjson"]
        stream = subprocess.Popen(stream_argv, stdout=subprocess.PIPE,
                                  stderr=subprocess.STDOUT, bufsize=0,
                                  start_new_session=True)
        owned.callback(stream.stdout.close)
        owned.callback(stop_process, stream)
        events = StructuredLogStream(stream)
        events.await_attach(min(deadline, time.monotonic() + cfg["stream_attach_timeout_s"]))

        # The pipe is attached before spawn; even a marker emitted before
        # simctl returns its PID is retained and subsequently PID-filtered.
        t0 = time.monotonic_ns()
        if platform == "macos":
            app = subprocess.Popen(
                [str(app_path / "Contents" / "MacOS" / packaged["exe"])],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                start_new_session=True)
            owned.callback(stop_process, app)
            pid = app.pid
        else:
            owned.callback(sim_cleanup, udid, subject["bundle_id"], "terminate")
            launch = run(["xcrun", "simctl", "launch", "--terminate-running-process",
                          udid, subject["bundle_id"]], remaining(60), capture=True)
            match = re.search(r":\s*(\d+)\s*$", launch["stdout"] or "")
            if not record_ok(launch) or not match:
                raise BenchError(f"simctl launch failed or gave no PID: {launch['stdout']}")
            pid = int(match.group(1))
        record["pid"] = pid
        record["argv"] = stream_argv
        marker_ms = events.first_paint(pid, cfg, deadline)
        observed_ms = (time.monotonic_ns() - t0) // 1_000_000
        rss = rss_sample(pid, cfg["rss_samples"], cfg["rss_interval_s"], deadline)
        record["metrics"] = {
            "first_paint_ms": marker_ms, "marker_observed_wall_ms": observed_ms,
            "rss_samples_bytes": rss,
            "steady_rss_bytes": int(statistics.median(rss[-cfg["steady_tail"]:])),
            "peak_rss_bytes": max(rss)}
    return record


def cmd_measure(manifest, side, subject_name, leg, platform, sample):
    if side not in manifest["sides"]:
        die(f"unknown side {side}")
    if subject_name not in manifest["subjects"]:
        die(f"unknown subject {subject_name}")
    ctx = dedicated_ctx(manifest)
    RESULTS.mkdir(parents=True, exist_ok=True)
    LOGS.mkdir(parents=True, exist_ok=True)
    legdef = manifest["legs"].get(leg) or die(f"unknown leg {leg}")
    if legdef.get("subjects") and subject_name not in legdef["subjects"]:
        die(f"leg {leg} does not apply to subject {subject_name}")
    if platform is None or platform not in legdef.get("platforms", []):
        die(f"leg {leg} requires --platform in {legdef.get('platforms')}")
    if sample not in range(1, manifest["samples_per_leg"] + 1):
        raise BenchError("sample must be in 1..samples_per_leg")
    identity = [side, subject_name, leg, platform, sample]
    if any(record_identity(rec) == tuple(identity) for rec in iter_records()):
        raise BenchError(f"sample already recorded: {identity}; use a fresh run for a repeat")
    state = load_state()
    if not all(inputs_locked(manifest, state).values()):
        raise BenchError("inputs/parity are not locked; finish setup, scaffold and parity")
    require_finalized(manifest, state)
    prerequisite = {"incremental-build": "cold-build", "preview-warm": "preview-cold", "launch": "package"}.get(leg)
    if prerequisite:
        require_predecessor(state, side, subject_name, platform, sample, prerequisite)
    state.pop("last_success", None)
    save_state(state)
    project = project_dir(manifest, side, subject_name)
    validate_backend_path(project / "Water.toml", backend_dir(side), side)
    verify_checkouts(state, side)
    if source_fingerprint(manifest, side, subject_name) != state["source_sha256"][f"{side}/{subject_name}"]:
        raise BenchError("application sources changed outside the defined incremental edit")
    lock = hashlib.sha256((project / "Cargo.lock").read_bytes()).hexdigest()
    if lock != state["lockfile_sha256"][f"{side}/{subject_name}"]:
        raise BenchError("application lockfile changed since scaffold")

    try:
        if leg == "cold-build":
            record = leg_build(manifest, ctx, side, subject_name,
                               platform, sample, True)
        elif leg == "incremental-build":
            record = leg_build(manifest, ctx, side, subject_name,
                               platform, sample, False)
        elif leg == "preview-cold":
            record = leg_preview(manifest, ctx, side, subject_name,
                                 sample, True)
        elif leg == "preview-warm":
            record = leg_preview(manifest, ctx, side, subject_name,
                                 sample, False)
        elif leg == "package":
            record = leg_package(manifest, ctx, side, subject_name,
                                 platform, sample)
        elif leg == "launch":
            record = leg_launch(manifest, ctx, side, subject_name,
                                platform, sample)
        else:
            die(f"leg {leg} not implemented")
    except (BenchError, OSError, ValueError, subprocess.SubprocessError) as exc:
        record = {"record_version": 3, "side": side, "subject": subject_name,
                  "leg": leg, "platform": platform, "sample": sample,
                  "diagnosis": str(exc), "failed_before_run": True}
    record["input_digest"] = input_digest(manifest, state)
    record["reproduce_argv"] = ["uv", "run", "--script", str(HERE / "drive.py"),
                                "measure", side, subject_name, leg,
                                "--platform", platform, "--sample", str(sample)]
    if hashlib.sha256((project / "Cargo.lock").read_bytes()).hexdigest() != lock:
        record["diagnosis"] = "application lockfile changed during measured command"
    if source_fingerprint(manifest, side, subject_name) != state["source_sha256"][f"{side}/{subject_name}"]:
        record["diagnosis"] = "application sources changed during measured command"
    write_record(side, subject_name, record)
    state = load_state()
    if record_ok(record):
        state["last_success"] = identity
    save_state(state)
    sys.exit(0 if record_ok(record) else 1)


def plan_steps(manifest):
    """Cold state and its consumer are indivisible; pair sides per platform."""
    for sample in range(1, manifest["samples_per_leg"] + 1):
        for subject in manifest["subjects"]:
            for platform in manifest["legs"]["cold-build"]["platforms"]:
                for side in manifest["sides"]:
                    for leg in ("cold-build", "incremental-build"):
                        yield side, subject, leg, platform, sample
        for subject in manifest["legs"]["preview-cold"]["subjects"]:
            for platform in manifest["legs"]["preview-cold"]["platforms"]:
                for side in manifest["sides"]:
                    for leg in ("preview-cold", "preview-warm"):
                        yield side, subject, leg, platform, sample
        for subject in manifest["subjects"]:
            for platform in manifest["legs"]["package"]["platforms"]:
                for side in manifest["sides"]:
                    yield side, subject, "package", platform, sample
                    if platform in manifest["legs"]["launch"]["platforms"]:
                        yield side, subject, "launch", platform, sample


def required_matrix(manifest):
    for subject in manifest["subjects"]:
        for leg, cfg in manifest["legs"].items():
            if subject not in cfg.get("subjects", manifest["subjects"]):
                continue
            for platform in cfg["platforms"]:
                yield (subject, leg, platform), cfg["required_metrics"]


def cmd_plan(manifest):
    prefix = ["uv", "run", "--script", str(HERE / "drive.py")]
    print("set -e")
    print(shlex_join([*prefix, "setup"]))
    print(shlex_join([*prefix, "finalize-inputs"]))
    for side in manifest["sides"]:
        print(shlex_join([*prefix, "scaffold", side]))
    print(shlex_join([*prefix, "parity"]))
    for side, subject, leg, platform, sample in plan_steps(manifest):
        print(shlex_join([*prefix, "measure", side, subject, leg,
                          "--platform", platform, "--sample", sample]))
    print(shlex_join([*prefix, "report"]))


def inputs_locked(manifest, state):
    """Coherent input locks: every checkout resolved+verified, all four
    lockfiles materialized, identical form source on both sides."""
    rp = state.get("resolved_pins", {})
    pins_ok = all(
        re.fullmatch(r"[0-9a-f]{40}", rp.get(side, {}).get(name, ""))
        and rp[side][name] not in manifest["sides"][side][name].get("rejected_shas", [])
        and (not manifest["sides"][side][name].get("sha")
             or rp[side][name] == manifest["sides"][side][name]["sha"])
        for side in manifest["sides"]
        for name in ("apple_backend", "waterui", "cli"))
    locks_ok = all(
        re.fullmatch(r"[0-9a-f]{64}", state.get("lockfile_sha256", {}).get(k, ""))
        for k in ("old/fresh", "old/form", "new/fresh", "new/form"))
    src = state.get("form_src_sha256", {})
    src_ok = len(src) == 2 and len(set(src.values())) == 1
    sources_ok = all(re.fullmatch(r"[0-9a-f]{64}", state.get("source_sha256", {}).get(f"{side}/{subject}", ""))
                     for side in manifest["sides"] for subject in manifest["subjects"])
    tools_ok = all(
        state.get("tools", {}).get(side, {}).get("source_sha") == rp.get(side, {}).get("cli")
        and bool(re.fullmatch(r"[0-9a-f]{64}", state.get("tools", {}).get(side, {}).get("binary_sha256", "")))
        for side in manifest["sides"])
    return {"inputs_finalized": state.get("inputs_finalized") is True,
            "cli_provenance": tools_ok,
            "parity_passed": state.get("parity_passed") is True,
            "source_fingerprints": sources_ok,
            "pins_resolved": bool(pins_ok), "lockfiles_materialized": locks_ok,
            "form_source_identical": src_ok}


def input_digest(manifest, state):
    data = {key: state.get(key) for key in (
        "resolved_pins", "lockfile_sha256", "form_src_sha256", "source_sha256", "tools")}
    data["manifest"] = manifest
    return hashlib.sha256(json.dumps(data, sort_keys=True).encode()).hexdigest()


def record_identity(rec):
    return tuple(rec.get(key) for key in ("side", "subject", "leg", "platform", "sample"))


def cmd_report(manifest):
    required_samples = set(range(1, manifest["samples_per_leg"] + 1))
    records = list(iter_records())
    state = load_state()
    locks = inputs_locked(manifest, state)
    digest = input_digest(manifest, state)
    grouped = {}
    errors = []
    expected = {
        (side, *key, sample)
        for key, _ in required_matrix(manifest)
        for side in manifest["sides"] for sample in required_samples}
    for rec in records:
        identity = record_identity(rec)
        if identity not in expected or type(rec.get("sample")) is not int:
            errors.append(f"unexpected sample identity: {identity}")
        grouped.setdefault(identity, []).append(rec)
    toolchain_consistent = bool(records) and bool(state.get("toolchain")) and all(
        rec.get("toolchain") == state["toolchain"] and rec.get("toolchain_match") is True
        for rec in records)
    coherent = all(locks.values()) and bool(records) and all(
        rec.get("input_digest") == digest for rec in records)
    if not records:
        errors.append("empty results")
    if not coherent:
        errors.append(f"inputs unlocked or record provenance diverged: {locks}")
    if not toolchain_consistent:
        errors.append("toolchain evidence missing or divergent")

    agg, incomplete = {}, []
    for key, metric_names in required_matrix(manifest):
        for metric in metric_names:
            entry, why = {}, []
            for side in manifest["sides"]:
                values = []
                for sample in sorted(required_samples):
                    rows = grouped.get((side, *key, sample), [])
                    if len(rows) != 1:
                        why.append(f"{side} sample {sample}: {len(rows)} records (requires one)")
                        continue
                    rec = rows[0]
                    value = rec.get("metrics", {}).get(metric)
                    complete_metrics = all(
                        type(rec.get("metrics", {}).get(name)) in (int, float)
                        and math.isfinite(rec["metrics"][name])
                        and rec["metrics"][name] >= 0
                        for name in metric_names)
                    if not record_ok(rec) or not complete_metrics:
                        why.append(f"{side} sample {sample}: failed or required metrics absent/invalid")
                        continue
                    values.append(value)
                if values:
                    entry[side] = {"n": len(values), "median": statistics.median(values),
                                   "min": min(values), "max": max(values)}
            why.extend(errors)
            if not why:
                old = entry["old"]["median"]
                # A zero-duration clock observation is valid but cannot be a divisor.
                if old == 0:
                    why.append("old median is zero; ratio undefined")
                else:
                    entry["ratio_new_over_old"] = entry["new"]["median"] / old
            entry["status"] = "incomplete" if why else "complete"
            if why:
                entry["why_incomplete"] = "; ".join(why)
                incomplete.append(f"{key}|{metric}")
            agg.setdefault("|".join(key), {})[metric] = entry

    RESULTS.mkdir(parents=True, exist_ok=True)
    out = RESULTS / "report.json"
    out.write_text(json.dumps({
        "issue": manifest["issue"], "generated_at": now_eastern(),
        "status": "incomplete" if incomplete or errors else "complete",
        "resolved_pins": state.get("resolved_pins"), "input_locks": locks,
        "toolchain_consistent": toolchain_consistent,
        "environment": state.get("toolchain"), "errors": errors, "metrics": agg,
        "commands": [rec.get("reproduce_argv") for rec in records],
        "app_features": {f"{rec['side']}/{rec['subject']}": rec.get("app_features")
                         for rec in records if "side" in rec and "subject" in rec},
        "build_scope": {side: cfg["build_scope"] for side, cfg in manifest["sides"].items()},
    }, indent=2) + "\n")
    lines = [
        "| Subject | Leg | Platform | Metric | Old median | New median "
        "| Ratio new/old | min–max old | min–max new | Status |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"]
    for key, metrics in sorted(agg.items()):
        subject, leg, platform = key.split("|")
        for metric, entry in sorted(metrics.items()):
            def cell(side):
                value = entry.get(side)
                return f"{value['median']:.0f}" if value else "—"
            def span(side):
                value = entry.get(side)
                return f"{value['min']:.0f}–{value['max']:.0f}" if value else "—"
            ratio = (f"{entry['ratio_new_over_old']:.3f}"
                     if "ratio_new_over_old" in entry else "—")
            lines.append(f"| {subject} | {leg} | {platform} | {metric} "
                         f"| {cell('old')} | {cell('new')} | {ratio} "
                         f"| {span('old')} | {span('new')} | {entry['status']} |")
    (RESULTS / "report.md").write_text("\n".join(lines) + "\n")
    print(f"report -> {out} and report.md")
    if incomplete or errors:
        raise BenchError(f"report incomplete: {len(incomplete)} metrics; {errors}")


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="command", required=True)
    for name in ("setup", "finalize-inputs"):
        preparation = sub.add_parser(name)
        preparation.add_argument("--cli-provenance", type=Path,
                                 help="coordinator receipts for existing installed binaries")
    p = sub.add_parser("scaffold")
    p.add_argument("side", choices=["old", "new"])
    sub.add_parser("parity")
    p = sub.add_parser("measure")
    p.add_argument("side", choices=["old", "new"])
    p.add_argument("subject")
    p.add_argument("leg")
    p.add_argument("--platform")
    p.add_argument("--sample", type=int, default=1)
    sub.add_parser("plan")
    sub.add_parser("report")
    args = ap.parse_args()

    manifest = load_manifest()
    try:
        if args.command in ("setup", "finalize-inputs"):
            cmd_setup(manifest, args.command == "finalize-inputs", args.cli_provenance)
        elif args.command == "scaffold":
            cmd_scaffold(manifest, args.side)
        elif args.command == "parity":
            cmd_parity(manifest)
        elif args.command == "measure":
            cmd_measure(manifest, args.side, args.subject, args.leg,
                        args.platform, args.sample)
        elif args.command == "plan":
            cmd_plan(manifest)
        elif args.command == "report":
            dedicated_ctx(manifest)
            cmd_report(manifest)
    except BenchError as exc:
        die(str(exc))


if __name__ == "__main__":
    main()
