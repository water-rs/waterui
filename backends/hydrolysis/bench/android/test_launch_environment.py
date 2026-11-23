"""Launch configuration stays outside argv and outlives the measured capture."""

import json
import subprocess
import unittest
from unittest.mock import patch

import run


class LaunchEnvironmentTests(unittest.TestCase):
    def test_private_handoff_cleans_up_after_capture_failure(self):
        values = {"WATERUI_TEST_LABEL": "中文 日本語 한국어 ' $(value)"}
        calls = []

        def command(args, **kwargs):
            calls.append((args, kwargs))
            return subprocess.CompletedProcess(args, 0, stdout=b"", stderr=b"")

        with patch.object(run.e2e.subprocess, "run", side_effect=command):
            with self.assertRaisesRegex(RuntimeError, "capture failed"):
                with run.e2e.launch_environment("device", "dev.waterui.test", values) as args:
                    self.assertEqual(args[:2], ["--es", "waterui.launch.environment"])
                    self.assertEqual(json.loads(calls[0][1]["input"]), values)
                    self.assertEqual(calls[-1][0][-2:], ["force-stop", "dev.waterui.test"])
                    self.assertNotIn(values["WATERUI_TEST_LABEL"], " ".join(args))
                    raise RuntimeError("capture failed")

        self.assertTrue(any("delete" in args for args, _ in calls))
        self.assertEqual(calls[-1][0][-2:], ["force-stop", "dev.waterui.test"])
        for args, _ in calls:
            self.assertNotIn(values["WATERUI_TEST_LABEL"], " ".join(args))

    def test_provider_rejection_fails_before_launch_and_rolls_back(self):
        def command(args, **kwargs):
            return subprocess.CompletedProcess(
                args, 0, stdout=b"", stderr=b"Error while accessing provider" if "write" in args else b""
            )

        with patch.object(run.e2e.subprocess, "run", side_effect=command) as execute:
            with self.assertRaisesRegex(RuntimeError, "could not stage"):
                with run.e2e.launch_environment("device", "dev.waterui.test", {"KEY": "value"}):
                    self.fail("a rejected payload must never reach launch")
            self.assertTrue(any("delete" in call.args[0] for call in execute.call_args_list))

    def test_compose_keeps_its_reference_app_protocol(self):
        with patch.object(run.e2e.subprocess, "run") as execute:
            with run.e2e.launch_environment(
                "device", "dev.waterui.reference", {"KEY": "value"}, compose=True
            ) as args:
                self.assertEqual(args, ["--es", "waterui.env.KEY", "value"])
            execute.assert_not_called()


if __name__ == "__main__":
    unittest.main()
