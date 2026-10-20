#!/usr/bin/env python3
"""`pixel_compare.py analyze`'s pinned-mode verdict, on the intervals of real
Pixel collections and on synthetic GPU-engine and CPU-engine collections."""
import hashlib
import importlib.util
import json
import pathlib
import random
import tempfile
import unittest

SOURCE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('pixel_compare', SOURCE / 'pixel_compare.py')
pixel_compare = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(pixel_compare)

LIMIT = 2.0


def decide(change, low, high):
    return pixel_compare.decide(change, low, high, LIMIT)


class WhollyAboveTheLimit(unittest.TestCase):
    """Collections from #2775 comparing a build with itself plus a known
    extra cost, whose intervals are wider than the limit (#2844)."""

    def test_lame_evals_6_to_128_on_continuous_s50_fails(self):
        self.assertEqual(decide(115.36, 113.71, 116.73), 'fail')

    def test_buffer_clear_over_4_rounds_fails(self):
        self.assertEqual(decide(4.59, 3.36, 5.74), 'fail')

    def test_buffer_clear_over_16_rounds_fails(self):
        self.assertEqual(decide(4.83, 3.06, 5.75), 'fail')

    def test_an_interval_touching_the_limit_is_not_wholly_above_it(self):
        self.assertEqual(decide(3.5, LIMIT, 5.0), 'unresolved')

    def test_an_interval_just_past_the_limit_is(self):
        self.assertEqual(decide(3.5, 2.001, 5.0), 'fail')


class TheWidthRuleStands(unittest.TestCase):
    def test_a_wide_interval_across_the_limit_is_unresolved(self):
        self.assertEqual(decide(1.0, -1.0, 3.1), 'unresolved')

    def test_a_narrow_interval_passes_at_or_below_the_limit(self):
        self.assertEqual(decide(0.2, -0.3, 0.7), 'pass')
        self.assertEqual(decide(LIMIT, 1.5, 2.5), 'pass')

    def test_a_narrow_interval_fails_above_the_limit(self):
        self.assertEqual(decide(2.4, 1.6, 3.2), 'fail')

    def test_a_half_width_of_exactly_half_the_limit_is_decided(self):
        self.assertEqual(decide(0.0, -1.0, 1.0), 'pass')

    def test_a_wide_interval_wholly_below_the_negative_limit_is_unresolved(self):
        # The limit bounds a cost increase: a decrease never fails, and a
        # pass needs the same width as any other.
        self.assertEqual(decide(-10.0, -12.0, -8.5), 'unresolved')

    def test_a_narrow_interval_below_the_negative_limit_passes(self):
        self.assertEqual(decide(-10.0, -10.5, -9.5), 'pass')


def collection(out, scenes, rounds=4, frames=40, cycles=None, cpu=None):
    """Writes a completed pinned collection. `scenes` maps a name to its
    per-invocation GPU milliseconds `(base, head)`, one pair per round; each
    invocation's frames scatter ±1% around its value. With `cycles`, a
    `--gpu-counters` collection whose shader-core cycles per invocation
    `cycles` gives the same way. With `cpu`, a cherenkov-cpu collection: the
    values are frame milliseconds, `cpu` maps each scene to the share of the
    frame its `shade` phase takes per invocation `(base, head)`, and every
    frame splits into a 10% encode and a submit of which 0.1 ms lies outside
    the phases."""
    rng = random.Random(0)
    save = pixel_compare.save
    if cpu is not None:
        engine, metrics = 'cherenkov-cpu', ['frame', 'shade']
    else:
        engine = 'cherenkov'
        metrics = ['gpu-time' if cycles is None else 'shader-core-cycles']
    save(out / pixel_compare.PLAN, {
        'arms': {}, 'rounds': rounds, 'frames': frames, 'warmup': 10,
        'clocks': {'gpu': 649000, 'policy7': 2294000}, 'engine': engine, 'metrics': metrics,
        'scenes': [{'name': name} for name in scenes]})
    save(out / pixel_compare.CONTROL, {'completed': True})
    rows = []
    for name, pairs in scenes.items():
        assert len(pairs) == rounds
        for pair, values in enumerate(pairs):
            for arm, (side, milliseconds) in enumerate(zip(('base', 'head'), values)):
                run_id = f'{name}-p{pair}-{side}'
                report = out / f'{run_id}.json'
                if cpu is None:
                    samples = [{'gpu_seconds': milliseconds * rng.uniform(0.99, 1.01) / 1000}
                               for _ in range(frames)]
                else:
                    samples = [cpu_sample(milliseconds * rng.uniform(0.99, 1.01),
                                          cpu[name][pair][arm]) for _ in range(frames)]
                if cycles is not None:
                    for sample in samples:
                        sample['gpu_counters'] = {'shader_core_active_cycles': round(
                            cycles[name][pair][arm] * rng.uniform(0.99, 1.01))}
                save(report, {'warmup_frames': 10, 'samples': samples})
                rows.append({'scene': name, 'pair': pair, 'side': side, 'run_id': run_id,
                             'report_sha256': hashlib.sha256(report.read_bytes()).hexdigest(),
                             'quiet': {'other_ms_per_s': 100.0, 'top': []}, 'rejected': False,
                             'failures': []})
    (out / pixel_compare.RUNS).write_text(''.join(json.dumps(r) + '\n' for r in rows))


