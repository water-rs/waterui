#!/usr/bin/env python3
"""Pure-fixture + real-control-flow checks for bench.py (stdlib only).

Covers: devicectl JSON parse, xcresult-error row marking, the report's
completeness gates, the resume gate, profile selection, capacity
summaries, the shared-bundle-id install cycle (uninstall, on-device
verification, install, installed-bundle check and their evidence) on an
in-memory device that answers through the real devicectl listing parser,
the staged set's contestant entries,
xctrace export parsing and frame attribution (with the render-server
frame gate), the pty line reader, the Instruments scratch sweep and its
SIGTERM path (real processes: copies of /bin/sleep), the device lock,
the staged-tarball check and xctestrun injection. No device, network,
or build required — failures are injected through real control flow,
not mocks shaped like the implementation.

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
import datetime
import importlib.util
import json
import plistlib
import shutil
import tempfile
import unittest
import urllib.parse
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("bench", ROOT / "bench.py")
bench = importlib.util.module_from_spec(spec)
sys.modules["bench"] = bench
spec.loader.exec_module(bench)


def devicectl_doc(**fields):
    """A devicectl --json-output shaped document with the fields nested
    the way the real tool nests them (hardwareProperties /
    deviceProperties)."""
    hw, dev = {}, {}
    for k, v in fields.items():
        if k in ("marketingName", "productType"):
            hw[k] = v
        else:
            dev[k] = v
    return {"info": {"arguments": ["device", "info", "details"]},
            "result": {"devices": [{
                "identifier": "test-udid",
                "hardwareProperties": hw,
                "deviceProperties": dev,
            }]}}


class TestDeviceInfoParse(unittest.TestCase):
    def test_identity_fields(self):
        st = bench.parse_device_info(devicectl_doc(
            marketingName="iPhone 16 Pro", productType="iPhone17,1",
            osVersionNumber="27.0"))
        self.assertEqual(st, {"model": "iPhone 16 Pro",
                              "product_type": "iPhone17,1",
                              "os_version": "27.0"})

    def test_unreported_fields_stay_absent(self):
        st = bench.parse_device_info(devicectl_doc(deviceName="phone"))
        self.assertEqual(st, {"model": "phone"})


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

    def test_flatten_skips_error_rows(self):
        results = {"runs": [
            {"contestant": "a", "workload": "w1", "error": "x",
             "metrics": {"t": {"m": {"unit": "s", "measurements": [9.0]}}}},
            {"contestant": "a", "workload": "w1",
             "metrics": {"t": {"m": {"unit": "s",
                                     "measurements": [1.0, 2.0]}}}},
        ]}
        table = bench.flatten(results)
        self.assertEqual(table[("a", "w1")]["t:m"]["median"], 1.5)


HEAD = "a1b2c3d4" * 5
SIZE = {"app_bytes": 3_000_000, "unsigned_ipa_bytes": 1_000_000,
        "thinned_ipa_bytes": 900_000, "thinned_app_bytes": 2_500_000}


def synth_results(reps=5, drop_cell=None, fail_reps=(), sizes=True):
    """A device results document covering every required cell, with
    `fail_reps` rep indices recorded as error rows (never counted) and
    `drop_cell`=(cid,w) left unrecorded."""
    runs = []
    for cid, w in sorted(bench.required_cells()):
        if (cid, w) == drop_cell:
            continue
        for rep in range(reps):
            row = {"contestant": cid, "workload": w,
                   "drive": bench.drive_for(w), "repeat": rep}
            if rep in fail_reps:
                row["error"] = "injected launch failure"
            else:
                # the producer's real shape: one dict per test
                # identifier, metric tails with unit + measurements
                row.update({"frames": 1400, "metrics": {
                    "testLaunch": {"duration": {
                        "unit": "s", "measurements": [1.0 + rep * 0.01]}},
                    "testWorkload": {
                        "physical_peak": {"unit": "kB",
                                          "measurements": [64000]},
                        "time": {"unit": "s", "measurements": [0.4]}}}})
            runs.append(row)
    return {"machine": {"hw_model": "Macmini9,1", "hw_uuid": "U"},
            "device": {"udid": "D", "model": "iPhone 16 Pro",
                       "product_type": "iPhone17,1"},
            "build": {"checkout_head": HEAD},
            "runs": runs,
            "sizes": {c["id"]: dict(SIZE) for c in bench.MANIFEST[
                "contestants"]} if sizes else {}}


def run_report(doc):
    """Run the real report command on a temp results file; returns the
    report text (SystemExit propagates)."""
    tmp = Path(tempfile.mkdtemp(prefix="bench-report-"))
    p = tmp / "r.json"
    p.write_text(json.dumps(doc))
    out = tmp / "report.md"
    try:
        bench.cmd_report(argparse.Namespace(input=str(p), out=str(out)))
    finally:
        text = out.read_text() if out.exists() else ""
        shutil.rmtree(tmp)
    return text


class TestReporting(unittest.TestCase):
    def test_flatten_reports_n_min_max_spread(self):
        table = bench.flatten(synth_results())
        cell = next(iter(table.values()))["testLaunch:duration"]
        self.assertEqual(cell["n"], 5)
        self.assertAlmostEqual(cell["median"], 1.02)
        self.assertAlmostEqual(cell["min"], 1.0)
        self.assertAlmostEqual(cell["max"], 1.04)
        self.assertAlmostEqual(cell["spread"], 0.04, places=6)

    def test_complete_dataset_passes_and_shows_n(self):
        text = run_report(synth_results())
        self.assertIn("n=5", text)
        self.assertIn("thinned .ipa", text)
        self.assertNotIn("DATASET INCOMPLETE", text)

    def test_missing_cell_fails_report(self):
        cell = next(iter(sorted(bench.required_cells())))
        with self.assertRaises(SystemExit):
            run_report(synth_results(drop_cell=cell))

    def test_missing_thinned_size_fails_report(self):
        doc = synth_results()
        doc["sizes"]["waterui"] = {"app_bytes": 1,
                                   "error": "thinning: injected"}
        with self.assertRaises(SystemExit):
            run_report(doc)

    def test_all_failed_and_mixed_cells_fail(self):
        # 2 of 5 reps are error rows -> 3 successful < floor
        with self.assertRaises(SystemExit):
            run_report(synth_results(fail_reps=(3, 4)))
        doc = synth_results()
        target = next(iter(sorted(bench.required_cells())))
        for r in doc["runs"]:
            if (r["contestant"], r["workload"]) == target:
                r.pop("metrics", None)
                r["error"] = "injected xcresult failure"
        with self.assertRaises(SystemExit) as ctx:
            run_report(doc)
        self.assertIn("incomplete", str(ctx.exception))


class TestResumeGate(unittest.TestCase):
    STAGING = {"checkout_head": HEAD, "artifacts": {"waterui": {
        "sha256": "s"}}}
    MACHINE = {"hw_model": "Macmini9,1", "hw_uuid": "U"}

    def state(self, **over):
        st = {"build": self.STAGING, "machine": self.MACHINE,
              "device": {"udid": "D"}, "runs": []}
        st.update(over)
        return st

    def test_same_build_host_and_device_merge(self):
        bench.check_resume(self.state(), self.STAGING, self.MACHINE, "D")

    def test_other_build_host_or_device_refused(self):
        for st, why in (
                (self.state(build={**self.STAGING, "checkout_head": "f"}),
                 "staged build"),
                (self.state(machine={**self.MACHINE, "hw_uuid": "V"}),
                 "device host"),
                (self.state(device={"udid": "E"}), "different device")):
            with self.assertRaises(SystemExit) as ctx:
                bench.check_resume(st, self.STAGING, self.MACHINE, "D")
            self.assertIn(why, str(ctx.exception))

    def test_unknown_identity_is_never_the_same_host(self):
        """A recorded host without hw_uuid/hw_model is refused even
        against a current host equally unknown."""
        blank = {"hw_model": None, "hw_uuid": None}
        with self.assertRaises(SystemExit) as ctx:
            bench.check_resume(self.state(machine=blank), self.STAGING,
                               blank, "D")
        self.assertIn("no device host identity", str(ctx.exception))


class TestDeviceCleanup(unittest.TestCase):
    def test_sh_real_failure_and_timeout(self):
        with self.assertRaises(RuntimeError):
            bench.sh("exit 7")
        # a timeout raises, never returns a value standing for it
        with self.assertRaises(bench.CommandTimeout) as cm:
            bench.sh("sleep 5", timeout=1)
        self.assertIn("timed out after 1s", str(cm.exception))


CONTESTANT_BID = bench.MANIFEST["harness"]["contestant_bundle_id"]
RUNNER_BID = bench.MANIFEST["runner"]["bundle_id"] + ".xctrunner"


class AppStore:
    """An in-memory iPhone for the install cycle: the apps it holds per
    bundle id, answered as a `devicectl device info apps` document parsed
    by the real bench.listed_apps. Installing reads the bundle id from
    the app's own Info.plist and lists the app at a percent-encoded file
    URL, the way devicectl does. Failures are the device's behaviour:
    `sticky` keeps an app listed after its uninstall, `list_fails` makes
    every listing fail, `installs_as` names the .app the device ends up
    holding instead of the one installed."""

    def __init__(self, sticky=False, list_fails=False, installs_as=None):
        self.apps: dict[str, list[dict]] = {}
        self.calls: list[tuple[str, str]] = []
        self.sticky = sticky
        self.list_fails = list_fails
        self.installs_as = installs_as

    @staticmethod
    def entry(bundle_id, app_name):
        return {"bundleIdentifier": bundle_id,
                "name": app_name.removesuffix(".app"),
                "url": "file:///private/var/containers/Bundle/Application/"
                       "0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0/"
                       + urllib.parse.quote(app_name) + "/",
                "version": "1.0", "bundleVersion": "1",
                "removable": True, "builtByDeveloper": True}

    def listed(self, bundle_id):
        self.calls.append(("listed", bundle_id))
        if self.list_fails:
            raise RuntimeError("command failed (1): xcrun devicectl device "
                               "info apps")
        return bench.listed_apps(
            {"info": {"arguments": ["device", "info", "apps"]},
             "result": {"apps": [e for es in self.apps.values()
                                 for e in es]}},
            bundle_id)

    def uninstall(self, bundle_id):
        self.calls.append(("uninstall", bundle_id))
        if not self.sticky:
            self.apps.pop(bundle_id, None)

    def install(self, app):
        self.calls.append(("install", app.name))
        bid = plistlib.loads((app / "Info.plist").read_bytes())[
            "CFBundleIdentifier"]
        self.apps[bid] = [self.entry(bid, self.installs_as or app.name)]


class TestSharedBundleInstall(unittest.TestCase):
    """Every contestant installs under one bundle id: before each install
    the id is cleared and verified absent on the device, the installed
    app must be the stage entry's, and every step is recorded."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="bench-install-"))
        # a signed copy as run_device makes it: the stage entry's .app
        # name, the shared id
        self.app = self.tmp / "WaterUI Bench.app"
        self.app.mkdir()
        (self.app / "Info.plist").write_bytes(plistlib.dumps(
            {"CFBundleIdentifier": CONTESTANT_BID}))

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def device(self, **kw):
        dev = AppStore(**kw)
        # the runner is installed throughout and never matches the id
        dev.apps[RUNNER_BID] = [AppStore.entry(RUNNER_BID,
                                               "BenchRunner-Runner.app")]
        return dev

    def assertUtc(self, value):
        t = datetime.datetime.fromisoformat(value)
        self.assertEqual(t.utcoffset(), datetime.timedelta(0))

    def test_manifest_declares_two_app_ids(self):
        self.assertEqual(CONTESTANT_BID, "dev.bench.contestant")
        self.assertNotEqual(CONTESTANT_BID, RUNNER_BID)
        for c in bench.MANIFEST["contestants"]:
            self.assertNotIn("bundle_id", c)

    def test_clean_device(self):
        dev, installed = self.device(), []
        cycle = bench.install_cycle(dev, CONTESTANT_BID, self.app,
                                    installed)
        self.assertNotIn("error", cycle)
        pre = cycle["pre_install"]
        self.assertEqual((pre["bundle_id"], pre["listed_before"],
                          pre["uninstall"], pre["listed_after"],
                          pre["verified_absent"]),
                         (CONTESTANT_BID, [], "not installed", [], True))
        for k in ("started_utc", "uninstall_finished_utc", "verified_utc"):
            self.assertUtc(pre[k])
        ins = cycle["install"]
        for k in ("started_utc", "finished_utc", "verified_utc"):
            self.assertUtc(ins[k])
        self.assertEqual([bench.installed_bundle_name(a)
                          for a in ins["listed"]], ["WaterUI Bench.app"])
        # verified absent before the install, checked after it
        self.assertEqual(dev.calls, [("listed", CONTESTANT_BID),
                                     ("listed", CONTESTANT_BID),
                                     ("install", self.app.name),
                                     ("listed", CONTESTANT_BID)])
        self.assertEqual(installed, [CONTESTANT_BID])
        # the post-cells uninstall: removed and verified, the runner kept
        post = bench.clear_bundle_id(dev, CONTESTANT_BID)
        self.assertEqual((post["uninstall"], post["verified_absent"]),
                         ("uninstalled", True))
        self.assertEqual(list(dev.apps), [RUNNER_BID])

    def test_previous_contestant_removed_before_install(self):
        dev, installed = self.device(), []
        dev.apps[CONTESTANT_BID] = [AppStore.entry(CONTESTANT_BID,
                                                   "Runner.app")]
        cycle = bench.install_cycle(dev, CONTESTANT_BID, self.app,
                                    installed)
        self.assertNotIn("error", cycle)
        pre = cycle["pre_install"]
        self.assertEqual(pre["uninstall"], "uninstalled")
        self.assertEqual([bench.installed_bundle_name(a)
                          for a in pre["listed_before"]], ["Runner.app"])
        self.assertTrue(pre["verified_absent"])
        self.assertEqual([c[0] for c in dev.calls],
                         ["listed", "uninstall", "listed", "install",
                          "listed"])

    def test_failed_verification_installs_nothing(self):
        dev, installed = self.device(sticky=True), []
        dev.apps[CONTESTANT_BID] = [AppStore.entry(CONTESTANT_BID,
                                                   "Runner.app")]
        cycle = bench.install_cycle(dev, CONTESTANT_BID, self.app,
                                    installed)
        self.assertIn("still installed", cycle["error"])
        self.assertFalse(cycle["pre_install"]["verified_absent"])
        self.assertUtc(cycle["pre_install"]["verified_utc"])
        self.assertNotIn("install", cycle)
        self.assertNotIn(("install", self.app.name), dev.calls)
        self.assertEqual(installed, [])

    def test_listing_failure_installs_nothing(self):
        dev, installed = self.device(list_fails=True), []
        cycle = bench.install_cycle(dev, CONTESTANT_BID, self.app,
                                    installed)
        self.assertIn("devicectl device info apps", cycle["error"])
        self.assertNotIn("verified_absent", cycle["pre_install"])
        self.assertNotIn("install", cycle)
        self.assertEqual(installed, [])

    def test_installed_app_must_be_the_stage_entry(self):
        dev, installed = self.device(installs_as="RnBench.app"), []
        cycle = bench.install_cycle(dev, CONTESTANT_BID, self.app,
                                    installed)
        self.assertIn("RnBench.app", cycle["error"])
        # it was installed: the caller still uninstalls it
        self.assertEqual(installed, [CONTESTANT_BID])

    def test_listing_without_apps_is_not_empty(self):
        for doc in ({"result": {}}, {"result": {"apps": None}}, []):
            with self.assertRaises(RuntimeError):
                bench.listed_apps(doc, CONTESTANT_BID)

    def test_stage_entries(self):
        arts = {c["id"]: {"app": Path(c["artifact"]).name}
                for c in bench.MANIFEST["contestants"]}
        bench.check_stage_entries(arts)
        with self.assertRaises(SystemExit):
            bench.check_stage_entries(
                {k: v for k, v in arts.items() if k != "rn"})
        with self.assertRaises(SystemExit) as cm:
            bench.check_stage_entries({**arts, "rn": arts["flutter"]})
        self.assertIn("not distinct", str(cm.exception))


