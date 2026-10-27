#!/usr/bin/env python3
"""`pixel_compare.py analyze`: the GPU engine's pinned-mode verdict on the
intervals of real Pixel collections, and synthetic GPU-engine and CPU-engine
collections — a CPU collection is reported, never decided."""
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

    def test_a_cpu_collection_reports_frame_and_shade_without_a_verdict(self):
        # Shade grows from 50% to 55% of a frame that grows 5%: shade +15.5%.
        entry = self.analyze({'map': [(40.0, 42.0)] * 4},
                             cpu={'map': [(0.5, 0.55)] * 4})['map']
        self.assertEqual(list(entry['metrics']), ['frame', 'shade'])
        self.assertNotIn('verdict', entry)
        frame, shade = self.metric(entry, 'frame'), self.metric(entry, 'shade')
        for judged in (frame, shade):
            self.assertNotIn('verdict', judged)
            self.assertNotIn('interval_percent', judged)
        self.assertAlmostEqual(frame['p50']['base'], 40.0, delta=0.5)
        self.assertAlmostEqual(frame['p50']['head'], 42.0, delta=0.5)
        self.assertAlmostEqual(frame['change_percent'], 5.0, delta=0.5)
        self.assertAlmostEqual(shade['change_percent'], 15.5, delta=0.5)

    def test_a_cost_outside_shade_shows_in_the_frame_alone(self):
        entry = self.analyze({'chart': [(20.0, 21.0)] * 4},
                             cpu={'chart': [(0.5, 0.5 * 20 / 21)] * 4})['chart']
        self.assertAlmostEqual(self.metric(entry, 'frame')['change_percent'],
                               5.0, delta=0.5)
        self.assertAlmostEqual(self.metric(entry, 'shade')['change_percent'],
                               0.0, delta=0.5)

    def test_identical_cpu_arms_report_a_zero_change_with_their_p99(self):
        entry = self.analyze({'text-page': [(5.0, 5.0)] * 4},
                             cpu={'text-page': [(0.8, 0.8)] * 4})['text-page']
        for metric, judged in entry['metrics'].items():
            expected = 5.0 if metric == 'frame' else 4.0
            self.assertAlmostEqual(judged['change_percent'], 0.0, delta=0.5)
            self.assertAlmostEqual(judged['p99_change_percent'], 0.0, delta=0.5)
            for side in ('base', 'head'):
                self.assertAlmostEqual(judged['p50'][side], expected, delta=0.1)
                self.assertGreater(judged['p99'][side], judged['p50'][side])

    def test_the_phase_gap_is_what_submit_holds_beyond_the_phases(self):
        entry = self.analyze({'ui-list': [(10.0, 10.0)] * 4},
                             cpu={'ui-list': [(0.6, 0.6)] * 4})['ui-list']
        for side in ('base', 'head'):
            p50, p99, worst = entry['phase_gap'][side]['ms']
            for value in (p50, p99, worst):
                self.assertAlmostEqual(value, 0.1, delta=1e-9)

    def test_a_frame_without_the_shade_phase_is_an_error(self):
        sample = cpu_sample(10.0, 0.5)
        sample['phases'] = sample['phases'][:2]
        with self.assertRaises(SystemExit):
            pixel_compare.METRICS['shade'][1](sample)


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