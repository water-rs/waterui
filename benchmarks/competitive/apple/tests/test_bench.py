#!/usr/bin/env python3
"""Pure-fixture + real-control-flow checks for bench.py (stdlib only).

Covers: devicectl JSON parse (nominal/warm/unreadable/missing),
xcresult-error row marking, CLI hash verification, host fingerprint
gating, the device cleanup path with injected cleanup failures, xctrace
export parsing, the pty line reader and xctestrun injection. No device,
simulator, network, or build required — failures are injected through
real control flow, not mocks shaped like the implementation.

Run: uv run tests/test_bench.py
"""
import sys

if sys.version_info < (3, 10):
    raise SystemExit(
        "benchmarks/competitive requires Python >= 3.10 "
        f"(this interpreter is {sys.version.split()[0]}); every leg "
        "declares its version in pyproject.toml + .python-version and "
        "runs under the uv-managed interpreter (`uv run`)")

import argparse
import importlib.util
import json
import shutil
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("bench", ROOT / "bench.py")
bench = importlib.util.module_from_spec(spec)
sys.modules["bench"] = bench
spec.loader.exec_module(bench)


def devicectl_doc(**fields):
    """A devicectl --json-output shaped document with the fields nested
    the way the real tool nests them (hardwareProperties /
    deviceProperties / screenProperties / connectionProperties)."""
    hw, dev, scr, conn = {}, {}, {}, {}
    for k, v in fields.items():
        if k in ("marketingName", "productType", "deviceType"):
            hw[k] = v
        elif k in ("osVersionNumber", "deviceName"):
            dev[k] = v
        elif k == "maximumFramesPerSecond":
            scr[k] = v
        else:
            conn[k] = v
    return {"info": {"arguments": ["device", "info", "details"]},
            "result": {"devices": [{
                "identifier": "test-udid",
                "hardwareProperties": hw,
                "deviceProperties": dev,
                "screenProperties": scr,
                "connectionProperties": conn,
            }]}}


class TestDeviceInfoParse(unittest.TestCase):
    def test_nominal(self):
        st = bench.parse_device_info(devicectl_doc(
            thermalState="nominal", maximumFramesPerSecond=120,
            marketingName="iPad Pro 13-inch (M4)",
            productType="iPad16,6", osVersionNumber="26.5"))
        self.assertEqual(st["thermal"], "nominal")
        self.assertEqual(st["refresh_hz"], 120)
        self.assertEqual(st["model"], "iPad Pro 13-inch (M4)")
        self.assertEqual(st["product_type"], "iPad16,6")
        self.assertEqual(st["os_version"], "26.5")

    def test_thermal_states_parse(self):
        # thermal is parsed when devicectl reports it (an absent or
        # unreadable value reads 'unknown' — the gate itself lives in
        # the on-device runner now)
        for st_name in ("fair", "serious", "critical"):
            st = bench.parse_device_info(
                devicectl_doc(thermalState=st_name))
            self.assertEqual(st["thermal"], st_name)

    def test_missing_thermal_is_unknown(self):
        st = bench.parse_device_info(devicectl_doc(
            marketingName="iPhone 17"))
        self.assertEqual(st["thermal"], "unknown")

    def test_missing_refresh_stays_null_no_spec_fallback(self):
        st = bench.parse_device_info(devicectl_doc(
            thermalState="nominal"))
        self.assertIsNone(st["refresh_hz"])
        self.assertNotIn("refresh_hz_source", st)

    def test_malformed_and_flat_field_placement(self):
        # fields nested differently than expected still parse via the
        # recursive key walk; a malformed document records read_error
        st = bench.parse_device_info({"result": {"devices": [{
            "connectionProperties": {"thermalState": "nominal",
                                     "maximumFramesPerSecond": 59.94}}]}})
        self.assertEqual(st["thermal"], "nominal")
        self.assertEqual(st["refresh_hz"], 59)
        # a non-dict document degrades to unknown-thermal, not a crash;
        # the read_error marker is recorded by device_state's JSON load
        bad = bench.parse_device_info(None)
        self.assertEqual(bad["thermal"], "unknown")
        self.assertIsNone(bad["refresh_hz"])


