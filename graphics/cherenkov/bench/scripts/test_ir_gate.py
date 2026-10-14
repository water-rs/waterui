#!/usr/bin/env python3
"""The Ir gate's sampling over hand-made Callgrind dumps (testdata/ir_gate)."""
import importlib.util
import hashlib
import json
import pathlib
import tempfile
import unittest
from unittest import mock

SOURCE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('ir_gate', SOURCE / 'ir_gate.py')
ir_gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ir_gate)

DATA = SOURCE / 'testdata' / 'ir_gate'
ROOT = '_RNvCs1_5bench4root'


class Host(unittest.TestCase):
    def write_icd(self, directory, name, library='libvulkan_lvp.so'):
        path = directory / name
        path.write_text(json.dumps({'file_format_version': '1.0.0',
                                    'ICD': {'library_path': library, 'api_version': '1.3.0'}}))
        return path

    def test_selects_library_independently_of_manifest_filename(self):
        for name in ('lvp_icd.json', 'lvp_icd.x86_64.json', 'lvp_icd.aarch64.json'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as root:
                directory = pathlib.Path(root)
                chosen = self.write_icd(directory, name)
                self.write_icd(directory, 'intel.json', 'libvulkan_intel.so')
                self.assertEqual(ir_gate.lavapipe_icd(directory=directory), chosen.resolve())

    def test_missing_or_ambiguous_driver_requires_explicit_selection(self):
        with tempfile.TemporaryDirectory() as root:
            directory = pathlib.Path(root)
            with self.assertRaisesRegex(RuntimeError, '--icd'):
                ir_gate.lavapipe_icd(directory=directory)
            chosen = self.write_icd(directory, 'one.json')
            self.write_icd(directory, 'two.json')
            with self.assertRaisesRegex(RuntimeError, '--icd'):
                ir_gate.lavapipe_icd(directory=directory)
            self.assertEqual(ir_gate.lavapipe_icd(chosen, directory), chosen.resolve())

    def test_explicit_other_driver_is_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            path = self.write_icd(pathlib.Path(root), 'intel.json', 'libvulkan_intel.so')
            with self.assertRaisesRegex(RuntimeError, 'Lavapipe'):
                ir_gate.lavapipe_icd(path)

    def test_layer_uses_the_selected_manifest(self):
        environment = ir_gate.layer_env(pathlib.Path('/test/layers'), pathlib.Path('/test/driver.json'))
        self.assertEqual(environment['VK_DRIVER_FILES'], '/test/driver.json')
        self.assertNotIn('VK_ICD_FILENAMES', environment)

    def test_resume_rejects_a_changed_requested_binary_or_driver(self):
        with tempfile.TemporaryDirectory() as root:
            directory = pathlib.Path(root)
            old, new = directory / 'old', directory / 'new'
            old.write_bytes(b'old executable')
            new.write_bytes(b'new executable')
            icd = self.write_icd(directory, 'driver.json')
            result = {'sha256': hashlib.sha256(old.read_bytes()).hexdigest(),
                      'icd_sha256': hashlib.sha256(icd.read_bytes()).hexdigest()}
            ir_gate.verify_reused_profile(result, old, icd)
            with self.assertRaises(AssertionError):
                ir_gate.verify_reused_profile(result, new, icd)
            self.write_icd(directory, 'driver.json', 'libvulkan_lvp.so.1')
            with self.assertRaises(AssertionError):
                ir_gate.verify_reused_profile(result, old, icd)


class Sampling(unittest.TestCase):
    def test_span_subtracts_only_calls_under_the_root(self):
        # run.1 closes the call in flight when instrumentation switched on,
        # run.2 is another root's return before the root is entered, run.3
        # and run.4 are cut by a nested root, run.5 returns. Another thread
        # frees through the same deallocate node in run.3, and code outside
        # the root frees through it in run.2.
        [sample] = ir_gate.call_samples(DATA, 'run', ROOT, after=[1, 5])
        self.assertEqual(sample['ir'], 50 + 40 + 30)
        self.assertEqual(sample['ncalls'], 1)
        # free under drop (30) and __rust_dealloc (20); the free inside
        # __rust_dealloc is part of that call.
        self.assertEqual(sample['alloc_ir'], 50)
        self.assertEqual((sample['allocs'], sample['reallocs'], sample['deallocs']), (0, 0, 2))
        self.assertEqual((sample['mem_ir'], sample['mem_calls']), (12, 2))
        self.assertEqual(sample['rust_ir'], 120 - 50 - 12)

    def test_first_span_holding_a_whole_call_is_the_first_sample(self):
        # Callgrind writes only --dump-after triggers: when instrumentation
        # switches on between calls, the first span already holds a whole
        # call and is sample 1, not a skipped call in flight.
        samples = ir_gate.call_samples(DATA, 'first', ROOT, after=[2, 4])
        self.assertEqual([(s['ir'], s['ncalls'], s['alloc_ir'], s['allocs'], s['rust_ir']) for s in samples],
                         [(65, 1, 10, 1, 55)] * 2)

    def test_call_nested_in_a_subtracted_call_counts_once(self):
        # glibc's realloc copies through _int_realloc and an unnamed local
        # function: that memcpy is inside __rust_realloc's 60, not another 28.
        [sample] = ir_gate.call_samples(DATA, 'nested', ROOT, after=[1])
        self.assertEqual((sample['ir'], sample['alloc_ir'], sample['reallocs']), (108, 60, 1))
        self.assertEqual((sample['mem_ir'], sample['mem_calls']), (8, 1))
        self.assertEqual(sample['rust_ir'], 40)

    def test_later_span_without_one_call_fails(self):
        with self.assertRaises(AssertionError):
            ir_gate.call_samples(DATA, 'run', ROOT, after=[1, 2])
        with self.assertRaises(AssertionError):
            ir_gate.call_samples(DATA, 'first', ROOT, after=[1, 4])

    def test_options_name_each_function_once(self):
        options = ir_gate.function_options(['_RNvCs1_5bench4root', '_RNvCs1_5bench6nested'])
        self.assertIn('--dump-after=_RNvCs1_5bench4root', options)
        self.assertFalse([o for o in options if o.startswith('--dump-before=')])
        with self.assertRaises(AssertionError):
            ir_gate.function_options(['free'])

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
        options = ir_gate.function_options([])
        self.assertIn(f'--separate-callers{ir_gate.CALLERS}=*___rust_dealloc', options)
        self.assertIn(f'--separate-callers{ir_gate.CALLERS}=__memset_*', options)


if __name__ == '__main__':
    unittest.main()
