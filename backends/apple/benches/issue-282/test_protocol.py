"""Source-only protocol tests. All command execution and measurements are mocked.

Run: uv run --with tomli-w==1.2.0 python -B -m unittest discover -s benches/issue-282
"""

import copy
import io
import json
import os
import tempfile
import tomllib
import unittest
from contextlib import ExitStack, redirect_stdout
from pathlib import Path
from unittest.mock import MagicMock, patch

import drive as d


class ProtocolTests(unittest.TestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory(dir=d.HERE))).resolve()
        for name, value in {"ROOT": self.root, "RESULTS": self.root / "results",
                            "LOGS": self.root / "logs", "STATE_PATH": self.root / "state.json"}.items():
            self.stack.enter_context(patch.object(d, name, value))
        d.RESULTS.mkdir()
        d.LOGS.mkdir()
        self.m = d.load_manifest()
        # No accidental local measurement or build may escape a test.
        for name in ("run", "du_bytes", "probe_toolchain"):
            self.stack.enter_context(patch.object(d, name, side_effect=AssertionError("real measurement prohibited")))
        self.stack.enter_context(patch.object(d.subprocess, "Popen", side_effect=AssertionError("real process prohibited")))
        self.stack.enter_context(patch.object(d.time, "sleep", side_effect=AssertionError("sleep prohibited")))

    def locked_state(self):
        return {"resolved_pins": {
            side: {name: cfg[name].get("sha") or "a" * 40
                   for name in ("apple_backend", "waterui", "cli")}
            for side, cfg in self.m["sides"].items()},
            "lockfile_sha256": {f"{side}/{subject}": "b" * 64
                                for side in self.m["sides"] for subject in self.m["subjects"]},
            "form_src_sha256": {"old": "c" * 64, "new": "c" * 64},
            "source_sha256": {f"{side}/{subject}": "d" * 64
                              for side in self.m["sides"] for subject in self.m["subjects"]},
            "inputs_finalized": True,
            "tools": {side: {"source_sha": cfg["cli"].get("sha") or "a" * 40,
                             "binary_sha256": "e" * 64}
                      for side, cfg in self.m["sides"].items()},
            "parity_passed": True, "toolchain": {"rustc": "test-toolchain"}}

    def report_records(self):
        state = self.locked_state()
        d.save_state(state)
        return [dict(side=side, subject=subject, leg=leg, platform=platform,
                     sample=sample, exit_code=0, toolchain=state["toolchain"],
                     toolchain_match=True, input_digest=d.input_digest(self.m, state),
                     metrics={metric: 10 + sample for metric in self.m["legs"][leg]["required_metrics"]})
                for side, subject, leg, platform, sample in d.plan_steps(self.m)]

    def report(self, rows, fails=False):
        with patch.object(d, "iter_records", return_value=iter(rows)), redirect_stdout(io.StringIO()):
            if fails:
                with self.assertRaises(d.BenchError):
                    d.cmd_report(self.m)
            else:
                d.cmd_report(self.m)
        return json.loads((d.RESULTS / "report.json").read_text())

    def test_complete_matrix(self):
        report = self.report(self.report_records())
        self.assertEqual(report["status"], "complete")
        self.assertEqual(sum(len(v) for v in report["metrics"].values()), 54)

    def test_empty_report_fails_with_full_matrix(self):
        d.save_state(self.locked_state())
        report = self.report([], fails=True)
        self.assertEqual(sum(len(v) for v in report["metrics"].values()), 54)

    def test_absent_metric_leg_side_and_duplicate_samples(self):
        original = self.report_records()
        variants = []
        absent = copy.deepcopy(original)
        for row in absent:
            row["metrics"].pop("steady_rss_bytes", None)
        variants.append(absent)
        variants.append([r for r in original if r["leg"] != "launch"])
        variants.append([r for r in original if r["side"] == "old"])
        variants.append(original + [original[0]])
        repeated = copy.deepcopy(original)
        repeated[0]["sample"] = 2
        variants.append(repeated)
        for rows in variants:
            with self.subTest(variant=len(rows)):
                self.report(rows, fails=True)

    def test_failed_nonfinite_and_unlocked_records_fail(self):
        for field, value in (("diagnosis", "failed"), ("exit_code", None),
                             ("input_digest", "wrong"), ("toolchain", {})):
            rows = self.report_records()
            rows[0][field] = value
            self.report(rows, fails=True)
        for value in (float("nan"), float("inf"), True, None, -1):
            rows = self.report_records()
            rows[0]["metrics"]["wall_ms"] = value
            self.report(rows, fails=True)

    def test_plan_exact_coverage_and_immediate_consumers(self):
        steps = list(d.plan_steps(self.m))
        expected = {(side, *key, n) for key, _ in d.required_matrix(self.m)
                    for side in self.m["sides"] for n in range(1, 6)}
        self.assertEqual(set(steps), expected)
        self.assertEqual(len(steps), len(expected))
        self.assertEqual(len(steps), 200)
        for index, (side, subject, leg, platform, sample) in enumerate(steps):
            predecessor = {"launch": "package", "incremental-build": "cold-build",
                           "preview-warm": "preview-cold"}.get(leg)
            if predecessor:
                self.assertEqual(steps[index - 1], (side, subject, predecessor, platform, sample))

    def test_warm_history_cannot_survive_another_leg(self):
        state = {"last_success": ["old", "fresh", "cold-build", "macos", 1]}
        d.require_predecessor(state, "old", "fresh", "macos", 1, "cold-build")
        with self.assertRaises(d.BenchError):
            d.require_predecessor(state, "old", "fresh", "macos", 2, "cold-build")

    def test_old_toml_exact_table_and_quoted_paths(self):
        path = self.root / "Water.toml"
        backend = self.root / 'a "quoted" path'
        d.write_toml(path, {"package": {"name": "A", "type": "app"},
                           "backends": {"android": {"backend_path": "unrelated"},
                                        "apple": {"backend_path": "wrong"}}})
        d.ensure_backend_path(path, backend, "bench282", "old")
        data = tomllib.loads(path.read_text())
        self.assertEqual(data["backends"]["apple"],
                         {"backend_path": str(backend), "scheme": "bench282"})
        self.assertEqual(data["backends"]["android"]["backend_path"], "unrelated")
        variants = []
        for label, edit in (
                ("wrong type", lambda v: v["package"].update({"type": "playground"})),
                ("missing type", lambda v: v["package"].pop("type")),
                ("wrong backend_path", lambda v: v["backends"]["apple"].update({"backend_path": "wrong"})),
                ("missing scheme", lambda v: v["backends"]["apple"].pop("scheme"))):
            variant = copy.deepcopy(data)
            edit(variant)
            variants.append((label, variant))
        for label, variant in variants:
            d.write_toml(path, variant)
            with self.subTest(broken=label):
                with self.assertRaises(d.BenchError):
                    d.validate_backend_path(path, backend, "old")

    def test_new_manifest_uses_waterui_path_and_owned_link(self):
        backend = d.backend_dir("new")
        backend.mkdir(parents=True)
        waterui = d.waterui_dir("new")
        link = waterui / "backends" / "apple"
        link.parent.mkdir(parents=True)
        link.symlink_to(backend, target_is_directory=True)
        path = self.root / "Water.toml"
        d.write_toml(path, {"waterui_path": str(waterui),
                           "package": {"name": "A",
                                       "bundle_identifier": "dev.waterui.a"}})
        d.ensure_backend_path(path, backend, "unused", "new")
        data = tomllib.loads(path.read_text())
        self.assertNotIn("backends", data)
        self.assertNotIn("type", data["package"])
        base = {"waterui_path": str(waterui), "package": {"name": "A"}}
        for retired in ({"backends": {"apple": {"backend_path": str(backend)}}},
                        {"backends": {"apple": {"scheme": "bench282"}}},
                        {"backends": {"path": "backends"}},
                        {"package": {"type": "app"}}):
            variant = copy.deepcopy(base)
            for table, edits in retired.items():
                variant[table] = variant.get(table, {}) | edits
            d.write_toml(path, variant)
            with self.subTest(retired=retired):
                with self.assertRaises(d.BenchError):
                    d.validate_backend_path(path, backend, "new")
        d.write_toml(path, {**base, "waterui_path": str(self.root / "foreign")})
        with self.assertRaises(d.BenchError):
            d.validate_backend_path(path, backend, "new")
        for replacement in (self.root / "elsewhere", waterui / "backends"):
            link.unlink()
            link.symlink_to(replacement, target_is_directory=True)
            d.write_toml(path, base)
            with self.subTest(link_target=replacement):
                with self.assertRaises(d.BenchError):
                    d.validate_backend_path(path, backend, "new")
        link.unlink()
        link.parent.rmdir()
        d.write_toml(path, base)
        with self.assertRaises(d.BenchError):
            d.validate_backend_path(path, backend, "new")

    def test_form_manifest_schemas_and_source_parity(self):
        origin = self.root / "checkouts/old/waterui/examples/form/src"
        origin.mkdir(parents=True)
        source = 'fn app() { text("WaterUI Form Examples"); }\n'
        (origin / "lib.rs").write_text(source)
        for side in ("old", "new"):
            if side == "new":
                backend = d.backend_dir("new")
                backend.mkdir(parents=True)
                link = d.waterui_dir("new") / "backends" / "apple"
                link.parent.mkdir(parents=True)
                link.symlink_to(backend, target_is_directory=True)
            dest = d.stage_form(self.m, side)
            data = tomllib.loads((dest / "Water.toml").read_text())
            self.assertEqual("type" in data["package"], side == "old")
            self.assertEqual(data["waterui_path"], str(d.waterui_dir(side)))
            if side == "old":
                self.assertEqual(data["backends"]["apple"],
                                 {"backend_path": str(d.backend_dir("old")),
                                  "scheme": "form_example"})
            else:
                self.assertNotIn("backends", data)
            self.assertEqual((dest / "src/lib.rs").read_text(), source)
            d.set_source_variant(self.m, side, "form", True)
            self.assertIn('text("WaterUI Form Examples!")', (dest / "src/lib.rs").read_text())
            with self.assertRaises(d.BenchError):
                d.set_source_variant(self.m, side, "form", True)
            d.set_source_variant(self.m, side, "form", False)
            self.assertEqual((dest / "src/lib.rs").read_text(), source)

    def event(self, pid, value=17, subsystem="dev.waterui"):
        return json.dumps({"processID": pid, "subsystem": subsystem,
                           "eventMessage": f"waterui_first_paint_ms={value}"}).encode() + b"\n"

    def test_fragmented_stream_retains_early_marker_and_filters_pid(self):
        proc = MagicMock()
        parser = d.StructuredLogStream(proc)
        chunks = [b"Filtering the log ", b"data\n" + self.event(91, 999) + self.event(92, 777, "foreign") + self.event(92)[:15],
                  self.event(92)[15:]]
        with patch.object(d.select, "select", return_value=([proc.stdout], [], [])), \
                patch.object(d.os, "read", side_effect=chunks):
            parser.await_attach(float("inf"))
            self.assertEqual(parser.first_paint(92, self.m["legs"]["launch"], float("inf")), 17)

    def launch_fixture(self, platform, fail=None):
        app = self.root / "Test.app"
        app.mkdir(exist_ok=True)
        d.save_state({"packaged": {f"old|fresh|{platform}|1": {"app": str(app), "exe": "Test"}},
                      "simulator_udid": "test-simulator"})
        stream, process = MagicMock(), MagicMock(pid=92)
        calls, stopped = [], []
        parser = MagicMock()
        def attach(deadline):
            calls.append("attach")
            if fail == "attach":
                raise d.BenchError("attach failed")
        def paint(pid, cfg, deadline):
            calls.append("paint")
            self.assertEqual(pid, 92)
            if fail == "marker":
                raise d.BenchError("marker missing")
            return 17
        parser.await_attach.side_effect = attach
        parser.first_paint.side_effect = paint
        def popen(argv, **kwargs):
            if "stream" in argv:
                self.assertNotIn("processID", " ".join(argv))
                self.assertIn("ndjson", argv)
                calls.append("stream")
                return stream
            self.assertIn("attach", calls)
            calls.append("spawn")
            if fail == "spawn":
                raise OSError("spawn failed")
            return process
        def run(argv, *args, **kwargs):
            if "launch" in argv:
                self.assertIn("attach", calls)
                calls.append("spawn")
                if fail == "pid":
                    return {"exit_code": 0, "stdout": "no PID"}
                return {"exit_code": 0, "stdout": "dev.waterui.bench282: 92"}
            return {"exit_code": 0, "stdout": ""}
        def rss(*args):
            if fail == "rss":
                raise d.BenchError("rss failed")
            return [100] * 20
        with patch.object(d.subprocess, "Popen", side_effect=popen), \
                patch.object(d, "StructuredLogStream", return_value=parser), \
                patch.object(d, "run", side_effect=run), \
                patch.object(d, "stop_process", side_effect=stopped.append), \
                patch.object(d, "sim_cleanup") as cleanup, \
                patch.object(d, "toolchain_guard"), patch.object(d, "app_features", return_value={}), \
                patch.object(d, "rss_sample", side_effect=rss):
            if fail == "cleanup":
                def fail_termination(udid, bundle, operation):
                    if operation == "terminate":
                        raise d.BenchError("termination failed")
                cleanup.side_effect = fail_termination
            if fail:
                with self.assertRaises((d.BenchError, OSError)):
                    d.leg_launch(self.m, {}, "old", "fresh", platform, 1)
            else:
                rec = d.leg_launch(self.m, {}, "old", "fresh", platform, 1)
                self.assertEqual(rec["metrics"]["first_paint_ms"], 17)
            self.assertIn(stream, stopped)
            stream.stdout.close.assert_called_once()
            if platform == "macos" and "spawn" in calls and fail != "spawn":
                self.assertIn(process, stopped)
            if platform == "ios-simulator":
                operations = [call.args[2] for call in cleanup.call_args_list]
                self.assertIn("uninstall", operations)
                if "spawn" in calls:
                    self.assertIn("terminate", operations)

    def test_launch_order_both_platforms(self):
        for platform in ("macos", "ios-simulator"):
            with self.subTest(platform=platform):
                self.launch_fixture(platform)

    def test_launch_cleanup_all_failure_boundaries(self):
        for platform in ("macos", "ios-simulator"):
            for fail in ("attach", "marker", "rss", "spawn" if platform == "macos" else "pid"):
                with self.subTest(platform=platform, fail=fail):
                    self.launch_fixture(platform, fail)

    def test_cleanup_exception_still_closes_stream_and_uninstalls(self):
        self.launch_fixture("ios-simulator", "cleanup")

    def test_attach_timeout_fails_before_spawn(self):
        proc = MagicMock()
        with patch.object(d.select, "select", return_value=([], [], [])):
            with self.assertRaisesRegex(d.BenchError, "deadline"):
                d.StructuredLogStream(proc).await_attach(float("inf"))

    def test_stream_eof_fails(self):
        proc = MagicMock()
        with patch.object(d.select, "select", return_value=([proc.stdout], [], [])), \
                patch.object(d.os, "read", return_value=b""):
            with self.assertRaisesRegex(d.BenchError, "closed"):
                d.StructuredLogStream(proc).await_attach(float("inf"))

    def test_stop_process_kills_group_even_after_leader_exits(self):
        proc = MagicMock(pid=92)
        proc.poll.return_value = 0
        with patch.object(d.os, "killpg") as kill:
            d.stop_process(proc)
        kill.assert_called_once_with(92, d.signal.SIGKILL)
        proc.wait.assert_called_once()

    def test_ownership_rejects_symlink_outside_dedicated_home(self):
        home = self.root / "home"
        home.mkdir()
        (home / "cache").symlink_to(self.root / "foreign")
        with self.assertRaises(d.BenchError):
            d.assert_owned({"home": home, "uid": os.getuid()}, home / "cache")

    def test_cold_cleanup_preserves_toolchains_and_inputs(self):
        home = self.root
        project = home / "apps/old/form"
        for path in (home / ".water/build_cache/p/Test.app", home / "Library/Developer/Xcode/DerivedData",
                     project / "target", project / "apple", project / "src", home / "toolchains/old/bin"):
            path.mkdir(parents=True)
        (project / "src/lib.rs").write_text("source\n")
        binary = home / "toolchains/old/bin/water"
        binary.write_text("test fixture\n")
        with patch.object(d, "du_bytes", return_value=1), \
                patch.object(d, "run", return_value={"exit_code": 0, "timed_out": False, "wall_ms": 1}):
            d.cold_clean(self.m, {"home": home, "uid": os.getuid()}, "old", self.m["subjects"]["form"], project)
        self.assertTrue(binary.exists())
        self.assertTrue((project / "src/lib.rs").exists())
        self.assertFalse((home / ".water/build_cache").exists())
        self.assertFalse((project / "target").exists())

    def test_required_backend_rejects_missing_placeholder_and_old_override(self):
        with patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(d.BenchError, "no pinned SHA"):
                d.resolve_pin(self.m["sides"]["new"], "apple_backend", "new")
        with patch.dict(os.environ, {"BENCH282_NEW_APPLE_BACKEND_SHA": "7088fd966688eaa0ac0f5f567ca3e9e4247b9e76"}):
            with self.assertRaisesRegex(d.BenchError, "placeholder"):
                d.resolve_pin(self.m["sides"]["new"], "apple_backend", "new")
        with patch.dict(os.environ, {"BENCH282_OLD_CLI_SHA": "f" * 40}):
            with self.assertRaisesRegex(d.BenchError, "cannot be overridden"):
                d.resolve_pin(self.m["sides"]["old"], "cli", "old")

    def test_finalization_refuses_any_started_run_before_mutating(self):
        state = self.locked_state()
        requested = copy.deepcopy(state["resolved_pins"])
        requested["new"]["apple_backend"] = "f" * 40
        for evidence in ("records", "scaffold", "parity"):
            with self.subTest(evidence=evidence):
                saved = copy.deepcopy(state)
                saved["parity_passed"] = evidence == "parity"
                saved["run_started"] = evidence == "scaffold"
                record = d.RESULTS / "old__fresh.jsonl"
                if evidence == "records":
                    record.write_text("{}\n")
                d.save_state(saved)
                with patch.object(d, "dedicated_ctx", return_value={}), \
                        patch.object(d, "requested_pins", return_value=requested), \
                        patch.object(d, "prepare_checkouts") as prepare:
                    with self.assertRaisesRegex(d.BenchError, "immutable finalized"):
                        d.cmd_setup(self.m, finalize=True)
                    prepare.assert_not_called()
                record.unlink(missing_ok=True)

    def test_finalize_backend_only_preserves_tools_and_invalidates_unmeasured_state(self):
        requested = self.locked_state()["resolved_pins"]
        previous = copy.deepcopy(requested)
        previous["new"]["apple_backend"] = "7088fd966688eaa0ac0f5f567ca3e9e4247b9e76"
        tools = {"old": {"source_sha": previous["old"]["cli"]},
                 "new": {"source_sha": previous["new"]["cli"]}}
        d.save_state({"resolved_pins": previous, "tools": tools,
                      "source_sha256": {"new/form": "stale"}, "packaged": {"stale": {}}})
        with patch.object(d, "dedicated_ctx", return_value={}), \
                patch.object(d, "requested_pins", return_value=requested), \
                patch.object(d, "preflight_inputs", return_value=d.PreparedInputs((), ())), \
                patch.object(d, "prepare_checkouts") as checkouts, \
                patch.object(d, "prepare_cli", return_value="water"), \
                patch.object(d, "prepare_simulator"), \
                patch.object(d, "probe_toolchain", return_value={"rustc": "test"}), redirect_stdout(io.StringIO()):
            d.cmd_setup(self.m, finalize=True)
        actual = d.load_state()
        self.assertEqual(actual["resolved_pins"], requested)
        self.assertEqual(actual["tools"], tools)
        self.assertTrue(actual["inputs_finalized"])
        self.assertNotIn("source_sha256", actual)
        self.assertNotIn("packaged", actual)
        self.assertTrue(checkouts.call_args.args[-1])

    def test_existing_cli_receipt_reuses_binary_without_install(self):
        binary = d.water_bin("new")
        binary.parent.mkdir(parents=True)
        binary.write_text("test executable fixture\n")
        sha = "a" * 40
        receipt = {"source_sha": sha, "binary_sha256": d.file_sha256(binary)}
        ctx = {"home": self.root, "uid": os.getuid()}
        with patch.object(d, "checked_output", side_effect=[sha, ""]), \
                patch.object(d, "preparation_command", return_value={"stdout": "water\n"}) as prep:
            state = {}
            d.prepare_cli(self.m, ctx, state, "new", sha, {"new": receipt})
            self.assertEqual(state["tools"]["new"], receipt)
            self.assertEqual(prep.call_args.args[0], [str(binary), "--version"])
        with patch.object(d, "checked_output", side_effect=[sha, ""]), \
                patch.object(d, "preparation_command") as prep:
            with self.assertRaisesRegex(d.BenchError, "provenance"):
                d.prepare_cli(self.m, ctx, {}, "new", sha, {})
            prep.assert_not_called()

    def test_untracked_cli_source_refuses_build_and_installed_provenance(self):
        binary = d.water_bin("new")
        sha = "a" * 40
        ctx = {"home": self.root, "uid": os.getuid()}
        for installed in (False, True):
            with self.subTest(installed=installed):
                supplied = {}
                if installed:
                    binary.parent.mkdir(parents=True)
                    binary.write_text("test executable fixture\n")
                    supplied["new"] = {"source_sha": sha, "binary_sha256": d.file_sha256(binary)}
                with patch.object(d, "checked_output", side_effect=[sha, "?? src/injected.rs"]) as output, \
                        patch.object(d, "preparation_command") as prep:
                    with self.assertRaisesRegex(d.BenchError, "untracked inputs.*provenance"):
                        d.prepare_cli(self.m, ctx, {}, "new", sha, supplied)
                    self.assertEqual(output.call_args.args[0][-3:],
                                     ["status", "--porcelain", "--untracked-files=all"])
                    prep.assert_not_called()
                    self.assertFalse(d.STATE_PATH.exists())

    def test_measured_checkouts_reject_untracked_inputs_except_exact_owned_link(self):
        state = self.locked_state()
        framework = d.waterui_dir("new")
        link = framework / "backends/apple"
        link.parent.mkdir(parents=True)
        backend = d.backend_dir("new")
        backend.mkdir(parents=True)
        link.symlink_to(backend, target_is_directory=True)
        for dirty in (None, "apple_backend", "waterui", "cli"):
            def output(argv):
                name = {"apple-backend": "apple_backend", "waterui": "waterui", "cli": "cli"}[Path(argv[2]).name]
                if argv[3:] == ["rev-parse", "HEAD"]:
                    return state["resolved_pins"]["new"][name]
                self.assertEqual(argv[3:], ["status", "--porcelain", "--untracked-files=all"])
                status = "?? backends/apple\n" if name == "waterui" else ""
                return status + ("?? src/injected.rs\n" if name == dirty else "")
            with self.subTest(dirty=dirty), patch.object(d, "checked_output", side_effect=output):
                if dirty:
                    with self.assertRaisesRegex(d.BenchError, "dirty checkout"):
                        d.verify_checkouts(state, "new")
                else:
                    d.verify_checkouts(state, "new")
        link.unlink()
        link.symlink_to(self.root / "foreign")
        with self.assertRaisesRegex(d.BenchError, "harness-owned backend link"):
            d.require_clean_checkout(framework, backend)

    def runtimes(self):
        return [{"identifier": "com.apple.CoreSimulator.SimRuntime.iOS-27-0",
                 "version": "27.0", "isAvailable": True,
                 "supportedDeviceTypes": [{"identifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro",
                                           "productFamily": "iPhone"}]},
                {"identifier": "com.apple.CoreSimulator.SimRuntime.iOS-26-5",
                 "version": "26.5", "isAvailable": True,
                 "supportedDeviceTypes": [{"identifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-XS-Max",
                                           "productFamily": "iPhone"}]}]

    def test_simulator_uses_runtime_compatibility_and_bootstatus(self):
        calls = []
        def prepare(argv, *args, **kwargs):
            calls.append(argv)
            if "runtimes" in argv:
                return {"stdout": json.dumps({"runtimes": self.runtimes()})}
            if "create" in argv:
                self.assertEqual(argv[-2:], [self.runtimes()[0]["supportedDeviceTypes"][0]["identifier"], self.runtimes()[0]["identifier"]])
                return {"stdout": "owned-identifier\n"}
            return {"stdout": ""}
        with patch.object(d, "preparation_command", side_effect=prepare):
            state = {"simulator_udid": "precreated-workaround"}
            d.prepare_simulator(state)
        self.assertEqual(state["simulator_udid"], "owned-identifier")
        self.assertEqual(calls[-1], ["xcrun", "simctl", "bootstatus", "owned-identifier", "-b"])
        self.assertEqual(len([argv for argv in calls if "create" in argv]), 1)

    def test_owned_simulator_reuse_checks_exact_identifier_and_compatibility(self):
        runtime = self.runtimes()[0]
        dtype = runtime["supportedDeviceTypes"][0]["identifier"]
        owned = {"udid": "owned", "runtime": runtime["identifier"], "device_type": dtype}
        devices = {runtime["identifier"]: [{"udid": "owned", "isAvailable": True,
                                            "deviceTypeIdentifier": dtype}]}
        def prepare(argv, *args, **kwargs):
            self.assertNotIn("create", argv)
            if "runtimes" in argv:
                return {"stdout": json.dumps({"runtimes": self.runtimes()})}
            if "devices" in argv:
                return {"stdout": json.dumps({"devices": devices})}
            return {"stdout": ""}
        with patch.object(d, "preparation_command", side_effect=prepare):
            d.prepare_simulator({"owned_simulator": owned})
            devices[runtime["identifier"]][0]["udid"] = "same-name-foreign-device"
            with self.assertRaisesRegex(d.BenchError, "identity"):
                d.prepare_simulator({"owned_simulator": owned})
        runtime.pop("supportedDeviceTypes")
        with self.assertRaisesRegex(d.BenchError, "missing supportedDeviceTypes"):
            d.compatible_devices([runtime])

    def test_backend_replacement_refuses_dirty_or_linked_checkout(self):
        path = self.root / "checkouts/new/apple-backend"
        path.mkdir(parents=True)
        (path / ".git").write_text("gitdir: foreign-worktree\n")
        ctx = {"home": self.root, "uid": os.getuid()}
        with self.assertRaisesRegex(d.BenchError, "linked worktree"):
            d.owned_checkout(ctx, path, "test-origin")
        (path / ".git").unlink()
        (path / ".git").mkdir()
        with patch.object(d, "checked_output", side_effect=[str(path), str(path / ".git"),
                "test-origin", f"worktree {path}\n", " M Cargo.toml"]):
            with self.assertRaisesRegex(d.BenchError, "dirty checkout"):
                d.owned_checkout(ctx, path, "test-origin")

    def test_backend_replacement_fetches_exact_sha_without_reinstall_or_reset(self):
        requested = self.locked_state()["resolved_pins"]
        paths = {}
        for side in self.m["sides"]:
            for name, folder in (("apple_backend", "apple-backend"), ("waterui", "waterui"), ("cli", "cli")):
                path = self.root / "checkouts" / side / folder
                (path / ".git").mkdir(parents=True)
                paths[str(path)] = (side, name)
        replacement = str(d.backend_dir("new"))
        replaced = False
        def output(argv, *args, **kwargs):
            path = argv[2]
            side, name = paths[path]
            operation = argv[3:]
            if operation == ["rev-parse", "HEAD"]:
                return "f" * 40 if path == replacement and not replaced else requested[side][name]
            if operation == ["rev-parse", "--show-toplevel"]:
                return path
            if operation == ["rev-parse", "--absolute-git-dir"]:
                return str(Path(path) / ".git")
            if operation == ["remote", "get-url", "origin"]:
                return self.m["sides"][side][name]["repo"]
            if operation == ["worktree", "list", "--porcelain"]:
                return f"worktree {path}\n"
            if operation == ["status", "--porcelain", "--untracked-files=all"]:
                return ""
            self.fail(f"unexpected command: {argv}")
        commands = []
        def prepare(argv, *args, **kwargs):
            nonlocal replaced
            commands.append(argv)
            self.assertEqual(argv[2], replacement)
            self.assertNotIn("--force", argv)
            self.assertNotIn("reset", argv)
            if "checkout" in argv:
                replaced = True
        with patch.object(d, "checked_output", side_effect=output), \
                patch.object(d, "preparation_command", side_effect=prepare), \
                patch.object(d, "checkout_commit") as clone:
            state = {}
            ctx = {"home": self.root, "uid": os.getuid()}
            plan = d.preflight_inputs(self.m, ctx, state, requested, True, {})
            d.prepare_checkouts(self.m, ctx, state, requested, plan)
            clone.assert_not_called()
        self.assertEqual(len(commands), 2)
        self.assertEqual(commands[-1][-3:], ["checkout", "--detach", requested["new"]["apple_backend"]])
        self.assertEqual(state["prepared_pins"], requested)

    def test_preparation_ledger_records_source_and_duration(self):
        result = {"exit_code": 0, "timed_out": False, "wall_ms": 31,
                  "started_at": "2026-10-02T00:36:00-04:00", "stdout": "prepared"}
        with patch.object(d, "run", return_value=result):
            d.preparation_command(["git", "fetch"], 60, "a" * 40)
        ledger = json.loads((d.LOGS / "preparation.jsonl").read_text())
        self.assertEqual(ledger["source_sha"], "a" * 40)
        self.assertEqual(ledger["wall_ms"], 31)
        self.assertFalse(list(d.RESULTS.glob("*.jsonl")))

    def reconciliation_fixture(self):
        previous = self.locked_state()["resolved_pins"]
        requested = copy.deepcopy(previous)
        requested["new"] = {name: digit * 40 for name, digit in
                            (("apple_backend", "1"), ("waterui", "2"), ("cli", "3"))}
        state = {"prepared_pins": previous, "resolved_pins": copy.deepcopy(previous),
                 "inputs_finalized": True, "tools": {}}
        for side in self.m["sides"]:
            for folder in ("apple-backend", "waterui", "cli"):
                (self.root / "checkouts" / side / folder / ".git").mkdir(parents=True)
            link = d.waterui_dir(side) / "backends/apple"
            link.parent.mkdir(parents=True)
            link.symlink_to(d.backend_dir(side))
            binary = d.water_bin(side)
            binary.parent.mkdir(parents=True)
            binary.write_text(f"{side} coordinator-built output\n")
            state["tools"][side] = {"source_sha": previous[side]["cli"],
                                    "binary_sha256": d.file_sha256(binary)}
        state["tools"]["new"]["binary_sha256"] = "f" * 64
        supplied = {"new": {"source_sha": requested["new"]["cli"],
                            "binary_sha256": d.file_sha256(d.water_bin("new"))}}
        ctx = {"home": self.root, "uid": os.getuid()}
        return state, requested, supplied, ctx

    def test_all_new_inputs_reconcile_with_explicit_receipt_and_history(self):
        state, requested, supplied, ctx = self.reconciliation_fixture()
        original = copy.deepcopy(state)
        heads = copy.deepcopy(state["prepared_pins"])
        def owned(ctx, path, url, backend=None):
            name = {"apple-backend": "apple_backend", "waterui": "waterui", "cli": "cli"}[path.name]
            if name == "waterui":
                self.assertEqual(backend, d.backend_dir(path.parent.name))
            return heads[path.parent.name][name]
        commands = []
        def preparation(argv, *args, **kwargs):
            commands.append(argv)
            if "checkout" in argv:
                path = Path(argv[2])
                name = {"apple-backend": "apple_backend", "waterui": "waterui", "cli": "cli"}[path.name]
                heads[path.parent.name][name] = argv[-1]
            self.assertNotIn("cargo", argv)
            return {"stdout": "water\n"}
        receipt_path = self.root / "receipts.json"
        receipt_path.write_text(json.dumps(supplied))
        d.save_state(state)
        with patch.object(d, "dedicated_ctx", return_value=ctx), \
                patch.object(d, "requested_pins", return_value=requested), \
                patch.object(d, "owned_checkout", side_effect=owned), \
                patch.object(d, "checked_output", side_effect=lambda argv: heads[Path(argv[2]).parent.name]["cli"] if "HEAD" in argv else ""), \
                patch.object(d, "preparation_command", side_effect=preparation), \
                patch.object(d, "prepare_simulator"), \
                patch.object(d, "probe_toolchain", return_value={"rustc": "test"}), redirect_stdout(io.StringIO()):
            d.cmd_setup(self.m, finalize=True, provenance_path=receipt_path)
        actual = d.load_state()
        self.assertEqual(actual["resolved_pins"], requested)
        self.assertEqual(actual["tools"]["old"], original["tools"]["old"])
        self.assertEqual(actual["tools"]["new"], supplied["new"])
        self.assertEqual(actual["tool_history"]["new"], [original["tools"]["new"]])
        self.assertEqual(actual["input_history"][-1]["previous"]["prepared_pins"], original["prepared_pins"])
        self.assertEqual(actual["input_history"][-1]["status"], "finalized")
        self.assertEqual(len([argv for argv in commands if "checkout" in argv]), 3)

    def test_reconciliation_preflight_rejects_old_started_wrong_receipt_and_dirty(self):
        state, requested, supplied, ctx = self.reconciliation_fixture()
        def owned(ctx, path, url, backend=None):
            name = {"apple-backend": "apple_backend", "waterui": "waterui", "cli": "cli"}[path.name]
            return state["prepared_pins"][path.parent.name][name]
        for case in ("old", "started", "setup", "missing-receipt", "wrong-sha", "wrong-hash", "dirty"):
            with self.subTest(case=case):
                current, pins, receipts = copy.deepcopy(state), copy.deepcopy(requested), copy.deepcopy(supplied)
                if case == "old":
                    pins["old"]["cli"] = "9" * 40
                elif case == "started":
                    current["run_started"] = True
                elif case == "missing-receipt":
                    receipts = {}
                elif case == "wrong-sha":
                    receipts["new"]["source_sha"] = state["prepared_pins"]["new"]["cli"]
                elif case == "wrong-hash":
                    receipts["new"]["binary_sha256"] = "0" * 64
                error = d.BenchError("dirty checkout: untracked input") if case == "dirty" else owned
                with patch.object(d, "owned_checkout", side_effect=error), \
                        patch.object(d, "preparation_command") as mutate, \
                        patch.object(d, "save_state") as save:
                    with self.assertRaises(d.BenchError):
                        d.preflight_inputs(self.m, ctx, current, pins, case != "setup", receipts)
                    mutate.assert_not_called()
                    save.assert_not_called()

    def test_receipt_preflight_failure_preserves_entire_existing_state(self):
        state, requested, supplied, ctx = self.reconciliation_fixture()
        d.save_state(state)
        original = d.STATE_PATH.read_bytes()
        supplied["new"]["binary_sha256"] = "0" * 64
        path = self.root / "receipt.json"
        path.write_text(json.dumps(supplied))
        def owned(ctx, checkout, url, backend=None):
            name = {"apple-backend": "apple_backend", "waterui": "waterui", "cli": "cli"}[checkout.name]
            return state["prepared_pins"][checkout.parent.name][name]
        with patch.object(d, "dedicated_ctx", return_value=ctx), \
                patch.object(d, "requested_pins", return_value=requested), \
                patch.object(d, "owned_checkout", side_effect=owned), \
                patch.object(d, "prepare_checkouts") as mutate:
            with self.assertRaisesRegex(d.BenchError, "matching source-SHA/binary-SHA256 provenance"):
                d.cmd_setup(self.m, True, path)
            mutate.assert_not_called()
        self.assertEqual(d.STATE_PATH.read_bytes(), original)

    def test_old_receipt_cannot_be_replaced_even_by_matching_installed_hash(self):
        state, requested, supplied, ctx = self.reconciliation_fixture()
        binary = d.water_bin("old")
        binary.write_text("different old executable fixture\n")
        supplied["old"] = {"source_sha": requested["old"]["cli"],
                           "binary_sha256": d.file_sha256(binary)}
        original = copy.deepcopy(state)
        with self.assertRaisesRegex(d.BenchError, "receipt replacement requires unstarted new-side"):
            d.cli_receipt(ctx, state, "old", requested["old"]["cli"], supplied, True)
        self.assertEqual(state, original)


if __name__ == "__main__":
    unittest.main()