class TestXcresultError(unittest.TestCase):
    def test_metrics_error_marks_run_and_keeps_diagnostics(self):
        orig = bench.parse_xcresult
        try:
            bench.parse_xcresult = lambda path: {
                "_error": "xcresulttool: invalid plist at byte 9"}
            rec = bench._metrics_record(Path("/nonexistent"))
            self.assertIn("error", rec)
            self.assertEqual(rec["xcresult_error"],
                             "xcresulttool: invalid plist at byte 9")
        finally:
            bench.parse_xcresult = orig

    def test_flatten_skips_error_rows_and_verbatim_strings(self):
        results = {"runs": [
            {"platform": "macos", "contestant": "a", "workload": "W1",
             "metrics": {"_error": "raw tail kept"}},
            {"platform": "macos", "contestant": "a", "workload": "W1",
             "metrics": {"t": {"m": {"unit": "s",
                                     "measurements": [1.0, 2.0]}}}},
        ]}
        table = bench.flatten(results, "macos")
        self.assertEqual(table[("a", "W1", "swipe")]["t:m"]["median"],
                         1.5)


class TestCliVerify(unittest.TestCase):
    def test_hash_match_and_mismatch(self):
        ev = {"binary": "/x/water", "sha256": "abc"}
        self.assertIsNone(bench.cli_check(ev, {"sha256": "abc"}))
        err = bench.cli_check(ev, {"sha256": "def"})
        self.assertIn("mismatch", err)
        self.assertIn("cannot hash",
                      bench.cli_check(
                          {"binary": "/x/water", "error": "binary not found"},
                          {"sha256": "abc"}))


FP = {"hw_model": "Mac15,6", "cpu_brand": "Apple M4",
      "hv_vmm_present": False, "gpus": ["Apple M4"]}
CLI_SHA = "b01f11d2" * 8
HEAD = "a1b2c3d4" * 5


def synth_results(plat="macos", reps=5, drop_cell=None, fail_reps=()):
    """A results document covering every required cell on `plat`, with
    `fail_reps` rep indices recorded as error rows (never counted) and
    `drop_cell`=(cid,w) left unrecorded."""
    runs = []
    for cid, w, dr in sorted(bench.required_cells(plat)):
        if (cid, w) == drop_cell:
            continue
        for rep in range(reps):
            if rep in fail_reps:
                runs.append({"platform": plat, "contestant": cid,
                             "workload": w, "drive": dr, "repeat": rep,
                             "error": "injected launch failure"})
            else:
                # the producer's real shape: one dict per test
                # identifier, metric tails with unit + measurements
                runs.append({"platform": plat, "contestant": cid,
                             "workload": w, "drive": dr, "repeat": rep,
                             "metrics": {
                                 "testLaunch": {
                                     "duration": {
                                         "unit": "s",
                                         "measurements":
                                             [1.0 + rep * 0.01]}},
                                 "testWorkload": {
                                     "physical_peak": {
                                         "unit": "kB",
                                         "measurements": [64000]},
                                     "time": {
                                         "unit": "s",
                                         "measurements": [0.4]},
                                     "hitch_time_ratio": {
                                         "unit": "ms/s",
                                         "measurements": [0.0]}}}})
    return {"machine": {"model": "Mac", "fingerprint": FP},
            "platform": plat,
            "pins": {"water_cli": CLI_SHA, "waterui_head": HEAD},
            "runs": runs, "sizes": {}}


def run_report(docs):
    """Run the real report command on temp results files; returns the
    report text (SystemExit propagates)."""
    import argparse
    tmp = Path(tempfile.mkdtemp(prefix="bench-report-"))
    paths = []
    for i, d in enumerate(docs):
        p = tmp / f"r{i}.json"
        p.write_text(json.dumps(d))
        paths.append(str(p))
    out = tmp / "report.md"
    # the synthetic docs only cover the platform(s) they synthesize — the
    # manifest's required_platforms would flag the others; scope the
    # requirement to the docs' platforms
    plats = sorted({d.get("platform") for d in docs if d.get("platform")})
    meas = bench.MANIFEST.setdefault("measurement", {})
    saved = meas.get("required_platforms")
    meas["required_platforms"] = plats
    try:
        bench.cmd_report(argparse.Namespace(
            input=",".join(paths), out=str(out)))
    finally:
        if saved is None:
            meas.pop("required_platforms", None)
        else:
            meas["required_platforms"] = saved
        text = out.read_text() if out.exists() else ""
        shutil.rmtree(tmp)
    return text