def cpu_sample(frame_ms, shade_share):
    """A cherenkov-cpu frame of `frame_ms`: a 10% encode, and a submit whose
    phases leave 0.1 ms, with `shade_share` of the frame in `shade` and the
    rest of the phases split evenly between `lower` and `glyph`."""
    encode = frame_ms * 0.1
    submit = frame_ms - encode
    shade = frame_ms * shade_share
    rest = (submit - 0.1 - shade) / 2
    return {'encode_seconds': encode / 1000, 'submit_seconds': submit / 1000,
            'phases': [{'name': 'lower', 'seconds': rest / 1000},
                       {'name': 'glyph', 'seconds': rest / 1000},
                       {'name': 'shade', 'seconds': shade / 1000}]}


class Analyze(unittest.TestCase):
    def analyze(self, scenes, cycles=None, cpu=None):
        with tempfile.TemporaryDirectory() as dir:
            out = pathlib.Path(dir)
            collection(out, scenes, cycles=cycles, cpu=cpu)
            return pixel_compare.analyze(out, LIMIT, 1000 / pixel_compare.RATE_HZ)['scenes']

    def metric(self, entry, name):
        return entry['metrics'][name]

    def test_a_large_cost_on_a_scene_that_varies_per_process_fails(self):
        # Per-process spread of ±25% leaves the interval far wider than the
        # limit, yet wholly above it.
        spread = [1.0, 1.25, 0.8, 1.1]
        entry = self.analyze({'tiny': [(0.1 * s, 0.16 * t) for s, t in
                                       zip(spread, reversed(spread))]})['tiny']
        low, high = self.metric(entry, 'gpu-time')['interval_percent']
        self.assertGreater(high - low, LIMIT)
        self.assertGreater(low, LIMIT)
        self.assertEqual(entry['verdict'], 'fail')

    def test_identical_arms_with_the_same_spread_stay_unresolved(self):
        spread = [1.0, 1.25, 0.8, 1.1]
        entry = self.analyze({'tiny': [(0.1 * s, 0.1 * t) for s, t in
                                       zip(spread, reversed(spread))]})['tiny']
        self.assertEqual(entry['verdict'], 'unresolved')

    def test_identical_stable_arms_pass(self):
        entry = self.analyze({'full': [(4.0, 4.0)] * 4})['full']
        self.assertEqual(entry['verdict'], 'pass')

    def test_a_counter_collection_is_judged_on_cycles_not_time(self):
        # Added work that GPU time reads as a speed-up and the shader-core
        # cycles read as a 5% cost (#2775).
        time = {'tiny': [(0.100, 0.097)] * 4}
        entry = self.analyze(time, cycles={'tiny': [(140_000, 147_000)] * 4})['tiny']
        self.assertEqual(list(entry['metrics']), ['shader-core-cycles'])
        judged = self.metric(entry, 'shader-core-cycles')
        self.assertEqual(judged['unit'], 'cycles')
        self.assertAlmostEqual(judged['change_percent'], 5.0, delta=0.5)
        self.assertEqual(entry['verdict'], 'fail')
        self.assertEqual(self.analyze(time)['tiny']['verdict'], 'pass')

    def test_a_cpu_collection_is_judged_on_frame_and_shade(self):
        # Shade grows from 50% to 55% of a frame that grows 5%: shade +15.5%.
        entry = self.analyze({'map': [(40.0, 42.0)] * 4},
                             cpu={'map': [(0.5, 0.55)] * 4})['map']
        self.assertEqual(list(entry['metrics']), ['frame', 'shade'])
        frame, shade = self.metric(entry, 'frame'), self.metric(entry, 'shade')
        self.assertAlmostEqual(frame['change_percent'], 5.0, delta=0.5)
        self.assertAlmostEqual(shade['change_percent'], 15.5, delta=0.5)
        self.assertEqual((frame['verdict'], shade['verdict'], entry['verdict']),
                         ('fail', 'fail', 'fail'))

    def test_a_cost_outside_shade_fails_the_frame_alone(self):
        entry = self.analyze({'chart': [(20.0, 21.0)] * 4},
                             cpu={'chart': [(0.5, 0.5 * 20 / 21)] * 4})['chart']
        self.assertEqual(self.metric(entry, 'frame')['verdict'], 'fail')
        self.assertEqual(self.metric(entry, 'shade')['verdict'], 'pass')

    def test_identical_cpu_arms_pass_with_their_p99_reported(self):
        entry = self.analyze({'text-page': [(5.0, 5.0)] * 4},
                             cpu={'text-page': [(0.8, 0.8)] * 4})['text-page']
        self.assertEqual(entry['verdict'], 'pass')
        for judged in entry['metrics'].values():
            self.assertEqual(judged['verdict'], 'pass')
            low, high = judged['p99_interval_percent']
            self.assertLessEqual(low, 0.0)
            self.assertGreaterEqual(high, 0.0)

    def test_the_phase_gap_is_what_submit_holds_beyond_the_phases(self):
        entry = self.analyze({'ui-list': [(10.0, 10.0)] * 4},
                             cpu={'ui-list': [(0.6, 0.6)] * 4})['ui-list']
        for side in ('base', 'head'):
            p50, p99, worst = entry['phase_gap'][side]['ms']
            for value in (p50, p99, worst):
                self.assertAlmostEqual(value, 0.1, delta=1e-9)

    def test_a_rejected_run_is_reported_and_left_out_of_the_decision(self):
        # The rejected first attempt of p0-head ran 30% slow; its rerun is
        # the slot's run, so identical arms still pass.
        with tempfile.TemporaryDirectory() as dir:
            out = pathlib.Path(dir)
            collection(out, {'full': [(4.0, 4.0)] * 4})
            slow = out / 'full-p0-head.json'
            rerun = out / 'full-p0-head-r1.json'
            slow.rename(rerun)
            pixel_compare.save(slow, {'warmup_frames': 10,
                                      'samples': [{'gpu_seconds': 0.0052}] * 40})
            rows = [json.loads(line) for line in (out / pixel_compare.RUNS).read_text().splitlines()]
            for row in rows:
                if row['run_id'] == 'full-p0-head':
                    row['run_id'] = 'full-p0-head-r1'
            rows.insert(0, {'scene': 'full', 'pair': 0, 'side': 'head', 'run_id': 'full-p0-head',
                            'report_sha256': hashlib.sha256(slow.read_bytes()).hexdigest(),
                            'quiet': {'other_ms_per_s': 400.0, 'top': [['dex2oat64', 812, 3.1]]},
                            'rejected': True, 'failures': []})
            (out / pixel_compare.RUNS).write_text(''.join(json.dumps(r) + '\n' for r in rows))
            entry = pixel_compare.analyze(out, LIMIT, 1000 / pixel_compare.RATE_HZ)['scenes']['full']
        self.assertEqual(entry['verdict'], 'pass')
        self.assertEqual(entry['runs'], 8)
        self.assertEqual([r['run_id'] for r in entry['rejected_runs']], ['full-p0-head'])
        self.assertEqual(entry['rejected_runs'][0]['other_ms_per_s'], 400.0)
        self.assertNotIn('full-p0-head', entry['other_ms_per_s'])

    def test_a_frame_without_the_shade_phase_is_an_error(self):
        sample = cpu_sample(10.0, 0.5)
        sample['phases'] = sample['phases'][:2]
        with self.assertRaises(SystemExit):
            pixel_compare.METRICS['shade'][1](sample)


