#!/usr/bin/env python3
"""The Ir gate's sampling over hand-made Callgrind dumps (testdata/ir_gate)."""
import importlib.util
import pathlib
import unittest
from unittest import mock

SOURCE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('ir_gate', SOURCE / 'ir_gate.py')
ir_gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ir_gate)

DATA = SOURCE / 'testdata' / 'ir_gate'
ROOT = '_RNvCs1_5bench4root'


class Sampling(unittest.TestCase):
    def test_span_subtracts_only_calls_under_the_root(self):
        # run.1 closes the call in flight when instrumentation switched on,
        # run.2 enters the root, run.3 and run.4 are cut by a nested root,
        # run.5 returns. Another thread frees through the same deallocate
        # node in run.3, and code outside the root frees through it in run.2.
        [sample] = ir_gate.call_samples(DATA, 'run', ROOT, before=[2], after=[1, 5])
        self.assertEqual(sample['ir'], 50 + 40 + 30)
        self.assertEqual(sample['ncalls'], 1)
        # free under drop (30) and __rust_dealloc (20); the free inside
        # __rust_dealloc is part of that call.
        self.assertEqual(sample['alloc_ir'], 50)
        self.assertEqual((sample['allocs'], sample['reallocs'], sample['deallocs']), (0, 0, 2))
        self.assertEqual((sample['mem_ir'], sample['mem_calls']), (12, 2))
        self.assertEqual(sample['rust_ir'], 120 - 50 - 12)

    def test_later_span_without_one_entry_fails(self):
        with self.assertRaises(AssertionError):
            ir_gate.call_samples(DATA, 'run', ROOT, before=[], after=[1, 5])
        with self.assertRaises(AssertionError):
            ir_gate.call_samples(DATA, 'run', ROOT, before=[2, 3], after=[1, 5])

    def test_truncated_chain_without_the_root_fails(self):
        with mock.patch.object(ir_gate, 'CALLERS', 3), self.assertRaises(AssertionError):
            ir_gate.measure(DATA / 'truncated.1', ROOT)
        self.assertEqual(ir_gate.measure(DATA / 'truncated.1', ROOT)['alloc_ir'], 0)

    def test_unseparated_allocator_fails(self):
        with self.assertRaises(AssertionError):
            ir_gate.measure(DATA / 'unseparated.1', ROOT)

    def test_one_table_classifies_and_separates(self):
        cases = {'_RNvCs1_7___rustc12___rust_alloc': ('alloc', 'allocs'),
                 '_RNvCs1_7___rustc19___rust_alloc_zeroed': ('alloc', 'allocs'),
                 '_RNvCs1_7___rustc14___rust_realloc': ('alloc', 'reallocs'),
                 '_RNvCs1_7___rustc11___rdl_alloc': ('alloc', None),
                 'free': ('alloc', 'deallocs'), 'posix_memalign': ('alloc', 'allocs'),
                 '__memcpy_avx_unaligned_erms': ('mem', None), 'bcmp': ('mem', None),
                 'freelist': None, '_RNvCs1_5alloc10deallocate': None}
        for name, share in cases.items():
            self.assertEqual(ir_gate.subtracted(name), share, name)
        options = ir_gate.separate_callers()
        self.assertIn(f'--separate-callers{ir_gate.CALLERS}=*___rust_dealloc', options)
        self.assertIn(f'--separate-callers{ir_gate.CALLERS}=__memset_*', options)


if __name__ == '__main__':
    unittest.main()