class TestReporting(unittest.TestCase):
    def test_flatten_reports_n_min_max_spread(self):
        results = synth_results()
        table = bench.flatten(results, "macos")
        cell = next(iter(table.values()))["testLaunch:duration"]
        self.assertEqual(cell["n"], 5)
        self.assertAlmostEqual(cell["median"], 1.02)
        self.assertAlmostEqual(cell["min"], 1.0)
        self.assertAlmostEqual(cell["max"], 1.04)
        self.assertAlmostEqual(cell["spread"], 0.04, places=6)

    def test_complete_dataset_passes_and_shows_n(self):
        text = run_report([synth_results()])
        self.assertIn("n=5", text)
        self.assertNotIn("DATASET INCOMPLETE", text)

    def test_missing_cell_fails_report(self):
        cid, w, _ = next(iter(sorted(bench.required_cells("macos"))))
        with self.assertRaises(SystemExit):
            run_report([synth_results(drop_cell=(cid, w))])

    def test_all_failed_and_mixed_cells_fail(self):
        # 2 of 5 reps are error rows -> 3 successful < floor
        with self.assertRaises(SystemExit):
            run_report([synth_results(fail_reps=(3, 4))])
        # all-failed cell: every rep an error
        doc = synth_results(reps=5)
        target = next(iter(sorted(bench.required_cells("macos"))))
        for r in doc["runs"]:
            if (r.get("contestant"), r.get("workload")) == target[:2]:
                r.pop("metrics", None)
                r["error"] = "injected xcresult failure"
        with self.assertRaises(SystemExit) as ctx:
            run_report([doc])
        self.assertIn("incomplete", str(ctx.exception))
        # the failed attempts are retained verbatim in the report
        # (report file was written before the SystemExit)

    def test_merge_requires_identical_fingerprints(self):
        a = synth_results()
        b = synth_results()
        run_report([a, b])  # same fp + cli sha: merges
        bad_host = synth_results()
        bad_host["machine"]["fingerprint"] = dict(FP, hw_model="Macmini9,1")
        with self.assertRaises(SystemExit):
            run_report([a, bad_host])
        bad_cli = synth_results()
        bad_cli["pins"]["water_cli"] = "deadbeef"
        with self.assertRaises(SystemExit):
            run_report([a, bad_cli])
        stale = synth_results()
        stale["machine"].pop("fingerprint")
        with self.assertRaises(SystemExit):
            run_report([a, stale])


class TestHostGate(unittest.TestCase):
    REAL = {"hw_model": "Mac15,6", "cpu_brand": "Apple M4",
            "hv_vmm_present": False, "gpus": ["Apple M4"]}
    VM = {"hw_model": "VirtualMac2,1", "cpu_brand": "Apple M4 (Virtual)",
          "hv_vmm_present": True, "gpus": ["Apple Paravirtual device"]}

    def test_virtual_detection(self):
        self.assertIsNone(bench.host_is_virtualized(self.REAL))
        self.assertIsNotNone(bench.host_is_virtualized(self.VM))
        para = dict(self.REAL, gpus=["Apple Paravirtual device"])
        self.assertIsNotNone(bench.host_is_virtualized(para))

    def test_vm_refused_with_no_escape(self):
        state = {"machine": {}}
        with self.assertRaises(SystemExit) as ctx:
            bench.check_host_fingerprint(state, self.VM)
        self.assertIn("virtualized", str(ctx.exception))

    def test_cli_fingerprint_mismatch_refused(self):
        state = {"machine": {"fingerprint": self.REAL},
                 "pins": {"waterui_head": HEAD,
                          "water_cli": "otherbinarysha"},
                 "runs": [{"platform": "macos"}]}
        orig_head, orig_ev = (bench.toolchain.checkout_head,
                              bench.cli_evidence)
        try:
            bench.toolchain.checkout_head = lambda: HEAD
            bench.cli_evidence = lambda: {
                "binary": "/x/water", "sha256": CLI_SHA}
            with self.assertRaises(SystemExit) as ctx:
                bench.check_source_fingerprint(state)
            self.assertIn("different water CLI", str(ctx.exception))
            # a matching recorded sha passes the gate
            state["pins"]["water_cli"] = CLI_SHA
            bench.check_source_fingerprint(state)
            # head mismatch still refuses
            state["pins"]["waterui_head"] = "f" * 40
            with self.assertRaises(SystemExit):
                bench.check_source_fingerprint(state)
            # predates provenance with runs present -> stale unknown
            with self.assertRaises(SystemExit):
                bench.check_source_fingerprint(
                    {"pins": {}, "runs": [{"platform": "macos"}]})
            # empty results pass both gates
            bench.check_source_fingerprint({"pins": {}, "runs": []})
        finally:
            bench.toolchain.checkout_head = orig_head
            bench.cli_evidence = orig_ev

    def test_mixed_host_refused(self):
        state = {"machine": {"fingerprint": dict(self.REAL,
                                               hw_model="Macmini9,1")}}
        with self.assertRaises(SystemExit) as ctx:
            bench.check_host_fingerprint(state, self.REAL)
        self.assertIn("different host", str(ctx.exception))

    def test_same_host_passes(self):
        state = {"machine": {"fingerprint": self.REAL}}
        bench.check_host_fingerprint(state, self.REAL)
        self.assertEqual(state["machine"]["fingerprint"], self.REAL)


