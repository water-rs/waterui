#!/usr/bin/env python3
"""Compare deterministic engine memory across two Cherenkov revisions.

For Lavapipe, export
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
XDG_RUNTIME_DIR=/tmp/runtime-ubuntu RUST_LOG=error before running this
script. It passes the caller's environment through unchanged.
"""

from __future__ import annotations

import argparse
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
from typing import Any


SCENES = ("map", "chart", "text-page", "ui-list", "effects")
DEFAULT_ENGINES = ("cherenkov", "cherenkov-cpu")
# Lifecycle snapshots compared per scene, in report order (#169 A5).
# Older reports lack `preparation`, `warmup_peak` and `post_retire`; a
# phase present on only one side is a schema change and does not
# compare. `peak` stays: it is the merged high-water snapshot.
PHASES = ("idle", "preparation", "warmup_peak", "steady", "post_retire", "peak")
# One Performance-policy allocator block (#169 A5): the gate's
# over-reservation check uses it as the floor of what a live
# allocator may hold.
POLICY_BLOCK = 64 * 1024 * 1024
# wgpu allocator counters carry submission-timing noise: a free lands
# when the allocator observes its submission complete, which identical
# runs of the same binary can straddle by a few small allocations
# (dev's own harness has been observed to wobble by ~98 KiB and ±4
# allocations). Engine readings are the deterministic contract and
# compare exactly; allocator `reserved_bytes`/`blocks` are policy
# state and also compare exactly, while `allocated_bytes`/`allocations`
# get this slack on same-side reruns.
ALLOCATED_SLACK = 256 * 1024
ALLOCATIONS_SLACK = 8
ROOT_MARKER = ".memory-gate-root.json"
WORKTREE_MARKER = ".memory-gate-worktree.json"
HARNESS_HINT = "pass --harness <ref with #101>"


class GateError(Exception):
    pass


def command(
    args: list[str],
    *,
    cwd: Path | None = None,
    binary_output: bool = False,
) -> subprocess.CompletedProcess[Any]:
    return subprocess.run(
        args,
        cwd=cwd,
        check=False,
        capture_output=True,
        text=not binary_output,
    )


def git_output(repo: Path, *args: str) -> str:
    result = command(["git", "-C", str(repo), *args])
    if result.returncode:
        raise GateError(
            f"git {' '.join(args)} failed:\n"
            f"{result.stdout}{result.stderr}"
        )
    return result.stdout.strip()


def resolve_commit(repo: Path, ref: str) -> str:
    return git_output(repo, "rev-parse", "--verify", f"{ref}^{{commit}}")


