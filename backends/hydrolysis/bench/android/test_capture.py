"""Keep reference capture outside the measured application's focused journey."""

from contextlib import ExitStack
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import run


class CaptureOrderingTests(unittest.TestCase):
    def test_reference_finishes_before_target_and_comparison_runs_after_stop(self):
        events = []
        with tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
            root = Path(directory)
            apk = root / "list.apk"
            apk.write_bytes(b"test apk")
            stack.enter_context(patch.object(run, "apk_package", return_value="target"))
            for name in ("prepare_display",):
                stack.enter_context(patch.object(run, name))
            stack.enter_context(patch.object(run, "start_launcher", side_effect=lambda _, package, *a, **kw: events.append(("start", package))))
            stack.enter_context(patch.object(run, "capture_first_frame", side_effect=lambda _, package, *a: events.append(("capture", package))))
            stack.enter_context(patch.object(run.e2e, "force_stop", side_effect=lambda _, package: events.append(("stop", package))))
            stack.enter_context(patch.object(run.e2e, "install_apk"))
            stack.enter_context(patch.object(run.e2e, "dump_logcat"))
            stack.enter_context(patch.object(run.metrics, "adb"))
            stack.enter_context(patch.object(run.frame_profile, "clear_logcat"))
            stack.enter_context(patch.object(run.frame_profile, "collect", return_value={}))
            stack.enter_context(patch.object(run.frame_metrics, "collect", return_value={}))
            stack.enter_context(patch.object(run.memory_metrics, "collect_at", return_value={}))
            stack.enter_context(patch.object(run.latency_metrics, "collect", return_value={}))
            stack.enter_context(patch.object(run, "display_size", return_value=(960, 2142)))
            def replay(*args):
                events.append(("journey", "target"))
                return []
            stack.enter_context(patch.object(run, "replay_journey", side_effect=replay))
            def compare(args, **kwargs):
                events.append((args[1], "image"))
                return subprocess.CompletedProcess(args, 0, stdout='{"visual_review":"required"}')
            stack.enter_context(patch.object(run.subprocess, "run", side_effect=compare))
            result = run.launch_and_capture(
                "device", "list", {"mode": "verify", "env": {}}, apk,
                {"settle_timeout_seconds": 30, "env": {}}, root, "hwui", [],
                "compare-screenshots", "reference",
            )
            self.assertLess(events.index(("stop", "reference")), events.index(("start", "target")))
            self.assertLess(events.index(("start", "target")), events.index(("journey", "target")))
            last_stop = max(i for i, event in enumerate(events) if event == ("stop", "target"))
            self.assertLess(last_stop, events.index(("compare", "image")))
            self.assertEqual(result["verify"], "review_required")


if __name__ == "__main__":
    unittest.main()