class TestDeviceCleanup(unittest.TestCase):
    """Real control flow: injected cleanup failures must not suppress
    other cleanup actions or the original failure."""

    def test_cleanup_step_isolates_failures(self):
        tmp = Path(tempfile.mkdtemp(prefix="bench-clean-"))
        (tmp / "a").mkdir()
        (tmp / "b").mkdir()
        diags = []
        calls = []
        def rm(p):
            shutil.rmtree(p)
            calls.append(p)
        for p in (tmp / "a", tmp / "b"):
            d = bench._cleanup_step(f"rm {p}", lambda q=p: rm(q))
            if d:
                diags.append(d)
        d = bench._cleanup_step("fail", lambda: 1 / 0)
        if d:
            diags.append(d)
        self.assertFalse((tmp / "a").exists())
        self.assertFalse((tmp / "b").exists())
        self.assertEqual(len(calls), 2)
        self.assertEqual(len(diags), 1)
        self.assertIn("fail", diags[0])
        shutil.rmtree(tmp)

    def test_sh_real_failure_and_timeout(self):
        with self.assertRaises(RuntimeError):
            bench.sh("exit 7")
        self.assertIsNone(bench.sh("sleep 5", timeout=1))


class TestSanitizeKeepsErrors(unittest.TestCase):
    """N4: a failed attempt is evidence — sanitize drops superseded
    harness rows (missing/changed drive), never error records."""

    def test_error_rows_survive_sanitize(self):
        state = {"runs": [
            {"platform": "macos", "contestant": "waterui",
             "workload": None, "repeat": 0,
             "error": "missing artifact /x/Bench.app"},
            {"platform": "macos", "contestant": "waterui",
             "workload": "W1", "drive": "swipe", "repeat": 0,
             "error": "thermal gate closed"},
            {"platform": "macos", "contestant": "waterui",
             "workload": "W1", "drive": "stale-drive", "repeat": 0,
             "metrics": {"t": {"m": {"unit": "s", "measurements": [1]}}}},
        ]}
        bench.sanitize_runs(state)
        self.assertEqual(len(state["runs"]), 2)
        self.assertTrue(all("error" in r for r in state["runs"]))


class TestRunFloors(unittest.TestCase):
    """N9: a cell needs the required metric tails + frame evidence, not
    any metrics dict."""

    def _run(self, metrics):
        return {"platform": "macos", "contestant": "a",
                "workload": "W1", "repeat": 0, "metrics": metrics}

    def test_full_shape_passes(self):
        r = self._run({"testLaunch": {"duration": {
            "unit": "s", "measurements": [1.0]}},
            "testWorkload": {
                "physical_peak": {"unit": "kB", "measurements": [1]},
                "time": {"unit": "s", "measurements": [1]},
                "hitch_time_ratio": {"unit": "ms/s",
                                     "measurements": [0]}}})
        self.assertTrue(bench._run_succeeded(r))

    def test_missing_memory_fails(self):
        r = self._run({"testLaunch": {"duration": {
            "unit": "s", "measurements": [1.0]}},
            "testWorkload": {"time": {"unit": "s",
                                      "measurements": [1]}}})
        self.assertFalse(bench._run_succeeded(r))

    def test_no_frame_evidence_fails(self):
        r = self._run({"testLaunch": {"duration": {
            "unit": "s", "measurements": [1.0]}},
            "testWorkload": {
                "physical_peak": {"unit": "kB", "measurements": [1]},
                "time": {"unit": "s", "measurements": [1]}}})
        self.assertFalse(bench._run_succeeded(r))
        # xctrace presented frames on device satisfy the floor too
        r["frames"] = 42
        self.assertTrue(bench._run_succeeded(r))


