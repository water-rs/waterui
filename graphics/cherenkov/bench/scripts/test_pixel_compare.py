#!/usr/bin/env python3
"""`pixel_compare.py analyze`'s pinned-mode verdict, on the intervals of real
Pixel collections and on synthetic collections."""
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


def collection(out, scenes, rounds=4, frames=40, cycles=None):
    """Writes a completed pinned collection. `scenes` maps a name to its
    per-invocation GPU milliseconds `(base, head)`, one pair per round; each
    invocation's frames scatter ±1% around its value. With `cycles`, a
    `--gpu-counters` collection whose shader-core cycles per invocation
    `cycles` gives the same way."""
    rng = random.Random(0)
    save = pixel_compare.save
    save(out / pixel_compare.PLAN, {
        'arms': {}, 'rounds': rounds, 'frames': frames, 'warmup': 10,
        'clocks': {'gpu_khz': 649000, 'cpu_khz': 2294000},
        'metric': 'gpu-time' if cycles is None else 'shader-core-cycles',
        'scenes': [{'name': name} for name in scenes]})
    save(out / pixel_compare.CONTROL, {'completed': True})
    rows = []
    for name, pairs in scenes.items():
        assert len(pairs) == rounds
        for pair, values in enumerate(pairs):
            for arm, (side, milliseconds) in enumerate(zip(('base', 'head'), values)):
                run_id = f'{name}-p{pair}-{side}'
                report = out / f'{run_id}.json'
                samples = [{'gpu_seconds': milliseconds * rng.uniform(0.99, 1.01) / 1000}
                           for _ in range(frames)]
                if cycles is not None:
                    for sample in samples:
                        sample['gpu_counters'] = {'shader_core_active_cycles': round(
                            cycles[name][pair][arm] * rng.uniform(0.99, 1.01))}
                save(report, {'warmup_frames': 10, 'samples': samples})
                rows.append({'scene': name, 'pair': pair, 'side': side, 'run_id': run_id,
                             'report_sha256': hashlib.sha256(report.read_bytes()).hexdigest(),
                             'failures': []})
    (out / pixel_compare.RUNS).write_text(''.join(json.dumps(r) + '\n' for r in rows))


class Analyze(unittest.TestCase):
    def analyze(self, scenes, cycles=None):
        with tempfile.TemporaryDirectory() as dir:
            out = pathlib.Path(dir)
            collection(out, scenes, cycles=cycles)
            return pixel_compare.analyze(out, LIMIT, 1000 / pixel_compare.RATE_HZ)['scenes']

    def test_a_large_cost_on_a_scene_that_varies_per_process_fails(self):
        # Per-process spread of ±25% leaves the interval far wider than the
        # limit, yet wholly above it.
        spread = [1.0, 1.25, 0.8, 1.1]
        entry = self.analyze({'tiny': [(0.1 * s, 0.16 * t) for s, t in
                                       zip(spread, reversed(spread))]})['tiny']
        low, high = entry['interval_percent']
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
        self.assertEqual((entry['metric'], entry['unit']), ('shader-core-cycles', 'cycles'))
        self.assertAlmostEqual(entry['change_percent'], 5.0, delta=0.5)
        self.assertEqual(entry['verdict'], 'fail')
        self.assertEqual(self.analyze(time)['tiny']['verdict'], 'pass')


if __name__ == '__main__':
    unittest.main()
