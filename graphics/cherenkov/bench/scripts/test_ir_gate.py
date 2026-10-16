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
# The CPU leg's roots, in the shapes the real binary's names take.
MARKER = '_RNvXs5_NtCs1_15cherenkov_bench16cherenkov_cpu_adNtB5_9CherenkovNtB7_6Engine6submit'
RENDER = '_RINvNtNtCs1_9cherenkov6engine6thread13apply_messageNtCs1_13cherenkov_cpu6RasterECs1_15cherenkov_bench'
WAITER = '_RNvMs8_NtCs1_10rayon_core8registryNtB5_12WorkerThread15wait_until_cold'
JOB = '_RNvXs2_NtCs1_10rayon_core3jobINtB5_8StackJobNtB5_4BandE0ENtB5_3Job7executeB3l_'
RUN = '_RINvNtNtCs1_9cherenkov6engine6thread3runNtCs1_13cherenkov_cpu6RasterE'
LATCH = '_RNvXs4_NtCs1_10rayon_core5latchNtB5_9LockLatchNtB5_5Latch3set'
WAIT = '_RNvMs3_NtCs1_10rayon_core5latchNtB5_9LockLatch14wait_and_reset'
ALLOC = '_RNvCs1_7___rustc12___rust_alloc'
ROOTS = ir_gate.CpuRoots(marker=MARKER, render=RENDER, waiter=WAITER, jobs=(JOB,))


def write_dump(directory, number, trigger, edges):
    """One Callgrind dump `cpu.<number>`: `edges` are (caller, callee, calls, inclusive Ir)."""
    lines = ['# callgrind format', 'version: 1', 'creator: callgrind-3.22.0', 'cmd: cherenkov-bench measure',
             f'part: {number}', '', f'desc: Trigger: --dump-{trigger[0]}={trigger[1]}', '', 'positions: line',
             'events: Ir', '', '', 'ob=(1) /bench', 'fl=(1) ???']
    ids = {}

    def name(function):
        if function in ids:
            return f'({ids[function]})'
        ids[function] = len(ids) + 1
        return f'({ids[function]}) {function}'
    for caller, callee, calls, cost in edges:
        lines += [f'fn={name(caller)}', '0 1', f'cfn={name(callee)}', f'calls={calls} 0', f'0 {cost}', '']
    (directory / f'cpu.{number}').write_text('\n'.join(lines) + '\n')


def chain(function, *callers):
    return "'".join((function, *callers))