class TestRequiredPlatforms(unittest.TestCase):
    """N2: a required platform with zero rows fails the report."""

    def test_missing_required_platform_fails(self):
        # a macos-only dataset under the real manifest (which declares
        # ios-sim, macos and ios-device required) must fail
        doc = synth_results("macos")
        tmp = Path(tempfile.mkdtemp(prefix="bench-req-"))
        try:
            p = tmp / "r.json"
            p.write_text(json.dumps(doc))
            out = tmp / "report.md"
            with self.assertRaises(SystemExit) as ctx:
                bench.cmd_report(argparse.Namespace(
                    input=str(p), out=str(out)))
            self.assertIn("incomplete", str(ctx.exception))
            self.assertTrue(out.exists())
        finally:
            shutil.rmtree(tmp)


class TestStagingManifest(unittest.TestCase):
    """N5: the run must measure exactly what the build staged."""

    def test_dir_sha256_detects_tamper(self):
        tmp = Path(tempfile.mkdtemp(prefix="bench-stage-"))
        try:
            app = tmp / "A.app"
            (app / "Contents").mkdir(parents=True)
            (app / "Contents" / "bin").write_bytes(b"\x01" * 100)
            h1 = bench.dir_sha256(app)
            h2 = bench.dir_sha256(app)
            self.assertEqual(h1, h2)  # stable
            (app / "Contents" / "bin").write_bytes(b"\x02" * 100)
            self.assertNotEqual(h1, bench.dir_sha256(app))
            # a new file is a different artifact too
            (app / "extra").write_text("x")
            self.assertNotEqual(h2, bench.dir_sha256(app))
        finally:
            shutil.rmtree(tmp)


class TestRunnerLog(unittest.TestCase):
    """N1/N8: the runner's marks + on-device record are read from the
    runner log; stale rows from a previous rep can't alias in."""

    def test_marks_and_device_record(self):
        tmp = Path(tempfile.mkdtemp(prefix="bench-rlog-"))
        try:
            log = tmp / "bench-runner.log"
            log.write_text(
                "100.0 drive-begin\n"
                "105.0 measure-end\n"
                "199.9 device-record thermal=0 maxFps=120\n"
                "200.0 drive-begin\n"
                "207.0 measure-end\n")
            marks, dev = bench.read_runner_log(log, since=150.0)
            self.assertEqual(marks, {"drive-begin": 200.0,
                                     "measure-end": 207.0})
            self.assertEqual(dev, {"thermal": "0", "maxFps": "120"})
            # marks from before `since` are invisible
            marks, dev = bench.read_runner_log(log, since=300.0)
            self.assertEqual(marks, {})
            self.assertEqual(dev, {})
        finally:
            shutil.rmtree(tmp)

    def test_unpaired_mark_is_no_window(self):
        tmp = Path(tempfile.mkdtemp(prefix="bench-rlog-"))
        try:
            log = tmp / "bench-runner.log"
            log.write_text("200.0 drive-begin\n")
            self.assertEqual(bench.read_runner_log(log, since=0.0)[0], {})
        finally:
            shutil.rmtree(tmp)


class TestSamplerPidRebind(unittest.TestCase):
    """N8: a delta across a pid-set change is None, never the difference
    of two different processes' cumulative times."""

    def _sampler(self):
        return bench.CpuSampler(lambda: {}, interval=0.5)

    def test_pid_change_returns_none(self):
        s = self._sampler()
        s.series = [
            (100.0, {"app": {"pids": [10], "cpu_s": 1.0, "rss_kb": 1}}),
            (105.0, {"app": {"pids": [20], "cpu_s": 0.2, "rss_kb": 1}}),
            (110.0, {"app": {"pids": [20], "cpu_s": 0.9, "rss_kb": 1}}),
        ]
        self.assertIsNone(s.delta("app", 100.0, 110.0))
        self.assertAlmostEqual(s.delta("app", 105.0, 110.0), 0.7)

    def test_whole_run_requires_same_set(self):
        s = self._sampler()
        s.series = [
            (100.0, {"app": {"pids": [10], "cpu_s": 1.0, "rss_kb": 1}}),
            (110.0, {"app": {"pids": [20], "cpu_s": 0.9, "rss_kb": 1}}),
        ]
        self.assertIsNone(s.delta("app"))
        self.assertEqual(s.peak_rss_mb("app", 90.0, 120.0), 0.0)