class TestSanitizeKeepsErrors(unittest.TestCase):
    """N4: a failed attempt is evidence — sanitize drops superseded
    harness rows (missing/changed drive), never error records."""

    def test_error_rows_survive_sanitize(self):
        state = {"runs": [
            {"contestant": "waterui", "workload": None, "repeat": 0,
             "error": "missing artifact /x/Bench.app"},
            {"contestant": "waterui", "workload": "w1", "drive": "tap",
             "repeat": 0, "error": "thermal gate closed"},
            {"contestant": "waterui", "workload": "w1",
             "drive": "stale-drive", "repeat": 0,
             "metrics": {"t": {"m": {"unit": "s", "measurements": [1]}}}},
            {"contestant": "waterui", "workload": "w1", "drive": "tap",
             "repeat": 1,
             "metrics": {"t": {"m": {"unit": "s", "measurements": [1]}}}},
        ]}
        bench.sanitize_runs(state)
        self.assertEqual([r["repeat"] for r in state["runs"]], [0, 0, 1])


class TestRunFloors(unittest.TestCase):
    """N9: a cell needs the required metric tails + owned frames, not
    any metrics dict."""

    FULL = {"testLaunch": {"duration": {"unit": "s", "measurements": [1.0]}},
            "testWorkload": {
                "physical_peak": {"unit": "kB", "measurements": [1]},
                "time": {"unit": "s", "measurements": [1]}}}

    def _run(self, metrics, frames=42):
        return {"contestant": "a", "workload": "w1", "repeat": 0,
                "metrics": metrics, "frames": frames}

    def test_full_shape_passes(self):
        self.assertTrue(bench._run_succeeded(self._run(self.FULL)))

    def test_missing_memory_fails(self):
        m = {**self.FULL, "testWorkload": {"time": {
            "unit": "s", "measurements": [1]}}}
        self.assertFalse(bench._run_succeeded(self._run(m)))

    def test_no_frames_or_error_fails(self):
        self.assertFalse(bench._run_succeeded(self._run(self.FULL, 0)))
        self.assertFalse(bench._run_succeeded(
            {**self._run(self.FULL), "error": "frame attribution: x"}))