def stat(pid, comm, ppid, own, children=0, start=1000):
    """A `/proc/PID/stat` line: `own` ticks as utime (with stime 0) and
    `children` as cutime (with cstime 0)."""
    return (f'{pid} ({comm}) S {ppid} {pid} {pid} 0 -1 4194560 0 0 0 0 {own} 0 {children} 0 '
            f'20 0 1 0 {start} 0 0 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 7 0 0 0 0 0\n')


def proc_stat(uptime):
    """`cat /proc/uptime /proc/stat` on a one-CPU device."""
    return f'{uptime} 1.00\ncpu  0 0 0 0 0 0 0 0 0 0\ncpu0 0 0 0 0 0 0 0 0 0 0\nintr 0\n'


class OutsideCpu(unittest.TestCase):
    """The quiet gate's accounting of what ran beside the workload. The
    supervisor is pid 50; its workload, its `sleep` and its snapshot `cat`
    are its children."""

    def measure(self, before, after, seconds=10.0):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp)
            (path / 'supervisor-pid').write_text('50\n')
            (path / 'processes-before').write_text(''.join(before))
            (path / 'processes-after').write_text(''.join(after))
            (path / 'system-before').write_text(proc_stat(100.0))
            (path / 'system-after').write_text(proc_stat(100.0 + seconds))
            return pixel_compare.outside_cpu(path, 100)

    def test_other_processes_count_and_the_workload_does_not(self):
        quiet = self.measure(
            [stat(1, 'init', 0, 500, 900), stat(50, 'sh', 40, 10, 5), stat(51, 'cat', 50, 1),
             stat(300, 'system_server', 1, 7000)],
            # The workload's 900 ticks reached the supervisor's children
            # counter; it polled for 20 ticks itself. A `sleep` child shows.
            [stat(1, 'init', 0, 500, 900), stat(50, 'sh', 40, 30, 905),
             stat(60, 'sleep', 50, 0), stat(300, 'system_server', 1, 7080)])
        self.assertAlmostEqual(quiet['workload_cpu_seconds'], 9.0)
        self.assertAlmostEqual(quiet['other_cpu_seconds'], 1.0)
        self.assertAlmostEqual(quiet['other_ms_per_s'], 100.0)
        self.assertEqual([name for name, *_ in quiet['top']], ['system_server', 'sh'])

    def test_a_process_born_in_the_window_counts_whole(self):
        quiet = self.measure([stat(50, 'sh', 40, 0)],
                             [stat(50, 'sh', 40, 0), stat(70, 'dex2oat64', 1, 300, 20)])
        self.assertAlmostEqual(quiet['other_cpu_seconds'], 3.2)
        self.assertEqual(quiet['top'], [['dex2oat64', 70, 3.2]])

    def test_an_exit_counts_only_its_time_inside_the_window(self):
        # 80 exits after 30 ticks in the window; its 1000-tick lifetime
        # reaches 1's children counter.
        quiet = self.measure(
            [stat(1, 'init', 0, 0, 0), stat(50, 'sh', 40, 0), stat(80, 'logd', 1, 970)],
            [stat(1, 'init', 0, 0, 1000), stat(50, 'sh', 40, 0)])
        self.assertAlmostEqual(quiet['other_cpu_seconds'], 0.3)

    def test_an_exit_no_parent_waits_for_takes_nothing_back(self):
        # A kworker exits: kthreadd never accumulates it, so the exit's time
        # inside the window is unknown and nothing goes negative.
        quiet = self.measure(
            [stat(2, 'kthreadd', 0, 0, 0), stat(50, 'sh', 40, 0),
             stat(81, 'kworker/7:0-events', 2, 67), stat(300, 'system_server', 1, 10)],
            [stat(2, 'kthreadd', 0, 0, 0), stat(50, 'sh', 40, 0),
             stat(300, 'system_server', 1, 30)])
        self.assertAlmostEqual(quiet['other_cpu_seconds'], 0.2)

    def test_an_exit_under_an_exited_parent_reaches_the_living_ancestor(self):
        # 82 forks 83; both exit in the window and init reaps both lifetimes.
        quiet = self.measure(
            [stat(1, 'init', 0, 0, 0), stat(50, 'sh', 40, 0),
             stat(82, 'dumpstate', 1, 100), stat(83, 'dumpsys', 82, 40)],
            [stat(1, 'init', 0, 0, 100 + 10 + 40 + 5), stat(50, 'sh', 40, 0)])
        self.assertAlmostEqual(quiet['other_cpu_seconds'], 0.15)

    def test_a_reused_pid_is_a_different_process(self):
        quiet = self.measure(
            [stat(1, 'init', 0, 0, 0), stat(50, 'sh', 40, 0), stat(90, 'old', 1, 400, start=10)],
            [stat(1, 'init', 0, 0, 400), stat(50, 'sh', 40, 0), stat(90, 'new', 1, 50, start=20)])
        self.assertAlmostEqual(quiet['other_cpu_seconds'], 0.5)

    def test_a_name_with_spaces_and_parentheses_parses(self):
        procs = pixel_compare.processes(stat(7, 'kworker/u16:2 (x) y', 2, 42, 3))
        self.assertEqual(procs[7, 1000],
                         pixel_compare.Process(7, 2, 'kworker/u16:2 (x) y', 42, 3))


