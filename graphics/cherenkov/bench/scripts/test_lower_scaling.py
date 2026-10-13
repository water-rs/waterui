#!/usr/bin/env python3
"""The scaling report's fit and verdict over synthetic `ir_gate` results."""
import argparse
import importlib.util
import json
import pathlib
import tempfile
import unittest

SOURCE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('lower_scaling', SOURCE / 'lower_scaling.py')
lower_scaling = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(lower_scaling)


def results(out, tag, costs):
    """Writes one `ir_gate` result per size with `lower` Rust Ir from `costs`."""
    for (scene, segments), cost in zip(lower_scaling.SIZES, costs):
        (out / f'{tag}-{scene}.result.json').write_text(json.dumps({
            'tag': tag, 'scene': scene, 'sha256': '0' * 64,
            'lower': {'second': {'ir': cost + 1_000, 'rust_ir': cost}},
        }))
        assert segments > 0


def run(out, tag, exponent, gate):
    args = argparse.Namespace(out=str(out), tag=tag, exponent=exponent, gate=gate)
    return lower_scaling.report(args)


class Fit(unittest.TestCase):
    def test_exact_powers_recover_their_exponent(self):
        for exponent in (1.0, 1.25, 2.0):
            points = [(x, x ** exponent) for x, _ in ((1_000, 0), (2_000, 0), (8_000, 0), (64_000, 0))]
            self.assertAlmostEqual(lower_scaling.fit(points), exponent, places=9)

    def test_local_exponents_are_per_pair(self):
        points = [(100, 100), (200, 400), (400, 800)]
        self.assertEqual([(a, b, round(e, 6)) for a, b, e in lower_scaling.local(points)],
                         [(100, 200, 2.0), (200, 400, 1.0)])

    def test_a_constant_cost_cannot_be_fitted_without_two_points(self):
        with self.assertRaises(AssertionError):
            lower_scaling.fit([(1_000, 1_000)])


class Report(unittest.TestCase):
    def test_linear_growth_is_within_the_bound_and_gates_pass(self):
        with tempfile.TemporaryDirectory() as dir:
            out = pathlib.Path(dir)
            results(out, 'gpu', [40 * segments for _, segments in lower_scaling.SIZES])
            self.assertEqual(run(out, 'gpu', lower_scaling.EXPONENT, gate=True), 0)
            report = (out / 'gpu-lower-scaling.md').read_text()
            self.assertIn('Gate: pass.', report)
            self.assertIn('**1.000**', report)

    def test_superlinear_growth_fails_the_gate(self):
        with tempfile.TemporaryDirectory() as dir:
            out = pathlib.Path(dir)
            results(out, 'cpu', [int(segments ** 1.4) for _, segments in lower_scaling.SIZES])
            self.assertEqual(run(out, 'cpu', lower_scaling.EXPONENT, gate=True), 1)
            self.assertIn('Gate: FAIL.', (out / 'cpu-lower-scaling.md').read_text())

    def test_without_the_gate_a_failing_exponent_still_exits_zero(self):
        with tempfile.TemporaryDirectory() as dir:
            out = pathlib.Path(dir)
            results(out, 'cpu', [int(segments ** 1.4) for _, segments in lower_scaling.SIZES])
            self.assertEqual(run(out, 'cpu', lower_scaling.EXPONENT, gate=False), 0)
            report = (out / 'cpu-lower-scaling.md').read_text()
            self.assertIn('above the bound', report)
            self.assertIn('Report only', report)
            self.assertNotIn('Gate:', report)

    def test_a_missing_size_is_an_error_rather_than_a_shorter_fit(self):
        with tempfile.TemporaryDirectory() as dir:
            out = pathlib.Path(dir)
            results(out, 'cpu', [40 * segments for _, segments in lower_scaling.SIZES])
            (out / f'cpu-{lower_scaling.SIZES[2][0]}.result.json').unlink()
            with self.assertRaises(FileNotFoundError):
                run(out, 'cpu', lower_scaling.EXPONENT, gate=False)


if __name__ == '__main__':
    unittest.main()