class TestCapacitySummary(unittest.TestCase):
    """Budget shares are over the step's presents; collapse is fewer than
    half the presents inside two 60 Hz budgets (WORKLOADS.md)."""

    @staticmethod
    def step(n, intervals):
        return {"n": n, "frame_stats": {"presents": len(intervals) + 1,
                                        "intervals_ms": intervals}}

    def test_capacities_and_collapse(self):
        steps = [self.step(200, [8.3] * 999),          # 99.9% at 120 Hz
                 self.step(400, [16.7] * 999),         # 60 Hz only
                 self.step(800, [40.0] * 99)]          # collapsed
        cap = bench.capacity_summary(steps)
        self.assertEqual(cap, {"capacity_120hz": 200,
                               "capacity_60hz": 400})
        self.assertEqual([s["collapsed"] for s in steps],
                         [False, False, True])
        self.assertEqual(steps[1]["in_120hz_pct"], 0.0)

    def test_no_presents_is_collapsed(self):
        st = {"n": 25600, "frame_stats": {"presents": 0,
                                          "intervals_ms": []}}
        bench.capacity_summary([st])
        self.assertTrue(st["collapsed"])


class TestProfileSelection(unittest.TestCase):
    """A profile signs a bundle id only for its exact App ID, the
    signing identity's certificate, the device, and the validity
    margin — a wildcard is never assumed."""

    CERT = b"der-certificate"

    def setUp(self):
        import hashlib
        self.ident = bench.Identity(
            hashlib.sha1(self.CERT).hexdigest().upper(),
            "Apple Development: someone (ABCDE12345)")
        self.now = bench.datetime.datetime(2026, 10, 5)
        self.doc = {"TeamIdentifier": ["4AZ53N9R83"],
                    "Entitlements": {"application-identifier":
                                     "4AZ53N9R83.dev.bench.contestant"},
                    "DeveloperCertificates": [self.CERT],
                    "ProvisionedDevices": ["UDID"],
                    "ExpirationDate": bench.datetime.datetime(2026, 10, 9)}

    def why(self, **over):
        return bench.profile_rejection({**self.doc, **over},
                                       "dev.bench.contestant", self.ident,
                                       "UDID", self.now)

    def test_exact_profile_accepted(self):
        self.assertIsNone(self.why())

    def test_each_mismatch_rejected(self):
        self.assertIn("App ID", self.why(Entitlements={
            "application-identifier": "4AZ53N9R83.*"}))
        self.assertIn("certificate", self.why(
            DeveloperCertificates=[b"other"]))
        self.assertIn("device", self.why(ProvisionedDevices=["OTHER"]))
        self.assertIn("expires", self.why(
            ExpirationDate=bench.datetime.datetime(2026, 10, 4)))


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
    """The runner's marks + on-device record are read from the runner
    log by the rep's run nonce; another rep's lines can't alias in,
    whatever their device time."""

    def log(self, text):
        tmp = Path(tempfile.mkdtemp(prefix="bench-rlog-"))
        self.addCleanup(shutil.rmtree, tmp)
        p = tmp / "bench-runner.log"
        p.write_text(text)
        return p

    def test_marks_and_device_record_by_nonce(self):
        # rep 7 ran later on the device clock than rep 9: time decides
        # nothing, the nonce does
        log = self.log(
            "9 100.0 testWorkload: launching x w=w1 drive=tap\n"
            "9 199.9 device-record thermal=0 maxFps=120\n"
            "9 200.0 drive-begin\n"
            "9 207.0 measure-end\n"
            "7 300.0 device-record thermal=1 maxFps=60\n"
            "7 300.5 drive-begin\n"
            "7 309.0 measure-end\n")
        marks, dev = bench.read_runner_log(log, 9)
        self.assertEqual(marks, {"drive-begin": 200.0,
                                 "measure-end": 207.0})
        self.assertEqual(dev, {"thermal": "0", "maxFps": "120"})
        marks, dev = bench.read_runner_log(log, 7)
        self.assertEqual(dev, {"thermal": "1", "maxFps": "60"})
        # a nonce the log does not carry has neither
        self.assertEqual(bench.read_runner_log(log, 5), ({}, {}))

    def test_unpaired_or_repeated_mark_is_no_window(self):
        self.assertEqual(bench.read_runner_log(
            self.log("3 200.0 drive-begin\n"), 3)[0], {})
        self.assertEqual(bench.read_runner_log(self.log(
            "3 200.0 drive-begin\n3 201.0 drive-begin\n"
            "3 209.0 measure-end\n"), 3)[0], {})


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
        rows, err = bench.parse_export_rows(self.DOC, "time-sample")
        self.assertIsNone(err)
        self.assertEqual(rows[0]["time"], "1000000")
        self.assertEqual(rows[0]["thread-state"], "Running")
        # the ref repeats the defining element's value
        self.assertEqual(bench._pid_of_thread(rows[1]["thread"]), 42)
        self.assertIsNone(rows[1]["thread-state"])

    def test_cell_count_mismatch_is_an_error(self):
        doc = self.DOC.replace("<sentinel/>", "")
        rows, err = bench.parse_export_rows(doc, "time-sample")
        self.assertIsNone(rows)
        self.assertIn("schema columns", err)

    def test_process_cell_name(self):
        self.assertEqual(bench._proc_name("WaterUI Bench (311)"),
                         "WaterUI Bench")
        self.assertIsNone(bench._proc_name(None))
        self.assertEqual(bench._pid_of_process("WaterUI Bench (311)"), 311)

    SIGNPOST_NODE = """<node xpath="a"><schema name="os-signpost">
<col><mnemonic>time</mnemonic></col><col><mnemonic>name</mnemonic></col>
</schema>
<row><event-time id="1">10</event-time><string id="2">drive-begin</string></row>
</node>"""

    def test_schemaless_auxiliary_node_is_not_a_table(self):
        """Xcode 26.6 exports the os-signpost table as two nodes, the
        second without a <schema>: rows come from the schema node."""
        doc = ("<?xml version=\"1.0\"?><trace-query-result>"
               + self.SIGNPOST_NODE
               + '<node xpath="b"/></trace-query-result>')
        rows, err = bench.parse_export_rows(doc, "os-signpost")
        self.assertIsNone(err)
        self.assertEqual(rows, [{"time": "10", "name": "drive-begin"}])

    def test_exactly_one_schema_node(self):
        """Two schema nodes mean the xpath named more than one table;
        none, or one of another schema, names no table of this one."""
        two = ("<?xml version=\"1.0\"?><trace-query-result>"
               + self.SIGNPOST_NODE + self.SIGNPOST_NODE.replace('"a"', '"b"')
               + "</trace-query-result>")
        rows, err = bench.parse_export_rows(two, "os-signpost")
        self.assertIsNone(rows)
        self.assertIn("2 <node>s", err)
        none = ('<?xml version="1.0"?><trace-query-result><node xpath="b"/>'
                "</trace-query-result>")
        rows, err = bench.parse_export_rows(none, "os-signpost")
        self.assertIsNone(rows)
        self.assertIn("0 <node>s", err)
        rows, err = bench.parse_export_rows(self.DOC, "os-signpost")
        self.assertIsNone(rows)
        self.assertIn("time-sample", err)


