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
# The caller Callgrind reports for a job at opt-level 3: the `Sleep::sleep`
# instantiation for `wait_until_cold`'s closure.
SLEEPER = '_RINvMNtCs1_10rayon_core5sleepNtB3_5Sleep5sleepNCNvMs8_NtB5_8registryNtB10_12WorkerThread15wait_until_cold0EB5_'
JOB = '_RNvXs2_NtCs1_10rayon_core3jobINtB5_8StackJobNtB5_4BandE0ENtB5_3Job7executeB3l_'
RUN = '_RINvNtNtCs1_9cherenkov6engine6thread3runNtCs1_13cherenkov_cpu6RasterE'
LATCH = '_RNvXs4_NtCs1_10rayon_core5latchNtB5_9LockLatchNtB5_5Latch3set'
WAIT = '_RNvMs3_NtCs1_10rayon_core5latchNtB5_9LockLatch14wait_and_reset'
ALLOC = '_RNvCs1_7___rustc12___rust_alloc'
FUTEX = '_RNvMNtNtNtNtCs1_3std3sys4sync5mutex5futexNtB2_5Mutex'
ROOTS = ir_gate.CpuRoots(marker=MARKER, render=RENDER, jobs=(JOB,))


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


# Four bench frames, written the way Callgrind writes them: every dump holds
# the Ir each call ran since the previous dump, returned or not, and a call's
# count rides on the first nonzero write after its entry. Dump 1 holds the
# tail of a render call in flight when instrumentation switched on, and a
# futex `syscall` the parked worker entered before then, with no caller
# chain. Frame 1's render call ends in dump 4, whose write carries the count
# of the call that dump's own trigger enters; frame 1's job runs past the
# frame's `submit` (dump 3) into dump 4. The allocator, memory-primitive and
# latch calls under a root land in the dumps of its window; a call inside
# the latch wait is part of it.
CPU_RUN = {
    1: (('before', RENDER), [(RUN, RENDER, 0, 9), (WAITER, 'syscall', 1, 2)]),
    2: (('before', JOB), [(RUN, RENDER, 1, 60), (RENDER, chain(ALLOC, RENDER, RUN), 1, 5)]),
    3: (('after', MARKER), [(RUN, RENDER, 0, 40), (RENDER, chain(WAIT, RENDER, RUN), 1, 20),
                            (WAIT, chain(ALLOC, WAIT, RENDER, RUN), 1, 4), (WAITER, JOB, 1, 200),
                            (JOB, chain('memcpy', JOB, WAITER), 1, 8)]),
    4: (('before', RENDER), [(RUN, RENDER, 1, 7), (WAITER, JOB, 0, 100), (JOB, chain(LATCH, JOB, WAITER), 1, 6)]),
    5: (('before', JOB), [(RUN, RENDER, 0, 70), (RENDER, chain('free', RENDER, RUN), 1, 3)]),
    6: (('after', MARKER), [(RUN, RENDER, 0, 40), (WAITER, JOB, 1, 280)]),
    7: (('before', RENDER), []),
    8: (('before', JOB), [(RUN, RENDER, 1, 115)]),
    9: (('after', MARKER), [(RUN, RENDER, 0, 5), (WAITER, JOB, 1, 290)]),
    10: (('before', RENDER), []),
    11: (('before', JOB), [(RUN, RENDER, 1, 100)]),
    12: (('after', MARKER), [(RUN, RENDER, 0, 30), (WAITER, JOB, 1, 300)]),
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
    def test_joined_worker_cost_is_counted_with_its_lowering_call(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            write_dump(root, 1, ('after', MARKER), [
                (RUN, ROOT, 1, 40), (WAITER, JOB, 1, 200),
                (ROOT, chain(WAIT, ROOT, RUN), 1, 10),
                (JOB, chain(LATCH, JOB, WAITER), 1, 6),
                (JOB, chain(ALLOC, JOB, WAITER), 1, 5),
                (JOB, chain('memcpy', JOB, WAITER), 1, 8),
            ])
            write_dump(root, 2, ('after', ROOT), [
                (RUN, ROOT, 0, 60), (WAITER, JOB, 1, 100),
                (JOB, chain(ALLOC, JOB, WAITER), 1, 2),
                (JOB, chain('memcpy', JOB, WAITER), 1, 4),
            ])
            [sample] = ir_gate.call_samples(root, 'cpu', ROOT, [2], count=1, workers=(JOB,), sync=ir_gate.SYNC)
            self.assertEqual(sample['ncalls'], 1)
            self.assertEqual(sample['worker_calls'], 2)
            self.assertEqual(sample['ir'], 400)
            self.assertEqual(sample['rust_ir'], 365)
            self.assertEqual(sample['sync_ir'], 16)
            self.assertEqual(sample['allocs'], 2)
            self.assertEqual(sample['mem_calls'], 2)

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

    def test_fresh_sample_requires_the_first_complete_call(self):
        [sample] = ir_gate.call_samples(DATA, 'first', ROOT, after=[2, 4],
                                        count=1, allow_inflight=False)
        self.assertEqual(sample['rust_ir'], 55)
        with self.assertRaises(AssertionError):
            ir_gate.call_samples(DATA, 'run', ROOT, after=[1, 5],
                                 count=1, allow_inflight=False)
        self.assertTrue(ir_gate.sampled({'lower': [2], 'encode': [3], 'run_transaction': [1]},
                                        ir_gate.EVERY_FRAME, 'fresh', returns=1))
        self.assertFalse(ir_gate.sampled({'lower': [], 'encode': [], 'run_transaction': [1]},
                                         ir_gate.EVERY_FRAME, 'fresh', returns=1))

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

    def test_call_on_an_empty_call_stack_is_charged_to_no_root(self):
        # Every subtracted function is separated by caller chain, so a call
        # without one was entered on an empty call stack, before
        # instrumentation switched on.
        sample = ir_gate.measure(DATA / 'stackless.1', ROOT)
        self.assertEqual((sample['ir'], sample['alloc_ir'], sample['allocs']), (20, 0, 0))

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
        self.assertIn(f'--separate-callers{ir_gate.CALLERS}=*10rayon_core5latch*', options)
        self.assertIn(f'--dump-before={ir_gate.CPU_JOB}', options)
        self.assertIn(f'--dump-after={MARKER}', options)

    def test_callgrind_trie_drops_patterns_a_glob_would_match(self):
        # Callgrind follows only the first child whose text prefixes the
        # name: the second `5latch` child, and a `*` read literally after a
        # whole node, never reach their functions.
        patterns = ir_gate.CallgrindPatterns(['--separate-callers9=*10rayon_core8registry*inject*',
                                              '--separate-callers9=*10rayon_core5latch*wait_and_reset*',
                                              '--separate-callers9=*10rayon_core5latch*Latch3set*',
                                              '--separate-callers9=*5mutex5futex*lock_contended*',
                                              '--separate-callers9=*5mutex5futex*wake*'])
        self.assertEqual(patterns.options_reaching(LATCH), {'--separate-callers9'})
        self.assertEqual(patterns.options_reaching(WAIT), set())
        self.assertEqual(patterns.options_reaching(f'{FUTEX}14lock_contended'), {'--separate-callers9'})
        self.assertEqual(patterns.options_reaching(f'{FUTEX}4wake'), set())

    def test_the_legs_options_reach_what_they_name(self):
        functions = [ALLOC, '_RNvCs1_7___rustc19___rust_alloc_zeroed', '_RNvCs1_7___rustc14___rust_realloc',
                     '_RNvCs1_7___rustc14___rust_dealloc', '_RNvCs1_7___rustc11___rdl_alloc', 'malloc', 'realloc',
                     'free', 'posix_memalign', 'memcpy', '__memcpy_avx_unaligned_erms', '__memset_avx2_unaligned_erms',
                     'bcmp', '__memcmp_evex_movbe', LATCH, WAIT, f'{FUTEX}14lock_contended', f'{FUTEX}4wake',
                     '_RNvMs4_NtCs1_10rayon_core8registryNtB5_8Registry6inject',
                     '_RNvMNtCs1_10rayon_core5sleepNtB2_5Sleep16wake_any_threads',
                     '_RNvMs0_NtCs1_17crossbeam_channel5wakerNtB5_9SyncWaker6notify',
                     '_RNvMNtNtNtCs1_3std4sync4mpmc7contextNtB2_7Context10wait_until',
                     '_RNvMs0_NtNtNtCs1_3std4sync4mpmc5wakerNtB5_9SyncWaker6notify',
                     '_RINvMNtNtNtCs1_3std4sync6poison7condvarNtB3_7Condvar4waitbECs1_10rayon_core',
                     'syscall', 'sched_yield']
        command = ir_gate.function_options([], ir_gate.SYNC, before=[RENDER, ir_gate.CPU_JOB], marker=MARKER)
        self.assertEqual(ir_gate.unseparated(command, functions, ir_gate.SYNC), [])
        ir_gate.check_roots(command, {RENDER: '--dump-before', JOB: '--dump-before', MARKER: '--dump-after'}, 'cpu')
        gpu = ir_gate.function_options([ROOT, '_RNvCs1_5bench6encode'])
        self.assertEqual(ir_gate.unseparated(gpu, functions), [])
        ir_gate.check_roots(gpu, {ROOT: '--dump-after'}, 'gpu')
        with self.assertRaisesRegex(RuntimeError, 'other options'):
            ir_gate.check_roots(command, {WAIT: '--dump-before'}, 'cpu')

    def test_a_run_with_an_unseparated_function_fails(self):
        command = ['--separate-callers9=*5mutex5futex*lock_contended*', '--separate-callers9=*5mutex5futex*wake*']
        with tempfile.TemporaryDirectory() as root:
            directory = pathlib.Path(root)
            write_dump(directory, 1, ('after', MARKER), [(RENDER, f'{FUTEX}4wake', 1, 3),
                                                         (RENDER, chain(f'{FUTEX}14lock_contended', RENDER), 1, 9)])
            with self.assertRaisesRegex(RuntimeError, '4wake'):
                ir_gate.check_separated(command, directory, 'cpu', ir_gate.SYNC)

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


class Roots(unittest.TestCase):
    def test_run_transaction_is_the_instantiation_encode_opens(self):
        record = '_RINvMs1_NtCs1_16cherenkov_record7surfaceINtB6_6SharedNtCs1_13cherenkov_gpu3GpuE15run_transactionNCNvXs7_NtCs1_15cherenkov_bench12cherenkov_adNtB1Y_9CherenkovNtB20_6Engine{}EB20_'
        encode, prepare = record.format('6encodes_0'), record.format('7prepares4_0')
        suffix, parts = ir_gate.ROOTS['run_transaction']
        for symbols in ([encode], [prepare, encode]):
            self.assertEqual(ir_gate.one_symbol(symbols, suffix, parts, 't', 'run_transaction'), encode)
        with self.assertRaisesRegex(RuntimeError, 'expected one run_transaction symbol'):
            ir_gate.one_symbol([prepare], suffix, parts, 't', 'run_transaction')


class CpuFrames(unittest.TestCase):
    def samples(self, run):
        with tempfile.TemporaryDirectory() as root:
            directory = pathlib.Path(root)
            for number, (trigger, edges) in run.items():
                write_dump(directory, number, trigger, edges)
            return ir_gate.frame_samples(directory, 'cpu', {n: t for n, (t, _) in run.items()}, ROOTS)

    def test_calls_belong_to_the_frame_of_their_entry(self):
        first, second, third = self.samples(CPU_RUN)
        self.assertEqual((first['ir'], first['jobs'], first['messages']), (107 + 300, 1, 1))
        self.assertEqual((first['alloc_ir'], first['allocs']), (5, 1))
        self.assertEqual((first['mem_ir'], first['mem_calls']), (8, 1))
        self.assertEqual((first['sync_ir'], first['sync_calls']), (20 + 6, 2))
        self.assertEqual(first['rust_ir'], 407 - 5 - 8 - 26)
        self.assertEqual((second['ir'], second['alloc_ir'], second['deallocs'], second['rust_ir']), (390, 3, 1, 387))
        self.assertEqual((third['ir'], third['rust_ir']), (410, 410))

    def test_a_call_without_its_dump_fails(self):
        run = dict(CPU_RUN)
        run[8] = (('before', JOB), [(RUN, RENDER, 2, 115)])
        with self.assertRaisesRegex(AssertionError, 'entered without its dump'):
            self.samples(run)

    def test_a_dump_without_its_call_fails(self):
        run = dict(CPU_RUN)
        run[8] = (('before', JOB), [(RUN, RENDER, 0, 115)])
        run[11] = (('before', JOB), [(RUN, RENDER, 0, 100)])
        with self.assertRaisesRegex(AssertionError, 'a dump without its call'):
            self.samples(run)

    def test_a_sampled_call_running_at_the_last_dump_fails(self):
        run = dict(CPU_RUN)
        run[11] = (('after', MARKER), [(RUN, RENDER, 1, 100)])
        run[12] = (('after', MARKER), [(RUN, RENDER, 0, 30), (WAITER, JOB, 0, 300)])
        with self.assertRaisesRegex(AssertionError, 'still running at the last dump'):
            self.samples(run)

    def test_a_job_counts_whichever_pool_function_dispatches_it(self):
        run = {n: (trigger, [(SLEEPER if caller == WAITER else caller, callee, calls, cost)
                             for caller, callee, calls, cost in edges])
               for n, (trigger, edges) in CPU_RUN.items()}
        self.assertEqual(self.samples(run), self.samples(CPU_RUN))

    def test_a_nested_job_fails(self):
        run = dict(CPU_RUN)
        run[6] = (('after', MARKER), [(RUN, RENDER, 0, 40), (JOB, JOB, 1, 280)])
        with self.assertRaisesRegex(AssertionError, 'one worker per pool'):
            self.samples(run)

    def test_too_few_frames_fail(self):
        run = {n: v for n, v in CPU_RUN.items() if n < 10}
        with self.assertRaisesRegex(AssertionError, 'frames closed'):
            self.samples(run)


if __name__ == '__main__':
    unittest.main()