class TestXctraceExport(unittest.TestCase):
    """xctrace export rows are keyed by the schema's column mnemonics:
    row children carry engineering-type tags in column order, refs
    resolve to the earlier definition, and an empty cell is a
    sentinel."""

    DOC = """<?xml version="1.0"?>
<trace-query-result><node xpath="x">
<schema name="time-sample">
<col><mnemonic>time</mnemonic><engineering-type>sample-time</engineering-type></col>
<col><mnemonic>thread</mnemonic><engineering-type>thread</engineering-type></col>
<col><mnemonic>thread-state</mnemonic><engineering-type>thread-state</engineering-type></col>
</schema>
<row><sample-time id="1" fmt="00:00.001">1000000</sample-time><thread id="2" fmt="Main Thread 0x1 (BenchUIKit, pid: 42)"/><thread-state id="3" fmt="Running">Running</thread-state></row>
<row><sample-time id="4" fmt="00:00.002">2000000</sample-time><thread ref="2"/><sentinel/></row>
</node></trace-query-result>"""

    def test_rows_keyed_by_mnemonic(self):
        rows, err = bench.parse_export_rows(self.DOC)
        self.assertIsNone(err)
        self.assertEqual(rows[0]["time"], "1000000")
        self.assertEqual(rows[0]["thread-state"], "Running")
        # the ref repeats the defining element's value
        self.assertEqual(bench._pid_of_thread(rows[1]["thread"]), 42)
        self.assertIsNone(rows[1]["thread-state"])

    def test_cell_count_mismatch_is_an_error(self):
        doc = self.DOC.replace("<sentinel/>", "")
        rows, err = bench.parse_export_rows(doc)
        self.assertIsNone(rows)
        self.assertIn("schema columns", err)

    def test_process_cell_name(self):
        self.assertEqual(bench._proc_name("WaterUI Bench (311)"),
                         "WaterUI Bench")
        self.assertIsNone(bench._proc_name(None))
        self.assertEqual(bench._pid_of_process("WaterUI Bench (311)"), 311)

    def test_one_document_many_tables(self):
        """Every table of a schema comes back, each zipped against its
        own node's schema; refs resolve across nodes."""
        doc = """<?xml version="1.0"?>
<trace-query-result>
<node xpath="a"><schema name="os-signpost">
<col><mnemonic>time</mnemonic></col><col><mnemonic>name</mnemonic></col>
</schema>
<row><event-time id="1">10</event-time><string id="2">drive-begin</string></row>
</node>
<node xpath="b"><schema name="os-signpost">
<col><mnemonic>name</mnemonic></col><col><mnemonic>time</mnemonic></col>
</schema>
<row><string ref="2"/><event-time id="3">20</event-time></row>
</node></trace-query-result>"""
        rows, err = bench.parse_export_rows(doc)
        self.assertIsNone(err)
        self.assertEqual(rows, [{"time": "10", "name": "drive-begin"},
                                {"name": "drive-begin", "time": "20"}])


