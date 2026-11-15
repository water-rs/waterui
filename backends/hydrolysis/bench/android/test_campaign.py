"""A campaign cannot silently change the Compose build under measurement."""

import argparse
import io
import json
import shlex
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import run


class CampaignLockTests(unittest.TestCase):
    def test_new_bom_requires_refresh_without_changing_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "reference").mkdir()
            lock = root / "campaign-lock.json"
            props = root / "reference/gradle.properties"
            lock.write_text(json.dumps({"compose_bom": "2026.09.00"}))
            props.write_text("composeBomVersion=2026.09.00\n")
            original = (lock.read_bytes(), props.read_bytes())
            metadata = b"<metadata><version>2026.10.00</version></metadata>"
            with patch.object(run, "BENCH_DIR", root), patch(
                "urllib.request.urlopen", return_value=io.BytesIO(metadata)
            ), self.assertRaisesRegex(SystemExit, "--refresh"):
                run.cmd_lock_campaign(argparse.Namespace(refresh=False))
            self.assertEqual((lock.read_bytes(), props.read_bytes()), original)


class InstrumentationResultTests(unittest.TestCase):
    def test_success_preserves_shell_arguments_and_pulls_reports(self):
        with tempfile.TemporaryDirectory() as directory:
            args = argparse.Namespace(serial="device", fixture="list", klass="StartupBenchmark#cold",
                                      extra_arg=["ready_text=100,000-row lazy List"], out=directory)
            with patch.object(run.metrics, "adb_out", return_value=b"OK (1 test)\n") as adb, patch.object(
                run.subprocess, "run"
            ) as pull:
                self.assertEqual(run.cmd_instrument(args), 0)
            command = shlex.split(adb.call_args.args[2])
            self.assertIn("100,000-row lazy List", command)
            self.assertEqual(command[-1], "dev.waterui.android.macrobenchmark/androidx.test.runner.AndroidJUnitRunner")
            pull.assert_called_once()
            self.assertTrue(pull.call_args.kwargs["check"])

    def test_failed_measurement_preserves_reported_trace(self):
        with tempfile.TemporaryDirectory() as directory:
            args = argparse.Namespace(serial="device", fixture="list", klass="EnergyBenchmark",
                                      extra_arg=None, out=directory)
            output = b"INSTRUMENTATION_STATUS: additionalTestOutputFile_energy=/sdcard/energy.perfetto-trace\nFAILURES!!!"
            with patch.object(run.metrics, "adb_out", return_value=output), patch.object(
                run.subprocess, "run"
            ) as pull, self.assertRaisesRegex(RuntimeError, "diagnostic artifacts were saved"):
                run.cmd_instrument(args)
            pull.assert_called_once()

    def test_failed_instrumentation_does_not_pull_results(self):
        args = argparse.Namespace(serial="device", fixture="list", klass="FrameBenchmark",
                                  extra_arg=None, out="unused")
        for output in (b"INSTRUMENTATION_FAILED: runner missing", b"FAILURES!!!", b"OK (0 tests)"):
            with self.subTest(output=output), patch.object(run.metrics, "adb_out", return_value=output), patch.object(
                run.subprocess, "run"
            ) as pull, self.assertRaisesRegex(RuntimeError, "instrumentation failed"):
                run.cmd_instrument(args)
            pull.assert_not_called()


if __name__ == "__main__":
    unittest.main()