class TestFrameAttribution(unittest.TestCase):
    """H2/M2: frames are the contestant's own by the swap join, and the
    window is [first owned present + warmup, + capture] on the trace
    clock, gated by the runner's drive-begin / measure-end marks."""

    BUNDLE = "WaterUI Bench.app"
    MAIN = "WaterUI Bench"
    APP = "/private/var/containers/Bundle/Application/X/WaterUI Bench.app"
    PROCS = [
        {"name": "backboardd", "pid": 150,
         "path": "/usr/libexec/backboardd"},
        {"name": "WaterUI Bench", "pid": 700,
         "path": f"{APP}/WaterUI Bench"},
        {"name": "helper", "pid": 701,
         "path": f"{APP}/Frameworks/H.framework/helper"},
        {"name": "SpringBoard", "pid": 300,
         "path": "/System/Library/CoreServices/SpringBoard.app/SpringBoard"},
    ]

    def test_toc_processes(self):
        toc = """<?xml version="1.0"?>
<trace-toc><run number="1"><processes>
<process name="kernel" pid="0"/>
<process name="WaterUI Bench" pid="700" path="/x/WaterUI Bench.app/WaterUI Bench"/>
</processes></run></trace-toc>"""
        procs = bench.parse_toc_processes(toc)
        self.assertEqual(procs[1], {"name": "WaterUI Bench", "pid": 700,
                                    "path": "/x/WaterUI Bench.app/"
                                            "WaterUI Bench"})
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
    def update(pid, swap, display="1"):
        return {"process": f"P ({pid})", "swap-id": str(swap),
                "display": display}

    def test_swap_join_keeps_only_owned_frames(self):
        frames = [self.frame(1_000, 500, 1),      # menu bar only
                  self.frame(2_000, 500, 2),      # contestant
                  self.frame(3_000, 500, 3),      # helper (owned)
                  self.frame(4_000, None, 4),     # never presented
                  self.frame(5_000, 500, 2, "2")]  # swap 2 of another display
        updates = [self.update(300, 1), self.update(700, 2),
                   self.update(701, 3), self.update(700, 4),
                   # updates without a swap id join nothing — counted
                   dict(self.update(700, 0), **{"swap-id": None}),
                   dict(self.update(300, 0), **{"swap-id": None})]
        j = bench.owned_presents(frames, updates, {700, 701})
        self.assertEqual(j["presents_ns"], [2_500, 3_500])
        self.assertEqual(j["display"], "1")
        self.assertEqual(j["updates_owned"], 3)
        self.assertEqual((j["updates_without_swap"],
                          j["owned_updates_without_swap"]), (2, 1))

    def test_swap_join_refuses_unattributable(self):
        # a trace without a single frame lifetime recorded no frames
        with self.assertRaises(bench.TraceAttributionError) as cm:
            bench.owned_presents([], [self.update(700, 1)], {700})
        self.assertIn("0 rows", str(cm.exception))
        with self.assertRaises(bench.TraceAttributionError):
            bench.owned_presents([self.frame(0, 1, 1)],
                                 [self.update(300, 1)], {700})
        with self.assertRaises(bench.TraceAttributionError):
            bench.owned_presents([self.frame(0, 1, 9)],
                                 [self.update(700, 1)], {700})
        two_displays = [self.frame(0, 1, 1, "1"), self.frame(5, 1, 1, "2")]
        with self.assertRaises(bench.TraceAttributionError):
            bench.owned_presents(two_displays,
                                 [self.update(700, 1, "1"),
                                  self.update(700, 1, "2")], {700})
        # a schema without the join columns fails, never joins on nothing
        with self.assertRaises(bench.TraceAttributionError):
            bench.owned_presents([{"start": "0", "duration": "1"}],
                                 [self.update(700, 1)], {700})

    def test_frames_classified_by_their_updates(self):
        """The evidence the attribution rule is decided on: each frame on
        the contestant's display inside the span, by whose updates its
        swap carried."""
        frames = [self.frame(1_000, 10, 1),       # owned only
                  self.frame(2_000, 10, 2),       # owned + menu bar
                  self.frame(3_000, 10, 3),       # menu bar only
                  # no client update, on the surface 700 updated
                  dict(self.frame(4_000, 10, 4), **{"surface-id": "9"}),
                  self.frame(4_500, 10, 7),       # no client update
                  self.frame(5_000, 10, 5, "2"),  # other display
                  self.frame(9_000, 10, 6)]       # after the span
        updates = [dict(self.update(700, 1), **{"surface-id": "9"}),
                   self.update(700, 2),
                   self.update(300, 2), self.update(300, 3),
                   self.update(300, 5, "2"), self.update(700, 6)]
        c = bench.classify_display_frames(frames, updates, {700}, "1",
                                          1_000, 5_000)
        self.assertEqual((c["owned"], c["owned+foreign"], c["foreign"],
                          c["no-client-update"],
                          c["no-client-update-on-owned-surface"]),
                         (1, 1, 1, 2, 1))
        self.assertEqual(c["foreign_updaters"],
                         [{"process": "P (300)", "frames": 2}])
        # while the render-server rule is undecided, a window holding a
        # frame without any client update fails the rep
        with self.assertRaises(bench.TraceAttributionError) as cm:
            bench.require_client_updates(c)
        self.assertIn("2 frame(s)", str(cm.exception))
        # a window whose every frame carried a client update passes
        bench.require_client_updates(bench.classify_display_frames(
            frames, updates, {700}, "1", 1_000, 3_500))

    def test_frame_without_swap_id_fails_the_rep(self):
        """A presented frame carrying no swap-id cannot join the client
        updates at all: on the contestant's display inside the window it
        is counted `no-swap-id` and fails the rep; outside the window it
        is not counted."""
        frames = [self.frame(1_000, 10, 1),                  # owned
                  dict(self.frame(2_000, 10, 0),
                       **{"swap-id": None}),                 # no swap-id
                  dict(self.frame(9_000, 10, 0),
                       **{"swap-id": None})]                 # outside
        updates = [self.update(700, 1)]
        c = bench.classify_display_frames(frames, updates, {700}, "1",
                                          500, 5_000)
        self.assertEqual(c["no-swap-id"], 1)
        self.assertEqual(c["owned"], 1)
        with self.assertRaises(bench.TraceAttributionError) as cm:
            bench.require_client_updates(c)
        self.assertIn("1 frame(s)", str(cm.exception))
        # the swap-id-less frame outside the window is not counted
        c = bench.classify_display_frames(frames, updates, {700}, "1",
                                          5_000, 10_000)
        self.assertEqual(c["no-swap-id"], 1)
        self.assertEqual(c["owned"], 0)
        # a window holding none of them passes
        bench.require_client_updates(bench.classify_display_frames(
            frames, updates, {700}, "1", 500, 1_500))

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
                                     capture_ms=12000, tolerance_ms=100,
                                     driven=True)
        self.assertEqual(w["window_start_ms"], 3500.0)
        self.assertEqual(w["window_end_ms"], 15500.0)
        self.assertEqual(w["drive_offset_ms"], -20.0)
        # a drive outside tolerance is not "at window start"
        late = dict(marks, **{"drive-begin": 3_700 * ms})
        with self.assertRaises(bench.TraceAttributionError):
            bench.measurement_window(
                presents, late, warmup_ms=3000, capture_ms=12000,
                tolerance_ms=100, driven=True)
        # an undriven cell (W3/W5) has no drive to align: the offset is
        # reported, not gated
        w = bench.measurement_window(
            presents, late, warmup_ms=3000, capture_ms=12000,
            tolerance_ms=100, driven=False)
        self.assertEqual(w["drive_offset_ms"], 200.0)
        # released before the window end: the tail is not the workload,
        # driven or not
        for driven in (True, False):
            with self.assertRaises(bench.TraceAttributionError):
                bench.measurement_window(
                    presents, dict(marks, **{"measure-end": 15_400 * ms}),
                    warmup_ms=3000, capture_ms=12000, tolerance_ms=100,
                    driven=driven)

    def test_stats_clip_to_trace_window(self):
        """Frame statistics cover exactly the trace window, however long
        the runner held past it (e.g. a late dev.bench.end)."""
        ms = 1_000_000
        presents = [0] + [(3_000 + 10 * i) * ms for i in range(0, 200)]
        presents[0] = 0
        w = bench.measurement_window(
            presents, {"drive-begin": 3_000 * ms,
                       "measure-end": 6_000 * ms},
            warmup_ms=3000, capture_ms=1000, tolerance_ms=100, driven=True)
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