class TestFrameAttribution(unittest.TestCase):
    """H2/M2: frames are the contestant's own by the swap join, and the
    window is [first owned present + warmup, + capture] on the trace
    clock, gated by the runner's drive-begin / measure-end marks."""

    BUNDLE = "WaterUI Bench.app"
    MAIN = "Contents/MacOS/WaterUI Bench"
    PROCS = [
        {"name": "WindowServer", "pid": 150,
         "path": "/System/Library/PrivateFrameworks/SkyLight.framework/"
                 "Resources/WindowServer"},
        {"name": "WaterUI Bench", "pid": 700,
         "path": "/b/Release/WaterUI Bench.app/Contents/MacOS/WaterUI Bench"},
        {"name": "helper", "pid": 701,
         "path": "/b/Release/WaterUI Bench.app/Contents/Frameworks/"
                 "H.app/Contents/MacOS/helper"},
        {"name": "SystemUIServer", "pid": 300,
         "path": "/System/Library/CoreServices/SystemUIServer.app/"
                 "Contents/MacOS/SystemUIServer"},
    ]

    def test_toc_processes(self):
        toc = """<?xml version="1.0"?>
<trace-toc><run number="1"><processes>
<process name="kernel" pid="0"/>
<process name="WaterUI Bench" pid="700" path="/x/WaterUI Bench.app/Contents/MacOS/WaterUI Bench"/>
</processes></run></trace-toc>"""
        procs = bench.parse_toc_processes(toc)
        self.assertEqual(procs[1], {"name": "WaterUI Bench", "pid": 700,
                                    "path": "/x/WaterUI Bench.app/Contents/"
                                            "MacOS/WaterUI Bench"})
        main, owned = bench.owned_processes(procs, self.BUNDLE, self.MAIN)
        self.assertEqual((main, owned), (700, {700}))

    def test_owned_processes_bundle_tree(self):
        main, owned = bench.owned_processes(self.PROCS, self.BUNDLE,
                                            self.MAIN)
        self.assertEqual(main, 700)
        self.assertEqual(owned, {700, 701})
        # a second instance of the main executable is not one launch
        two = self.PROCS + [dict(self.PROCS[1], pid=702)]
        with self.assertRaises(bench.TraceAttributionError):
            bench.owned_processes(two, self.BUNDLE, self.MAIN)
        with self.assertRaises(bench.TraceAttributionError):
            bench.owned_processes(self.PROCS[:1], self.BUNDLE, self.MAIN)

    @staticmethod
    def frame(start, dur, swap, display="1"):
        return {"start": str(start), "duration":
                None if dur is None else str(dur),
                "swap-id": str(swap), "display": display}

    @staticmethod
    def update(pid, swap):
        return {"process": f"P ({pid})", "swap-id": str(swap)}

    def test_swap_join_keeps_only_owned_frames(self):
        frames = [self.frame(1_000, 500, 1),      # menu bar only
                  self.frame(2_000, 500, 2),      # contestant
                  self.frame(3_000, 500, 3),      # helper (owned)
                  self.frame(4_000, None, 4)]     # never presented
        updates = [self.update(300, 1), self.update(700, 2),
                   self.update(701, 3), self.update(700, 4)]
        j = bench.owned_presents(frames, updates, {700, 701})
        self.assertEqual(j["presents_ns"], [2_500, 3_500])
        self.assertEqual(j["display"], "1")
        self.assertEqual(j["updates_owned"], 3)

    def test_swap_join_refuses_unattributable(self):
        with self.assertRaises(bench.TraceAttributionError):
            bench.owned_presents([self.frame(0, 1, 1)],
                                 [self.update(300, 1)], {700})
        with self.assertRaises(bench.TraceAttributionError):
            bench.owned_presents([self.frame(0, 1, 9)],
                                 [self.update(700, 1)], {700})
        two_displays = [self.frame(0, 1, 1, "1"), self.frame(5, 1, 1, "2")]
        with self.assertRaises(bench.TraceAttributionError):
            bench.owned_presents(two_displays, [self.update(700, 1)], {700})
        # a schema without the join columns fails, never joins on nothing
        with self.assertRaises(bench.TraceAttributionError):
            bench.owned_presents([{"start": "0", "duration": "1"}],
                                 [self.update(700, 1)], {700})

    def test_runner_marks(self):
        rows = [{"time": "100", "name": "drive-begin",
                 "subsystem": "dev.bench"},
                {"time": "900", "name": "measure-end",
                 "subsystem": "dev.bench"},
                {"time": "50", "name": "drive-begin",
                 "subsystem": "com.other"}]
        self.assertEqual(bench.runner_marks(rows),
                         {"drive-begin": 100, "measure-end": 900})
        with self.assertRaises(bench.TraceAttributionError):
            bench.runner_marks(rows + [rows[0]])
        with self.assertRaises(bench.TraceAttributionError):
            bench.runner_marks(rows[1:])

    def test_window_from_first_owned_present(self):
        ms = 1_000_000
        presents = [500 * ms, 516 * ms, 532 * ms]
        marks = {"drive-begin": 3_480 * ms, "measure-end": 15_600 * ms}
        w = bench.measurement_window(presents, marks, warmup_ms=3000,
                                     capture_ms=12000, tolerance_ms=100)
        self.assertEqual(w["window_start_ms"], 3500.0)
        self.assertEqual(w["window_end_ms"], 15500.0)
        self.assertEqual(w["drive_offset_ms"], -20.0)
        # a drive outside tolerance is not "at window start"
        with self.assertRaises(bench.TraceAttributionError):
            bench.measurement_window(
                presents, dict(marks, **{"drive-begin": 3_700 * ms}),
                warmup_ms=3000, capture_ms=12000, tolerance_ms=100)
        # released before the window end: the tail is not the workload
        with self.assertRaises(bench.TraceAttributionError):
            bench.measurement_window(
                presents, dict(marks, **{"measure-end": 15_400 * ms}),
                warmup_ms=3000, capture_ms=12000, tolerance_ms=100)

    def test_stats_clip_to_trace_window(self):
        """Frame statistics cover exactly the trace window, however long
        the runner held past it (e.g. a late dev.bench.end)."""
        ms = 1_000_000
        presents = [0] + [(3_000 + 10 * i) * ms for i in range(0, 200)]
        presents[0] = 0
        w = bench.measurement_window(
            presents, {"drive-begin": 3_000 * ms,
                       "measure-end": 6_000 * ms},
            warmup_ms=3000, capture_ms=1000, tolerance_ms=100)
        st = bench.lib_frames.frame_statistics(
            [t / 1e6 for t in presents], w["window_start_ms"],
            1000.0, 1000.0 / 120)
        self.assertEqual(st["presents"], 101)  # 3000..4000 ms inclusive