class CpuList(unittest.TestCase):
    def test_ranges_and_single_cpus_expand_sorted(self):
        self.assertEqual(pixel_compare.cpu_list('7'), [7])
        self.assertEqual(pixel_compare.cpu_list('4-6,0'), [0, 4, 5, 6])
        self.assertEqual(pixel_compare.cpu_list('0-7'), list(range(8)))


class CpuEngines(unittest.TestCase):
    def test_each_engine_states_the_worker_count_it_was_given(self):
        engines = pixel_compare.CPU_ENGINES
        self.assertEqual(engines['cherenkov-cpu'].environment(8), {'RAYON_NUM_THREADS': '8'})
        self.assertEqual(engines['cherenkov-cpu'].adapter(8), 'cherenkov-cpu (8 threads)')
        self.assertEqual(engines['vello-cpu'].environment(1), {'VELLO_CPU_THREADS': '0'})
        self.assertEqual(engines['vello-cpu'].adapter(1), 'vello-cpu (single-threaded dispatcher)')
        self.assertEqual(engines['vello-cpu'].environment(8), {'VELLO_CPU_THREADS': '8'})
        self.assertEqual(engines['vello-cpu'].adapter(8), 'vello-cpu (8 threads)')
        self.assertEqual(engines['skia-cpu'].max_workers, 1)