def ensure_work_root(work_root: Path, repo: Path) -> None:
    work_root.mkdir(parents=True, exist_ok=True)
    marker = work_root / ROOT_MARKER
    expected = {"repo": str(repo.resolve())}
    if marker.exists():
        try:
            actual = json.loads(marker.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise GateError(f"invalid work directory marker {marker}: {error}") from error
        if actual != expected:
            raise GateError(
                f"{work_root} is owned by another repository; use a fresh --work directory"
            )
    else:
        existing = list(work_root.iterdir())
        if existing:
            raise GateError(
                f"{work_root} is not an owned memory-gate directory; "
                "use a fresh --work directory"
            )
        marker.write_text(json.dumps(expected, sort_keys=True) + "\n", encoding="utf-8")


def ensure_worktree(
    repo: Path, work_root: Path, side: str, commit: str
) -> Path:
    path = work_root / f"{side}-{commit[:12]}"
    marker = path / WORKTREE_MARKER
    expected = {"commit": commit, "repo": str(repo.resolve()), "side": side}
    if path.exists():
        if not path.is_dir() or not marker.is_file():
            raise GateError(
                f"refusing to reuse {path}: it is not a marked memory-gate worktree"
            )
        try:
            actual = json.loads(marker.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise GateError(f"invalid worktree marker {marker}: {error}") from error
        if actual != expected:
            raise GateError(f"worktree marker mismatch at {path}")
        top = Path(git_output(path, "rev-parse", "--show-toplevel")).resolve()
        head = git_output(path, "rev-parse", "HEAD")
        if top != path.resolve() or head != commit:
            raise GateError(f"worktree identity changed at {path}")
        return path

    result = command(
        ["git", "-C", str(repo), "worktree", "add", "--detach", str(path), commit]
    )
    if result.returncode:
        raise GateError(
            f"could not create {side} worktree at {path}:\n"
            f"{result.stdout}{result.stderr}"
        )
    marker.write_text(json.dumps(expected, sort_keys=True) + "\n", encoding="utf-8")
    return path


def ensure_clean_except_owned_files(path: Path, *, allow_bench: bool) -> None:
    result = command(
        ["git", "-C", str(path), "status", "--porcelain", "--untracked-files=normal"]
    )
    if result.returncode:
        raise GateError(f"could not inspect owned worktree {path}: {result.stderr}")
    unexpected = []
    for line in result.stdout.splitlines():
        name = line[3:]
        if name == WORKTREE_MARKER:
            continue
        if allow_bench and (name == "bench" or name.startswith("bench/")):
            continue
        unexpected.append(line)
    if unexpected:
        raise GateError(
            f"refusing to build modified {path} worktree:\n" + "\n".join(unexpected)
        )


def overlay_bench(repo: Path, harness: str, base: Path) -> None:
    result = command(
        ["git", "-C", str(repo), "archive", "--format=tar", harness, "bench"],
        binary_output=True,
    )
    if result.returncode:
        raise GateError(f"could not archive bench from harness {harness}:\n{result.stderr}")
    with tempfile.TemporaryDirectory(prefix=".memory-gate-overlay-", dir=base) as temporary:
        temporary_path = Path(temporary)
        archive_root = temporary_path.resolve()
        try:
            with tarfile.open(fileobj=io.BytesIO(result.stdout), mode="r:") as archive:
                members = archive.getmembers()
                for member in members:
                    destination = (temporary_path / member.name).resolve()
                    if not destination.is_relative_to(archive_root):
                        raise GateError(f"unsafe path in harness archive: {member.name}")
                    if member.issym() or member.islnk():
                        target = (destination.parent / member.linkname).resolve()
                        if not target.is_relative_to(archive_root):
                            raise GateError(
                                f"unsafe link in harness archive: {member.name}"
                            )
                archive.extractall(temporary_path)
        except (OSError, tarfile.TarError) as error:
            raise GateError(f"could not extract harness bench archive: {error}") from error

        replacement = temporary_path / "bench"
        if not replacement.is_dir():
            raise GateError(f"harness {harness} contains no bench directory")
        destination = base / "bench"
        backup = base / ".memory-gate-bench-old"
        if backup.exists():
            raise GateError(f"refusing to replace leftover path {backup}")
        destination.rename(backup)
        try:
            replacement.rename(destination)
        except OSError:
            backup.rename(destination)
            raise
        shutil.rmtree(backup)


def cargo_target_directory(path: Path) -> Path:
    result = command(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=path
    )
    if result.returncode:
        raise GateError(f"cargo metadata failed in {path}:\n{result.stdout}{result.stderr}")
    metadata = json.loads(result.stdout)
    return Path(metadata["target_directory"])


def build(path: Path, out: Path, side: str) -> Path:
    log_path = out / f"build-{side}.log"
    result = command(
        [
            "cargo",
            "build",
            "--locked",
            "--release",
            "-p",
            "cherenkov-bench",
            "--features",
            "cherenkov,cherenkov-cpu",
        ],
        cwd=path,
    )
    log_path.write_text(result.stdout + result.stderr, encoding="utf-8")
    if result.returncode:
        raise GateError(
            f"release build failed for {side}; see {log_path}\n"
            f"{result.stdout}{result.stderr}"
        )
    executable = cargo_target_directory(path) / "release" / "cherenkov-bench"
    if os.name == "nt":
        executable = executable.with_suffix(".exe")
    if not executable.is_file():
        raise GateError(f"build succeeded but executable is missing: {executable}")
    return executable


def counter_engine_memory(
    report_path: Path, report: dict[str, Any]
) -> dict[str, Any] | None:
    counters = report.get("counters")
    if not isinstance(counters, dict):
        return None
    has_cpu = "memory_cpu_bytes" in counters
    has_gpu = "memory_gpu_bytes" in counters
    if not has_cpu and not has_gpu:
        return None
    if not has_cpu or not has_gpu:
        raise GateError(f"{report_path} has incomplete CPU/GPU engine memory counters")
    cpu_bytes = counters["memory_cpu_bytes"]
    gpu_bytes = counters["memory_gpu_bytes"]
    if cpu_bytes is None and gpu_bytes is None:
        return None
    if (
        not isinstance(cpu_bytes, int)
        or isinstance(cpu_bytes, bool)
        or not isinstance(gpu_bytes, int)
        or isinstance(gpu_bytes, bool)
        or cpu_bytes < 0
        or gpu_bytes < 0
    ):
        raise GateError(f"{report_path} has invalid CPU/GPU engine memory counters")
    measured = {"cpu_bytes": cpu_bytes, "gpu_bytes": gpu_bytes}
    # `backdrop_capture_bytes` is optional: binaries predating its
    # introduction do not serialize it, and adapters without backdrop
    # groups may leave it out.
    if "memory_backdrop_capture_bytes" in counters:
        captures = counters["memory_backdrop_capture_bytes"]
        if captures is not None:
            if (
                not isinstance(captures, int)
                or isinstance(captures, bool)
                or captures < 0
            ):
                raise GateError(
                    f"{report_path} has an invalid backdrop capture counter"
                )
            measured["backdrop_capture_bytes"] = captures
    return {"measured": measured}


def read_reading(node: Any, path: Path, name: str) -> dict[str, Any] | str | None:
    """One `Reading<T>`: `{"measured": {...}}`, `{"unavailable": reason}`,
    or `None` when the report does not carry the reading at all."""
    if not isinstance(node, dict):
        return None
    if "unavailable" in node:
        return {"unavailable": node["unavailable"]}
    measured = node.get("measured")
    if not isinstance(measured, dict):
        raise GateError(f"{path} has an invalid {name} reading")
    return measured


def read_phase_memory(report_path: Path, report: dict) -> dict[str, Any]:
    """Per-phase engine and wgpu-allocator readings of one report.

    Returns `{phase: {"engine": {...}, "allocator": {...}}}` — engine
    entries keep the `{cpu_bytes, gpu_bytes[, backdrop_capture_bytes]}`
    shape; allocator entries keep `{allocated_bytes, reserved_bytes,
    allocations, blocks}`. A required reading that reports `unavailable`
    fails the gate through `unavailable:` — it is not a pass (#169 A5).
    """
    memory = report.get("memory")
    if not isinstance(memory, dict):
        counters = counter_engine_memory(report_path, report)
        if counters is not None:
            return {"steady": {"engine": counters["measured"], "allocator": None}}
        raise GateError(f"{report_path} has no memory report; {HARNESS_HINT}")
    phases: dict[str, Any] = {}
    for phase in PHASES:
        snapshot = memory.get(phase)
        if snapshot is None:
            continue
        if not isinstance(snapshot, dict):
            raise GateError(f"{report_path} has an invalid {phase} memory snapshot")
        entry: dict[str, Any] = {"engine": None, "allocator": None}
        engine = read_reading(snapshot.get("engine"), report_path, f"{phase} engine")
        if engine is not None:
            if "unavailable" in engine:
                entry["engine"] = engine
            else:
                cpu_bytes = engine.get("cpu_bytes")
                gpu_bytes = engine.get("gpu_bytes")
                if (
                    not isinstance(cpu_bytes, int)
                    or isinstance(cpu_bytes, bool)
                    or not isinstance(gpu_bytes, int)
                    or isinstance(gpu_bytes, bool)
                    or cpu_bytes < 0
                    or gpu_bytes < 0
                ):
                    raise GateError(f"{report_path} has invalid {phase} CPU/GPU engine bytes")
                measured = {"cpu_bytes": cpu_bytes, "gpu_bytes": gpu_bytes}
                captures = engine.get("backdrop_capture_bytes")
                if captures is not None:
                    if (
                        not isinstance(captures, int)
                        or isinstance(captures, bool)
                        or captures < 0
                    ):
                        raise GateError(
                            f"{report_path} has invalid {phase} backdrop capture bytes"
                        )
                    measured["backdrop_capture_bytes"] = captures
                entry["engine"] = measured
        allocator = read_reading(
            snapshot.get("wgpu_allocator"), report_path, f"{phase} wgpu allocator"
        )
        if allocator is not None:
            if "unavailable" in allocator:
                entry["allocator"] = allocator
            else:
                fields = {}
                for key in ("allocated_bytes", "reserved_bytes", "allocations", "blocks"):
                    value = allocator.get(key)
                    if (
                        not isinstance(value, int)
                        or isinstance(value, bool)
                        or value < 0
                    ):
                        raise GateError(
                            f"{report_path} has invalid {phase} allocator {key}"
                        )
                    fields[key] = value
                entry["allocator"] = fields
        phases[phase] = entry
    if not phases:
        raise GateError(f"{report_path} has no memory snapshot")
    counters = counter_engine_memory(report_path, report)
    if counters is not None:
        steady = phases.get("steady", {}).get("engine")
        if isinstance(steady, dict) and "unavailable" not in steady:
            # Compare only the keys present on both sides: optional fields
            # like `backdrop_capture_bytes` may be absent from one of them.
            shared = set(counters["measured"]) & set(steady)
            if any(
                counters["measured"][key] != steady[key] for key in shared
            ):
                raise GateError(
                    f"{report_path} has inconsistent memory readings: "
                    f"memory.steady.engine={steady}, counters={counters}"
                )
    return phases


def same_side_equal(first: dict[str, Any], second: dict[str, Any]) -> bool:
    """Whether two measure runs of the same side agree.

    Engine readings and allocator `reserved_bytes`/`blocks` compare
    exactly; `allocated_bytes`/`allocations` allow the submission-timing
    slack documented at ALLOCATED_SLACK. Phases and unavailable
    reasons must match exactly.
    """
    if set(first) != set(second):
        return False
    for phase, a in first.items():
        b = second[phase]
        if not isinstance(a, dict) or not isinstance(b, dict):
            return a == b
        if a.get("engine") != b.get("engine"):
            return False
        alloc_a, alloc_b = a.get("allocator"), b.get("allocator")
        if isinstance(alloc_a, dict) != isinstance(alloc_b, dict):
            return False
        if alloc_a is None or alloc_b is None:
            if alloc_a != alloc_b:
                return False
            continue
        for key in ("allocated_bytes", "reserved_bytes", "allocations", "blocks"):
            if key not in alloc_a or key not in alloc_b:
                return alloc_a == alloc_b
        if "unavailable" in alloc_a:
            if alloc_a != alloc_b:
                return False
            continue
        if (
            abs(alloc_a["allocated_bytes"] - alloc_b["allocated_bytes"])
            > ALLOCATED_SLACK
            or alloc_a["reserved_bytes"] != alloc_b["reserved_bytes"]
            or abs(alloc_a["allocations"] - alloc_b["allocations"])
            > ALLOCATIONS_SLACK
            or alloc_a["blocks"] != alloc_b["blocks"]
        ):
            return False
    return True


def allocator_verdict(allocator: dict[str, int]) -> str | None:
    """The #169 acceptance rule on one allocator reading.

    Reserved storage may hold roughly one Performance-policy block of
    slack over what is live; a report that reserves far more than its
    simultaneously-live requirement (roughly 38 MiB allocated over
    192 MiB reserved) fails. Block *count* alone never fails — a policy
    that emits several small blocks keeps a reserved total near
    `allocated` and passes.
    """
    allocated = allocator["allocated_bytes"]
    reserved = allocator["reserved_bytes"]
    if reserved > 2 * max(allocated, POLICY_BLOCK):
        return (
            f"over-reserved: {reserved:,} B reserved over "
            f"{allocated:,} B live in {allocator['blocks']} block(s)"
        )
    return None


def measure_side(
    side: str,
    root: Path,
    binary: Path,
    scene_root: Path,
    engines: tuple[str, ...],
    warmup: int,
    frames: int,
    out: Path,
) -> dict[tuple[str, str], dict[str, Any] | str]:
    readings: dict[tuple[str, str], dict[str, Any] | str] = {}
    for scene in SCENES:
        scene_path = scene_root / "scenes" / "perf" / scene
        for engine in engines:
            samples: list[dict[str, Any] | str] = []
            for attempt in (1, 2):
                report_path = out / f"{side}-{scene}-{engine}-{attempt}.json"
                log_path = out / f"{side}-{scene}-{engine}-{attempt}.log"
                result = command(
                    [
                        str(binary),
                        "measure",
                        "--engine",
                        engine,
                        "--scene",
                        str(scene_path),
                        "--warmup",
                        str(warmup),
                        "--frames",
                        str(frames),
                        "--out",
                        str(report_path),
                    ],
                    cwd=root,
                )
                log_path.write_text(result.stdout + result.stderr, encoding="utf-8")
                if result.returncode:
                    samples.append(
                        f"command error: measure exited {result.returncode}; "
                        f"see {log_path}"
                    )
                    continue
                try:
                    report = json.loads(report_path.read_text(encoding="utf-8"))
                except (OSError, json.JSONDecodeError) as error:
                    samples.append(f"report error: could not read {report_path}: {error}")
                    continue
                if not isinstance(report, dict):
                    samples.append(f"report error: {report_path} has no memory report")
                    continue
                try:
                    samples.append(read_phase_memory(report_path, report))
                except GateError as error:
                    samples.append(f"report error: {error}")
            key = (scene, engine)
            if len(samples) != 2:
                readings[key] = "nondeterministic: expected two samples"
            elif isinstance(samples[0], dict) != isinstance(samples[1], dict):
                readings[key] = "nondeterministic: same-side memory readings differ"
            elif isinstance(samples[0], dict) and not same_side_equal(
                samples[0], samples[1]
            ):
                readings[key] = "nondeterministic: same-side memory readings differ"
            elif isinstance(samples[0], str) and isinstance(samples[1], str):
                first_kind = samples[0].split(":", maxsplit=1)[0]
                second_kind = samples[1].split(":", maxsplit=1)[0]
                if first_kind != second_kind:
                    readings[key] = "nondeterministic: same-side measure outcomes differ"
                else:
                    readings[key] = samples[0]
            elif isinstance(samples[0], str):
                readings[key] = samples[0]
            elif "unavailable" in samples[0]:
                readings[key] = (
                    f"unavailable: {samples[0]['unavailable']}"
                )
            else:
                readings[key] = samples[0]
    return readings


def format_bytes(value: int | None) -> str:
    return "—" if value is None else f"{value:,}"


def markdown_table(
    base_ref: str,
    head_ref: str,
    rows: list[tuple[str, str, str, str, str, str, str, str, str]],
) -> str:
    lines = [
        "# Engine memory landing gate",
        "",
        f"- Base: `{base_ref}`",
        f"- Head: `{head_ref}`",
        "",
        "| Scene | Engine | Phase | Base CPU (B) | Base GPU (B) | Head CPU (B) | Head GPU (B) | Base reserved (B) | Delta |",
        "|---|---|---|---:|---:|---:|---:|---:|---|",
    ]
    for (
        scene,
        engine,
        phase,
        base_cpu,
        base_gpu,
        head_cpu,
        head_gpu,
        base_reserved,
        delta,
    ) in rows:
        delta = delta.replace("|", "\\|")
        lines.append(
            f"| {scene} | {engine} | {phase} | {base_cpu} | {base_gpu} | "
            f"{head_cpu} | {head_gpu} | {base_reserved} | {delta} |"
        )
    lines.append("")
    return "\n".join(lines)


def self_test() -> int:
    """The #169 gate fixture: roughly 38 MiB allocated over 192 MiB
    reserved in 2 blocks must fail even when `Engine::memory()` is
    unchanged, and several small blocks must not be rejected for their
    count alone."""
    mib = 1024 * 1024
    fixture = {
        "allocated_bytes": int(37.6 * mib),
        "reserved_bytes": 192 * mib,
        "allocations": 300,
        "blocks": 2,
    }
    small_blocks = {
        "allocated_bytes": 150 * mib,
        "reserved_bytes": 160 * mib,
        "allocations": 4000,
        "blocks": 6,
    }
    sane = {
        "allocated_bytes": int(41.5 * mib),
        "reserved_bytes": 64 * mib,
        "allocations": 200,
        "blocks": 1,
    }
    failures = []
    verdict = allocator_verdict(fixture)
    if verdict is None:
        failures.append("fixture 37.6MiB/192MiB/2-block did not fail")
    else:
        print(f"fixture 37.6MiB/192MiB/2 blocks: FAIL (as required): {verdict}")
    if allocator_verdict(small_blocks) is not None:
        failures.append("small-block reading was rejected on count alone")
    else:
        print("fixture 150MiB/160MiB/6 blocks: pass (small blocks allowed)")
    if allocator_verdict(sane) is not None:
        failures.append("one-64MiB-block reading was rejected")
    else:
        print("fixture 41.5MiB/64MiB/1 block: pass")
    for failure in failures:
        print(f"self-test failure: {failure}")
    print(f"exit code: {1 if failures else 0}")
    return 1 if failures else 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", help="base git ref")
    parser.add_argument("--head", help="head git ref")
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run the #169 fixture check and exit",
    )
    parser.add_argument(
        "--harness",
        help="ref with #101 bench sources to overlay onto the base worktree",
    )
    parser.add_argument(
        "--engines", default=",".join(DEFAULT_ENGINES), help="comma-separated adapters"
    )
    parser.add_argument("--warmup", type=int, default=30)
    parser.add_argument("--frames", type=int, default=30)
    parser.add_argument("--out", type=Path, help="report output directory")
    parser.add_argument(
        "--work", type=Path, default=Path("/home/ubuntu/memory-gate")
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.self_test:
        return self_test()
    if not args.base or not args.head:
        raise GateError("--base and --head are required")
    if args.out is None:
        raise GateError("--out is required")
    if args.warmup < 0 or args.frames <= 0:
        raise GateError("--warmup must be nonnegative and --frames must be positive")
    engines = tuple(
        engine.strip() for engine in args.engines.split(",") if engine.strip()
    )
    if not engines:
        raise GateError("--engines must name at least one adapter")
    unknown_engines = set(engines) - set(DEFAULT_ENGINES)
    if unknown_engines:
        raise GateError(
            f"this build supports only {', '.join(DEFAULT_ENGINES)}; "
            f"unknown engines: {', '.join(sorted(unknown_engines))}"
        )

    repo = Path(__file__).resolve().parents[2]
    repo = Path(git_output(repo, "rev-parse", "--show-toplevel")).resolve()
    work_root = args.work.expanduser().resolve()
    out = args.out.expanduser().resolve()
    out.mkdir(parents=True, exist_ok=True)
    ensure_work_root(work_root, repo)

    base_commit = resolve_commit(repo, args.base)
    head_commit = resolve_commit(repo, args.head)
    harness_commit = resolve_commit(repo, args.harness) if args.harness else None
    base = ensure_worktree(repo, work_root, "base", base_commit)
    head = ensure_worktree(repo, work_root, "head", head_commit)
    ensure_clean_except_owned_files(base, allow_bench=harness_commit is not None)
    ensure_clean_except_owned_files(head, allow_bench=False)
    if harness_commit is not None:
        overlay_bench(repo, harness_commit, base)

    scene_root = head
    if not all(
        (scene_root / "scenes" / "perf" / scene / "scene.json").is_file()
        for scene in SCENES
    ):
        raise GateError(f"head worktree {head} is missing one or more perf scenes")

    base_binary = build(base, out, "base")
    base_readings = measure_side(
        "base", base, base_binary, scene_root, engines, args.warmup, args.frames, out
    )
    head_binary = build(head, out, "head")
    head_readings = measure_side(
        "head", head, head_binary, scene_root, engines, args.warmup, args.frames, out
    )

    rows = []
    failed = False
    for scene in SCENES:
        for engine in engines:
            key = (scene, engine)
            base_value = base_readings[key]
            head_value = head_readings[key]
            if isinstance(base_value, str) or isinstance(head_value, str):
                failed = True
                error = "; ".join(
                    value for value in (base_value, head_value) if isinstance(value, str)
                )
                rows.append(
                    (scene, engine, "—", "—", "—", "—", "—", "—", f"ERROR: {error}")
                )
                continue

            base_phases = base_value
            head_phases = head_value
            for phase in PHASES:
                base_phase = base_phases.get(phase)
                head_phase = head_phases.get(phase)
                if base_phase is None and head_phase is None:
                    continue
                if base_phase is None or head_phase is None:
                    # A phase present on only one side is a schema delta,
                    # not a regression — report it without failing.
                    side = "base" if base_phase is None else "head"
                    rows.append(
                        (scene, engine, phase, "—", "—", "—", "—", "—",
                         f"{phase} absent on {side}")
                    )
                    continue
                problems = []
                delta_parts = []
                for name in ("engine", "allocator"):
                    base_reading = base_phase[name]
                    head_reading = head_phase[name]
                    if isinstance(base_reading, dict) != isinstance(head_reading, dict):
                        if base_reading is None or head_reading is None:
                            continue
                        problems.append(
                            f"{name}: one side unavailable "
                            f"({base_reading or head_reading})"
                        )
                        continue
                    if base_reading is None:
                        continue
                    if "unavailable" in base_reading or "unavailable" in head_reading:
                        if "unavailable" in base_reading and "unavailable" in head_reading:
                            # Equally unavailable on both sides — an
                            # engine with no such counter is correct,
                            # not a failure (e.g. cherenkov-cpu has no
                            # wgpu allocator to report).
                            delta_parts.append(
                                f"{name} unavailable: "
                                f"{base_reading['unavailable']}"
                            )
                        else:
                            # A required measurement the other side
                            # produces is not a pass (#169 A5).
                            problems.append(
                                f"{name} unavailable: "
                                f"{base_reading.get('unavailable') or head_reading.get('unavailable')}"
                            )
                        continue
                    fields = sorted(set(base_reading) | set(head_reading))
                    deltas = []
                    for field in fields:
                        before = base_reading.get(field)
                        after = head_reading.get(field)
                        if before != after:
                            deltas.append(
                                f"{field} {format_bytes(before)} -> {format_bytes(after)}"
                            )
                    if deltas:
                        delta_parts.append(f"{name}: " + "; ".join(deltas))
                        # Engine bytes are the deterministic contract —
                        # any change fails. Allocator counters are
                        # observations the change explains; they gate
                        # only through the reserved-over-live verdict.
                        if name == "engine":
                            problems.append(f"{name}: " + "; ".join(deltas))
                for side, reading in (("base", base_phase), ("head", head_phase)):
                    allocator = reading["allocator"]
                    if isinstance(allocator, dict) and "unavailable" not in allocator:
                        verdict = allocator_verdict(allocator)
                        if verdict is not None:
                            problems.append(f"{side} {phase}: {verdict}")
                if problems:
                    failed = True
                engine_base = base_phase["engine"] or {}
                engine_head = head_phase["engine"] or {}
                rows.append(
                    (
                        scene,
                        engine,
                        phase,
                        format_bytes(engine_base.get("cpu_bytes")),
                        format_bytes(engine_base.get("gpu_bytes")),
                        format_bytes(engine_head.get("cpu_bytes")),
                        format_bytes(engine_head.get("gpu_bytes")),
                        format_bytes(
                            (base_phase["allocator"] or {}).get("reserved_bytes")
                            if isinstance(base_phase["allocator"], dict)
                            else None
                        ),
                        "; ".join(problems) if problems else "match",
                    )
                )

    table_path = out / "memory-gate.md"
    table = markdown_table(args.base, args.head, rows)
    table_path.write_text(table, encoding="utf-8")
    print(table, end="")
    print(f"table: {table_path}")
    print(f"exit code: {1 if failed else 0}")
    return 1 if failed else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except GateError as error:
        print(f"memory gate: {error}", file=sys.stderr)
        sys.exit(1)
