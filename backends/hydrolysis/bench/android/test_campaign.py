"""A campaign cannot silently change the Compose build under measurement."""

import argparse
import io
import json
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


if __name__ == "__main__":
    unittest.main()