# real kqueue process events, lsof and codesign: the Apple hosts this leg
# runs on (build host and device host) are macOS
@unittest.skipUnless(sys.platform == "darwin",
                     "exercises macOS kqueue, lsof and codesign")
class TestInstrumentsScratch(unittest.TestCase):
    """The cell owns the raw ktrace scratch its recording leaves: what
    appeared in the user temp dir or the recorder's scratch dir during
    the cell is removed (real lsof, real files); what predates the cell
    stays. The SIGTERM path runs against real processes — copies of
    /bin/sleep holding a scratch file open, one of them named
    DTServiceHub: exactly one DTServiceHub holder is terminated and
    logged; two DTServiceHub pids, a holder of another name, or several
    holders fail the sweep and terminate nothing."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="bench-scratch-"))
        self.user = self.tmp / "T"
        self.user.mkdir()
        self.root = self.tmp / "results"
        self.root.mkdir()
        self.children = []

    def tearDown(self):
        for c in self.children:
            if c.poll() is None:
                c.kill()
            c.wait(timeout=30)
            c.stdin.close()
        shutil.rmtree(self.tmp)

    def hold(self, name: str, f: Path):
        """A copy of /bin/sleep named `name` holding `f` open (fd 3,
        opened by the shell that execs the copy). Returns once the copy
        runs: the shell execs it only after the kqueue NOTE_EXEC watch on
        its pid is registered, so the event cannot be missed."""
        import select
        import subprocess
        exe = self.tmp / "bin" / name
        if not exe.exists():
            exe.parent.mkdir(exist_ok=True)
            shutil.copy("/bin/sleep", exe)
            # a copy of a platform binary outside the system paths is
            # killed at exec unless it carries its own (ad hoc) signature
            subprocess.run(["codesign", "--remove-signature", str(exe)],
                           check=True, capture_output=True)
            subprocess.run(["codesign", "--force", "--sign", "-", str(exe)],
                           check=True, capture_output=True)
        child = subprocess.Popen(
            ["/bin/sh", "-c", 'exec 3<"$1"; read go; exec "$0" 600',
             str(exe), str(f)], stdin=subprocess.PIPE)
        self.children.append(child)
        kq = select.kqueue()
        try:
            kq.control([select.kevent(
                child.pid, filter=select.KQ_FILTER_PROC,
                flags=select.KQ_EV_ADD | select.KQ_EV_ONESHOT,
                fflags=select.KQ_NOTE_EXEC)], 0, 0)
            child.stdin.write(b"go\n")
            child.stdin.flush()
            fired = kq.control(None, 1, 30)
        finally:
            kq.close()
        self.assertTrue(fired, f"{name} copy never started")
        return child

    def test_sweep_removes_the_cells_scratch(self):
        old = self.user / "instruments-before.ktrace"
        old.write_bytes(b"x")
        s = bench.InstrumentsScratch(self.root, self.user)
        sd = s.scratch_dir("cell")
        (self.user / "instrumentsAB12.ktrace").write_bytes(b"k" * 10)
        (sd / "instrumentsCD34.ktrace").write_bytes(b"k")
        self.assertIsNone(s.sweep())
        self.assertTrue(old.exists())
        self.assertFalse((self.user / "instrumentsAB12.ktrace").exists())
        self.assertFalse(sd.exists())
        self.assertEqual(sorted(r["bytes"] for r in s.removed), [1, 10])

    def test_one_hub_holder_is_terminated_and_logged(self):
        import contextlib
        import io
        import signal
        s = bench.InstrumentsScratch(self.root, self.user)
        f = self.user / "instrumentsHB01.ktrace"
        f.write_bytes(b"k")
        hub = self.hold("DTServiceHub", f)
        log = io.StringIO()
        with contextlib.redirect_stdout(log):
            err = s.sweep(bound_s=30)
        self.assertIsNone(err)
        self.assertFalse(f.exists())
        self.assertEqual(s.removed[0]["holders"], {hub.pid: "DTServiceHub"})
        self.assertEqual(s.terminated, hub.pid)
        self.assertEqual(hub.wait(timeout=30), -signal.SIGTERM)
        self.assertIn(f"SIGTERM DTServiceHub pid {hub.pid}", log.getvalue())

    def test_two_hub_holders_fail_and_terminate_nothing(self):
        s = bench.InstrumentsScratch(self.root, self.user)
        files = [self.user / f"instrumentsHB{i}.ktrace" for i in (2, 3)]
        hubs = []
        for f in files:
            f.write_bytes(b"k")
            hubs.append(self.hold("DTServiceHub", f))
        err = s.sweep(bound_s=30)
        self.assertIn("2 DTServiceHub pids", err)
        self.assertIsNone(s.terminated)
        self.assertEqual([h.poll() for h in hubs], [None, None])

    def test_foreign_holder_fails_and_terminates_nothing(self):
        s = bench.InstrumentsScratch(self.root, self.user)
        f = self.user / "instrumentsEF56.ktrace"
        f.write_bytes(b"k")
        other = self.hold("NotTheHub", f)
        err = s.sweep(bound_s=30)
        self.assertIn("held open by NotTheHub", err)
        self.assertIn("not DTServiceHub", err)
        self.assertFalse(f.exists())
        self.assertIsNone(s.terminated)
        self.assertIsNone(other.poll())

    def test_several_holders_fail_the_sweep(self):
        import subprocess
        s = bench.InstrumentsScratch(self.root, self.user)
        f = self.user / "instrumentsGH78.ktrace"
        f.write_bytes(b"k")
        child = subprocess.Popen(
            ["/bin/sh", "-c", 'exec 3<"$0"; echo held; read x', str(f)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        try:
            self.assertEqual(child.stdout.readline().strip(), "held")
            with open(f, "rb"):
                err = s.sweep()
        finally:
            child.communicate("\n", timeout=30)
        self.assertIn("held open by 2 processes", err)
        self.assertIsNone(s.terminated)


class TestDeviceLock(unittest.TestCase):
    """A run never waits on the device: a lock another run holds fails
    at once, naming the holder that run recorded."""

    def test_held_lock_fails_naming_the_holder(self):
        tmp = Path(tempfile.mkdtemp(prefix="bench-lock-"))
        orig = bench.LOCK_DIR
        bench.LOCK_DIR = tmp
        try:
            first = bench.device_lock("UDID", {"pid": 11, "run_dir": "/r/a"})
            with self.assertRaises(SystemExit) as cm:
                bench.device_lock("UDID", {"pid": 12, "run_dir": "/r/b"})
            self.assertIn('"run_dir": "/r/a"', str(cm.exception))
            # the failed attempt left the holder's record intact
            self.assertIn('"pid": 11', (tmp / "UDID.lock").read_text())
            first.close()
            second = bench.device_lock("UDID", {"pid": 12, "run_dir": "/r/b"})
            self.assertEqual(json.loads((tmp / "UDID.lock").read_text()),
                             {"pid": 12, "run_dir": "/r/b"})
            second.close()
        finally:
            bench.LOCK_DIR = orig
            shutil.rmtree(tmp)


class TestStageTarball(unittest.TestCase):
    """`device-session start` takes the sha256 `build` printed and
    verifies the tarball against it before anything else."""

    def test_sha256_mismatch_fails(self):
        tmp = Path(tempfile.mkdtemp(prefix="bench-tar-"))
        try:
            tar = tmp / "ios-stage-x.tar.gz"
            tar.write_bytes(b"staged")
            good = bench.toolchain.sha256_file(tar)
            bench.verify_stage(tar, good)
            with self.assertRaises(SystemExit) as cm:
                bench.verify_stage(tar, "0" * 64)
            self.assertIn(good, str(cm.exception))
        finally:
            shutil.rmtree(tmp)

    def test_unpack_reads_the_run_dir_copy(self):
        """device-run hashes and extracts the copy it made into the run
        dir — the original path is never read again (here: deleted)."""
        import tarfile
        tmp = Path(tempfile.mkdtemp(prefix="bench-tar-"))
        try:
            # a minimal staged set: the manifest's contestants, distinct
            # .app names, real dir hashes
            src = tmp / "src" / "stage"
            artifacts = {}
            for c in bench.MANIFEST["contestants"]:
                app = src / f"{c['id']}-app.app"
                app.mkdir(parents=True)
                (app / "bin").write_bytes(c["id"].encode())
                artifacts[c["id"]] = {"app": app.name,
                                      "sha256": bench.dir_sha256(app)}
            (src / "staging-manifest.json").write_text(json.dumps(
                {"checkout_head": "c" * 40, "artifacts": artifacts}))
            tar = tmp / "ios-stage-x.tar"
            with tarfile.open(tar, "w") as tf:
                tf.add(src, arcname="stage")
            run_dir = tmp / "run"
            run_dir.mkdir()
            # the run's flow: copy in, hash and verify the copy
            copy = run_dir / tar.name
            shutil.copy2(tar, copy)
            bench.verify_stage(copy, bench.toolchain.sha256_file(copy))
            # the checkout identity gates are environment checks, not
            # the copy's — pinned to the fixture manifest's values
            saved = (bench.toolchain.require_clean_checkout,
                     bench.toolchain.checkout_head)
            bench.toolchain.require_clean_checkout = lambda: bench.ROOT
            bench.toolchain.checkout_head = lambda: "c" * 40
            try:
                tar.unlink()   # the original is never read again
                stage, staging = bench.unpack_stage(copy, run_dir)
            finally:
                (bench.toolchain.require_clean_checkout,
                 bench.toolchain.checkout_head) = saved
            self.assertEqual(staging["checkout_head"], "c" * 40)
            self.assertTrue((stage / "staging-manifest.json").is_file())
            for c in bench.MANIFEST["contestants"]:
                self.assertTrue(
                    (stage / artifacts[c["id"]]["app"] / "bin").is_file())
        finally:
            shutil.rmtree(tmp)

    def test_start_requires_the_sha256(self):
        orig = sys.argv
        try:
            for argv in (["device-session", "start", "--stage", "x.tar.gz"],
                         ["device-session", "start", "--stage", "x.tar.gz",
                          "--stage-sha256", "not-a-digest"]):
                sys.argv = ["bench.py", *argv]
                with self.assertRaises(SystemExit) as cm:
                    bench.main()
                self.assertEqual(cm.exception.code, 2)
        finally:
            sys.argv = orig


class TestXctestrunInjection(unittest.TestCase):
    """The run nonce, the ladder value, the manifest's fling program and
    the installed contestant's identity reach the runner's
    environment."""

    def test_nonce_and_step(self):
        import plistlib
        tmp = Path(tempfile.mkdtemp(prefix="bench-xr-"))
        try:
            tmpl = tmp / "t.xctestrun"
            tmpl.write_bytes(plistlib.dumps({"BenchRunner": {
                "DependentProductPaths": []}}))
            out = tmp / "o.xctestrun"
            bench.write_xctestrun(tmpl, out, "BenchRunner",
                                  "Release-iphoneos", "X.app",
                                  "dev.bench.contestant", "flutter",
                                  "w5", "none", 12, nonce=12345, step=800)
            t = plistlib.loads(out.read_bytes())["BenchRunner"]
            env = t["EnvironmentVariables"]
            # launched by the shared id, readied by the stage entry's id
            self.assertEqual(env["BENCH_BUNDLE_ID"], "dev.bench.contestant")
            self.assertEqual(env["BENCH_CONTESTANT"], "flutter")
            self.assertEqual(env["BENCH_RUN_NONCE"], "12345")
            self.assertEqual(env["BENCH_STEP"], "800")
            self.assertEqual(env["BENCH_DURATION"], "12")
            self.assertEqual(t["UITargetAppPath"],
                             "__TESTROOT__/Release-iphoneos/X.app")
            fling = json.loads(env["BENCH_FLING"])
            self.assertEqual((fling["flings_down"], fling["flings_up"]),
                             (8, 2))
            self.assertAlmostEqual(fling["pause_s"], 0.35)
        finally:
            shutil.rmtree(tmp)


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
