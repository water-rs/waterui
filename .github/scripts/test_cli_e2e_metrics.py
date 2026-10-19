"""Tests for `cli_e2e_metrics.py`: leg records, the step summary, history."""

import csv
import io
import json
from datetime import datetime, timezone

import cli_e2e_metrics as m

NOW = datetime(2026, 10, 18, 9, 0, tzinfo=timezone.utc)


def record(platform, date, run_id, **metrics):
    return {
        "date": date,
        "recorded_at": f"{date}T09:00:00Z",
        "run_id": str(run_id),
        "sha": "0123456789abcdef",
        "platform": platform,
        "runner": "ubuntu-latest",
        "image": "ubuntu24 20261012.1",
        "rustc": "rustc 1.99.1 (5f94df478 2026-08-27)",
        "status": "success",
        "cold_verified": True,
        "metrics": metrics,
    }


def test_timings_keep_the_last_value_and_skip_noise():
    text = "cold_build\t100\n\nbogus\ncreate\t7\ncold_build\t120\nlaunch\tNaN?\n"
    assert m.parse_timings(text) == {"cold_build": 120, "create": 7}


def test_rustc_line_comes_from_verbose_version():
    text = "rustc 1.99.1 (5f94df478 2026-08-27)\nbinary: rustc\nhost: x86_64\n"
    assert m.parse_rustc(text) == "rustc 1.99.1 (5f94df478 2026-08-27)"
    assert m.parse_rustc("error: no toolchain") is None


def test_record_reads_the_leg_files_and_the_runner_environment(tmp_path):
    (tmp_path / "t.tsv").write_text("cli_install\t300\ncold_build\t900\n")
    (tmp_path / "f.txt").write_text("cli_install_cold=false\ncold_build_verified=true\n")
    (tmp_path / "r.log").write_text("rustc 1.99.1 (abc 2026-08-27)\n")
    args = m.argparse.Namespace(
        platform="linux",
        runner="ubuntu-latest",
        status="success",
        timings=str(tmp_path / "t.tsv"),
        rustc=str(tmp_path / "r.log"),
        flags=str(tmp_path / "f.txt"),
    )
    env = {
        "ImageOS": "ubuntu24",
        "ImageVersion": "20261012.1",
        "GITHUB_SHA": "deadbeef",
        "GITHUB_RUN_ID": "42",
        "GITHUB_EVENT_NAME": "schedule",
    }
    out = m.build_record(args, env, NOW)
    assert out["date"] == "2026-10-18"
    assert out["image"] == "ubuntu24 20261012.1"
    assert out["rustc"] == "rustc 1.99.1 (abc 2026-08-27)"
    assert out["cold_verified"] is True
    assert out["cli_install_cold"] is False
    assert out["metrics"] == {"cli_install": 300, "cold_build": 900}


def test_record_of_a_leg_that_timed_nothing_has_empty_metrics(tmp_path):
    args = m.argparse.Namespace(
        platform="android", runner="", status="failure",
        timings=str(tmp_path / "missing.tsv"), rustc=None, flags=None,
    )
    out = m.build_record(args, {}, NOW)
    assert out["metrics"] == {}
    assert out["cold_verified"] is None
    assert out["rustc"] is None


def test_summary_shows_delta_and_medians_against_earlier_nights():
    history = [
        record("linux", "2026-10-10", 1, cold_build=1000),
        record("linux", "2026-10-15", 2, cold_build=1100),
        record("linux", "2026-10-17", 3, cold_build=1200),
        record("macos", "2026-10-17", 3, cold_build=5000),
    ]
    tonight = [record("linux", "2026-10-18", 4, cold_build=1260, launch=9)]
    text = m.render_summary(tonight, history, history_url="H", now=NOW)
    row = next(line for line in text.splitlines() if line.startswith("| Cold app build"))
    # Previous night 1200 → +60s (+5.0%); 7-night median of 1100,1200; the
    # 30-night median also counts 1000. The macOS night never leaks in.
    assert "21m 00s" in row
    assert "+1m 00s (+5.0%)" in row
    assert "19m 10s (n=2)" in row
    assert "18m 20s (n=3)" in row
    launch = next(line for line in text.splitlines() if line.startswith("| Launch"))
    assert "| 9s | — | — | — |" in launch
    assert "### Ubuntu (Linux)" in text
    assert "### macOS" not in text
    assert "](H)" in text


def test_summary_flags_a_warm_build_and_a_leg_without_phases():
    warm = record("windows", "2026-10-18", 4, cold_build=10)
    warm["cold_verified"] = False
    empty = record("android", "2026-10-18", 4)
    text = m.render_summary([empty, warm], [], now=NOW)
    assert "not a cold number tonight" in text
    assert "_No phase completed._" in text
    assert text.index("### Windows") < text.index("### Android")


def test_summary_without_records_says_so():
    assert "No `cli-e2e` leg uploaded" in m.render_summary([], [], now=NOW)


def test_rerun_replaces_the_same_nights_records():
    history = [record("linux", "2026-10-17", 3, cold_build=1), record("linux", "2026-10-18", 4, cold_build=2)]
    merged = m.merge_history(history, [record("linux", "2026-10-18", 4, cold_build=3)])
    assert [r["metrics"]["cold_build"] for r in merged] == [1, 3]


def test_summarize_writes_jsonl_csv_and_readme(tmp_path, capsys):
    records = tmp_path / "records"
    records.mkdir()
    (records / "linux.json").write_text(json.dumps(record("linux", "2026-10-18", 4, cold_build=5, package=6)))
    (records / "junk.json").write_text("{not json")
    history = tmp_path / "history.jsonl"
    history.write_text(json.dumps(record("linux", "2026-10-17", 3, cold_build=4)) + "\nnot json\n")
    out = tmp_path / "out"
    assert m.main(["summarize", "--records", str(records), "--history", str(history), "--out-dir", str(out)]) == 0
    assert "Cold app build" in capsys.readouterr().out
    lines = (out / "history.jsonl").read_text().splitlines()
    assert [json.loads(line)["run_id"] for line in lines] == ["3", "4"]
    rows = list(csv.DictReader(io.StringIO((out / "history.csv").read_text())))
    assert rows[1]["cold_build"] == "5" and rows[1]["package"] == "6" and rows[1]["launch"] == ""
    assert rows[1]["cold_verified"] == "True"
    assert (out / "README.md").read_text().startswith("# Nightly CLI cold-compile metrics")


def test_missing_history_file_is_an_empty_history(tmp_path):
    assert m.load_history(str(tmp_path / "nope.jsonl")) == []