# Four bench frames. The render thread's call is entered at dump 1 (an
# earlier one, in flight when instrumentation switched on, returns there);
# frame 1's job returns only after the frame's `submit` (dump 3), in dump 4.
# Its allocator, memory-primitive and latch calls land in the dumps between
# its entry and its return; a call inside the latch wait is part of it.
CPU_RUN = {
    1: (('before', RENDER), [(RUN, RENDER, 1, 9)]),
    2: (('before', JOB), [(RENDER, chain(ALLOC, RENDER, RUN), 1, 5)]),
    3: (('after', MARKER), [(RUN, RENDER, 1, 100), (RENDER, chain(WAIT, RENDER, RUN), 1, 20),
                            (WAIT, chain(ALLOC, WAIT, RENDER, RUN), 1, 4), (JOB, chain('memcpy', JOB, WAITER), 1, 8)]),
    4: (('before', RENDER), [(WAITER, JOB, 1, 300), (JOB, chain(LATCH, JOB, WAITER), 1, 6)]),
    5: (('before', JOB), []),
    6: (('after', MARKER), [(RUN, RENDER, 1, 110), (WAITER, JOB, 1, 280), (JOB, chain('free', JOB, WAITER), 1, 3)]),
    7: (('before', RENDER), []),
    8: (('before', JOB), []),
    9: (('after', MARKER), [(RUN, RENDER, 1, 120), (WAITER, JOB, 1, 290)]),
    10: (('before', RENDER), []),
    11: (('after', MARKER), [(RUN, RENDER, 1, 130)]),
}


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
                self.assertEqual(ir_gate.lavapipe_icd(directories=[directory]), chosen.resolve())

    def test_missing_or_ambiguous_driver_requires_explicit_selection(self):
        with tempfile.TemporaryDirectory() as root:
            directory = pathlib.Path(root)
            with self.assertRaisesRegex(RuntimeError, '--icd'):
                ir_gate.lavapipe_icd(directories=[directory])
            chosen = self.write_icd(directory, 'one.json')
            self.write_icd(directory, 'two.json')
            with self.assertRaisesRegex(RuntimeError, '--icd'):
                ir_gate.lavapipe_icd(directories=[directory])
            self.assertEqual(ir_gate.lavapipe_icd(chosen, [directory]), chosen.resolve())

    def test_driver_search_follows_the_loader(self):
        self.assertEqual(ir_gate.icd_directories({'HOME': '/home/u'}),
                         [pathlib.Path(p) / 'vulkan' / 'icd.d' for p in
                          ('/home/u/.config', '/etc/xdg', '/etc', '/home/u/.local/share', '/usr/local/share', '/usr/share')])
        self.assertEqual(ir_gate.icd_directories({'HOME': '/h', 'XDG_DATA_DIRS': '/a:/b', 'XDG_CONFIG_DIRS': '/c'}),
                         [pathlib.Path(p) / 'vulkan' / 'icd.d' for p in
                          ('/h/.config', '/c', '/etc', '/h/.local/share', '/a', '/b')])

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


    def test_build_takes_the_one_executable_cargo_reports(self):
        artifact = {'reason': 'compiler-artifact', 'executable': '/target/release/cherenkov-bench'}
        library = {'reason': 'compiler-artifact', 'executable': None}
        messages = '\n'.join(json.dumps(m) for m in (library, {'reason': 'build-finished'}, artifact))
        self.assertEqual(ir_gate.executable(messages), pathlib.Path('/target/release/cherenkov-bench'))
        with self.assertRaisesRegex(RuntimeError, 'one executable'):
            ir_gate.executable(json.dumps(library))
        with self.assertRaisesRegex(RuntimeError, 'one executable'):
            ir_gate.executable('\n'.join(json.dumps(artifact) for _ in range(2)))


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

    def test_profile_stops_on_its_declared_roots(self):
        every, once = ir_gate.EVERY_FRAME, ir_gate.RECORDED_ONCE
        self.assertTrue(ir_gate.sampled({'lower': [2] * 4, 'encode': [1] * 4, 'run_transaction': []}, once, 's'))
        self.assertFalse(ir_gate.sampled({'lower': [2] * 3, 'encode': [1] * 8, 'run_transaction': []}, once, 's'))
        with self.assertRaisesRegex(RuntimeError, 'declare'):
            ir_gate.sampled({'lower': [2], 'encode': [1], 'run_transaction': [3]}, once, 's')
        with self.assertRaisesRegex(RuntimeError, 'does not run every frame'):
            ir_gate.sampled({'lower': [2] * 9, 'encode': [1] * 9, 'run_transaction': []}, every, 's')

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

    def test_leg_builds_its_engine(self):
        self.assertIn('cherenkov', ir_gate.build_args(ir_gate.LEGS['gpu']))
        self.assertIn('cherenkov-cpu', ir_gate.build_args(ir_gate.LEGS['cpu']))
        self.assertEqual(list(ir_gate.LEGS['cpu'].scenes), list(ir_gate.SCENES))

    def test_sync_is_subtracted_only_where_the_leg_asks(self):
        latch = '_RNvXs4_NtCs1_10rayon_core5latchNtB5_9LockLatchNtB5_5Latch3set'
        self.assertIsNone(ir_gate.subtracted(latch))
        self.assertEqual(ir_gate.subtracted(latch, ir_gate.SYNC), ('sync', None))
        self.assertIsNone(ir_gate.subtracted(JOB, ir_gate.SYNC))
        options = ir_gate.function_options([], ir_gate.SYNC, before=[RENDER, ir_gate.CPU_JOB], marker=MARKER)
        self.assertIn(f'--separate-callers{ir_gate.CALLERS}=*10rayon_core5latch*5Latch3set*', options)
        self.assertIn(f'--dump-before={ir_gate.CPU_JOB}', options)
        self.assertIn(f'--dump-after={MARKER}', options)

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


class CpuFrames(unittest.TestCase):
    def samples(self, run):
        with tempfile.TemporaryDirectory() as root:
            directory = pathlib.Path(root)
            for number, (trigger, edges) in run.items():
                write_dump(directory, number, trigger, edges)
            return ir_gate.frame_samples(directory, 'cpu', {n: t for n, (t, _) in run.items()}, ROOTS)

    def test_calls_belong_to_the_frame_of_their_entry(self):
        first, second, third = self.samples(CPU_RUN)
        self.assertEqual((first['ir'], first['jobs'], first['messages']), (100 + 300, 1, 1))
        self.assertEqual((first['alloc_ir'], first['allocs']), (5, 1))
        self.assertEqual((first['mem_ir'], first['mem_calls']), (8, 1))
        self.assertEqual((first['sync_ir'], first['sync_calls']), (20 + 6, 2))
        self.assertEqual(first['rust_ir'], 400 - 5 - 8 - 26)
        self.assertEqual((second['ir'], second['alloc_ir'], second['deallocs'], second['rust_ir']), (390, 3, 1, 387))
        self.assertEqual((third['ir'], third['rust_ir']), (410, 410))

    def test_a_lost_return_fails(self):
        run = dict(CPU_RUN)
        run[4] = (('before', RENDER), [])
        with self.assertRaisesRegex(AssertionError, 'never returned'):
            self.samples(run)

    def test_a_return_without_its_entry_fails(self):
        run = dict(CPU_RUN)
        run[5] = (('before', RENDER), [])
        with self.assertRaisesRegex(AssertionError, 'returned without an entry'):
            self.samples(run)

    def test_a_nested_job_fails(self):
        run = dict(CPU_RUN)
        run[6] = (('after', MARKER), [(RUN, RENDER, 1, 110), (JOB, JOB, 1, 280)])
        with self.assertRaisesRegex(AssertionError, 'one worker per pool'):
            self.samples(run)

    def test_too_few_frames_fail(self):
        run = {n: v for n, v in CPU_RUN.items() if n < 11}
        with self.assertRaisesRegex(AssertionError, 'frames closed'):
            self.samples(run)


if __name__ == '__main__':
    unittest.main()