class DeviceControlEvidence(unittest.TestCase):
    def evidence(self, directory, restored="10", applied="20", exit_code=0):
        files = {"registered-controls": "1", "exit-code": str(exit_code),
                 "control-0.kind": "node", "control-0.key": "/sys/test-frequency",
                 "control-0.original": "10", "control-0.requested": "20",
                 "control-0.restored": restored}
        if applied is not None:
            files["control-0.applied"] = applied
        for name, value in files.items():
            (directory / name).write_text(value + "\n")

    def test_restoration_mismatch_is_rejected_even_on_zero_exit(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp)
            self.evidence(path, restored="20")
            with self.assertRaisesRegex(pixel_compare.ControlError, "not restored"):
                pixel_compare.PerformanceControls.read_evidence(path)

    def test_success_requires_requested_setting_to_be_applied(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp)
            self.evidence(path, applied="10")
            with self.assertRaisesRegex(pixel_compare.ControlError, "not applied"):
                pixel_compare.PerformanceControls.read_evidence(path)

    def test_preflight_temperature_abort_preserves_unapplied_state(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp)
            self.evidence(path, applied=None, exit_code=125)
            result = pixel_compare.PerformanceControls.read_evidence(path)
            self.assertEqual(result["exit"], 125)
            self.assertIsNone(result["settings"][0]["applied"])


if __name__ == '__main__':
    unittest.main()