class TestPtyLines(unittest.TestCase):
    """The arm/begin readers block in select(2) on the descriptor and
    honour the caller's deadline — no polling."""

    def test_lines_then_eof(self):
        import os
        r, w = os.pipe()
        os.write(w, b"dev.bench.begin 0\r\ndev.bench.begin\npartial")
        os.close(w)
        lines = bench.PtyLines(r)
        self.assertEqual(lines.readline(None), "dev.bench.begin 0")
        self.assertEqual(lines.readline(None), "dev.bench.begin")
        self.assertEqual(lines.readline(None), "partial")
        self.assertIsNone(lines.readline(None))
        lines.close()

    def test_deadline(self):
        import os
        r, w = os.pipe()
        lines = bench.PtyLines(r)
        with self.assertRaises(TimeoutError):
            lines.readline(bench.time.monotonic() + 0.05)
        lines.close()
        os.close(w)


class TestXctestrunInjection(unittest.TestCase):
    """The run nonce and the ladder value reach the runner's
    environment; the notify channel exists only where the host can
    reach the namespace."""

    def test_nonce_and_step(self):
        import plistlib
        tmp = Path(tempfile.mkdtemp(prefix="bench-xr-"))
        try:
            tmpl = tmp / "t.xctestrun"
            tmpl.write_bytes(plistlib.dumps({"BenchRunner": {
                "DependentProductPaths": []}}))
            out = tmp / "o.xctestrun"
            bench.write_xctestrun(tmpl, out, "BenchRunner", "Release",
                                  "X.app", "dev.bench.x", "w5", "none", 12,
                                  runner_app="", nonce=12345, step=800)
            env = plistlib.loads(out.read_bytes())["BenchRunner"][
                "EnvironmentVariables"]
            self.assertEqual(env["BENCH_RUN_NONCE"], "12345")
            self.assertEqual(env["BENCH_STEP"], "800")
            self.assertEqual(env["BENCH_DURATION"], "12")
        finally:
            shutil.rmtree(tmp)

    def test_notify_namespaces(self):
        self.assertEqual(bench._notify_cmd("macos", None, "-p", "a"),
                         ["notifyutil", "-p", "a"])
        self.assertEqual(
            bench._notify_cmd("ios-sim", "U", "-p", "a"),
            ["xcrun", "simctl", "spawn", "U", "notifyutil", "-p", "a"])
        with self.assertRaises(ValueError):
            bench._notify_cmd("ios-device", "U", "-p", "a")


class TestGitFixture(unittest.TestCase):
    """Decision 9: source-identity checks run against a real temporary
    git repo built inside the test — nothing here assumes the export
    the tests were copied into is itself a checkout."""

    def test_checkout_head_and_clean_gate(self):
        import subprocess
        tmp = Path(tempfile.mkdtemp(prefix="bench-git-"))
        try:
            def git(*a):
                subprocess.run(["git", "-C", str(tmp), *a],
                               capture_output=True, text=True, check=True)
            git("init", "-q", "-b", "main")
            git("config", "user.email", "bench@test")
            git("config", "user.name", "bench")
            (tmp / "f").write_text("x")
            git("add", "f")
            git("commit", "-qm", "init")
            head = bench.toolchain.checkout_head(root=tmp)
            self.assertRegex(head, r"^[0-9a-f]{40}$")
            bench.toolchain.require_clean_checkout(root=tmp)
            # a dirty tracked file makes the gate refuse
            (tmp / "f").write_text("y")
            with self.assertRaises(RuntimeError):
                bench.toolchain.require_clean_checkout(root=tmp)
        finally:
            shutil.rmtree(tmp)


if __name__ == "__main__":
    unittest.main(verbosity=2)
