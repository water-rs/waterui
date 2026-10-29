#!/usr/bin/env python3
"""Callgrind Ir gate on the gated perf scenes, in two legs.

The `gpu` leg counts the GPU engine's `lower`, `encode` and `run_transaction`,
each scene sampling the roots it runs in steady state (SCENES). The `cpu` leg
counts the CPU backend's whole frame: the render thread's message handling and
every raster job its worker pools run (CPU_SCENES, phase `frame`).

Each profile runs `cherenkov-bench measure --pause-at P` under Callgrind with
instrumentation off. At the pause (a fixed frame index, not wall-clock time)
instrumentation is switched on and the bench resumes. Callgrind keeps one cost
table for all threads (`--separate-threads=no`), so every dump holds every
thread's events since the previous one and the dumps partition the run.

GPU leg. `--dump-after` on each root cuts a dump at every return. One call of
a root is the span between consecutive `--dump-after` triggers of that root,
holding exactly one incoming call of it: a root nested inside it
(`run_transaction` runs inside `encode`) and the other roots write their own
dumps within the span, so a sample sums the root's incoming edge and the
allocator calls over every dump in its span. The k-th call is sample k;
sample 2 is the gate's "second steady frame" and sample 3 its stability check.
GPU geometry batches join before the lowering root returns. Their worker-root
costs are added over the same dump span, with RAYON_NUM_THREADS=1 to include
all batch work and allocation beneath that root in deterministic order.
Only the first span may hold no complete call — the call in flight when
instrumentation switched on — so any later skip is an error, never a silent
shift of which frame is sampled.

CPU leg. The backend's frame runs on threads other than the bench's: the
render thread handles each message in `apply_message`, and the raster work it
hands to rayon (`ThreadPool::install`, and the global pool for plain `par_*`
calls) runs on pool workers, one job per `Job::execute` a worker's
`wait_until_cold` picks up. The leg runs with RAYON_NUM_THREADS=1, so every
pool has one worker and the jobs run in one fixed order; a native run checks
the backend reports one thread before every profile. At every dump Callgrind
writes, on every thread, the Ir each call has run since the previous dump,
returned or not, so a call's cost is spread over the dumps from its entry to
its return; its call count rides on the first nonzero write after the entry,
shared with any other call from the same call site, and pairs nothing. So
`--dump-before` on each root cuts a dump at every entry, `--dump-after` on the
adapter's `submit` (main thread) closes every bench frame, and a root call
belongs to the frame whose span holds its entry dump. Calls of one root
function run one after another, so the Ir that function accrues in the dumps
after one entry up to its next entry is that call's, whether it returns
before or after `submit` does; the counts written are checked against the
entries. Ir a root accrues before its first entry belongs to a call in flight
when instrumentation switched on and is no sample. Sample k is the k-th frame
after the pause; a profile runs one frame more so every sampled call is
closed by a later entry.

Callgrind keeps function options in a prefix trie (clo.c) whose lookup
follows only the first child whose text prefixes a name, while insertion
files a pattern that continues past a whole node as a new child of it without
merging it with an existing child of the same text, and reads a `*` right
after such a node literally. So an option can match a function by its glob
and never reach it: of `--dump-before=S` and `--dump-after=S` only the latter
takes effect, and of two patterns sharing a module path only the one given
last may apply. Every option names a function at most once, CallgrindPatterns
ports the trie, and every profile fails unless each root receives exactly its
dump trigger and every subtracted function its run executed is keyed by
caller chain.

Per sample it reports the root's inclusive Ir; the allocator's share of it
(the inclusive cost of every call from other code into the Rust allocator
shims and the libc malloc family) with the number of alloc, realloc and
dealloc calls; the libc memory primitives' share (memcpy, memmove, memset,
memcmp, bcmp), whose Ir depends on buffer addresses the allocator chooses,
with their call count; for the CPU leg the cross-thread synchronization's
share (rayon's job injection, latches and sleep, channel wakers, futex slow
paths), whose Ir depends on how the threads interleave, with its call count;
and the Rust Ir: inclusive Ir minus every share. A subtracted call made inside
another one, directly or through frames the gate keeps, is already in the
outer call's inclusive cost and is not counted again.

A share counts only calls made under the root, decided by calling context:
`--separate-callers<N>` keys every subtracted function by the N functions
above it on the calling thread's stack, so each call edge into one names its
caller chain, and a call is the root's exactly when the root is in that chain.
A chain shorter than N reached the bottom of the thread's stack, so a root
missing from it is outside the root, and a call with no chain at all was
entered on an empty stack, before instrumentation switched on (a thread
parked in a futex wait), outside every root; a chain of full length without the root
is undecidable and fails the run. Neither the plain call graph nor per-thread
dumps can decide it: the graph merges every caller of a shared function such
as `RawVecInner::deallocate`, and `--separate-threads=yes` does not isolate
threads — after instrumentation switches on mid-run, a thread's call-stack
underflow can attach its calls to a record another thread owns, so another
thread's frees land in the root thread's dumps.

  ir_gate.py ab      --leg L --base REV --head REV --work W --out O  # build both, profile, compare
  ir_gate.py profile --leg L --binary B --tag T --scene S --repo R --out O
  ir_gate.py batch   --leg L --binary B --tag T --repo R --out O
  ir_gate.py compare --leg L --out O BASE HEAD      # the +1% / no-more-calls rule
  ir_gate.py same    --leg L --out O TAG TAG [TAG]  # identical Ir across re-profiles

`ab` is the whole gate for one change: it checks out the merge base of BASE
and HEAD and HEAD itself as worktrees of equal path length under W, builds
each tree's `cherenkov-bench` from its own source with the leg's build
settings (release, unstripped symbols, limited debug info, the leg's engine
feature) into one shared target directory, profiles both against the perf
scenes of the checkout this script belongs to, writes provenance and
`comparison.md` into O, and exits non-zero when the gate fails. A revision
the checkout does not hold is fetched from origin. `profile` and `batch` take
a binary built the same way: without symbols the roots cannot be found and
the run stops.

For the GPU leg's Lavapipe it finds the installed driver manifest by its
library, or accepts an explicit --icd manifest. It sets VK_DRIVER_FILES,
XDG_RUNTIME_DIR and RUST_LOG, and it builds and enables
VK_LAYER_CHERENKOV_no_raster (ir_no_raster.c), a global instance layer that
no-ops the GPU-work vkCmd* entry points under Callgrind: lavapipe
rasterization dominates the profile wall time but every vkCmd* call sits
outside the measured lower/encode roots, so skipping it cannot change the
counted Ir. NODEVICE_SELECT disables the MESA device_select layer, whose
teardown crashes when another instance layer sits in front of it.
"""
import argparse, collections, concurrent.futures, ctypes, fnmatch, hashlib, json, os, pathlib, platform, re, selectors, shutil, struct, subprocess, sys, typing

# Each root as (name suffix, the substrings its symbol must hold).
# `run_transaction` is generic over its closure, and the bench opens one
# transaction per frame in `Engine::encode` and one at setup in
# `Engine::prepare`; whether the compiler keeps the setup instantiation as its
# own symbol depends on inlining (opt-level "z" folds it into `prepare`, 3
# does not), so the root is the instantiation whose closure is `encode`'s.
ROOTS = {'lower': ('13lower_content', ('cherenkov_gpu',)), 'encode': ('6Engine6encode', ('cherenkov_ad',)),
         'run_transaction': ('15run_transaction', ('cherenkov_record', '6Engine6encode'))}
# The roots each perf scene runs in steady state. `encode` runs once per frame
# in every scene. A motion or live scene records its content on the first
# frame only and later frames open no transaction (the bench's Cherenkov
# adapter), so `run_transaction` is not one of its roots. Every root is a
# dump trigger in every profile: a declared root that stops running, or an
# undeclared one that starts, fails the profile.
EVERY_FRAME = ('lower', 'encode', 'run_transaction')
RECORDED_ONCE = ('lower', 'encode')
SCENES = {'map': EVERY_FRAME, 'chart': EVERY_FRAME, 'text-page': EVERY_FRAME, 'ui-list': EVERY_FRAME,
          'effects': EVERY_FRAME, 'road-network': RECORDED_ONCE, 'brick-grid': RECORDED_ONCE,
          'sparse-bbox': RECORDED_ONCE}
FRAME_ROOT = 'encode'
# Frames after which every declared root must have returned the four times a
# profile needs (the call in flight, then three samples).
STALL_FRAMES = 8
PAUSE = 'cherenkov-bench: paused before frame {}'
# The CPU leg's one phase: the render thread's messages and the pool jobs
# they hand out, per bench frame, on the same perf scenes.
CPU_FRAME = 'frame'
CPU_SCENES = {scene: (CPU_FRAME,) for scene in SCENES}
# The CPU leg's mangled names (`--demangle=no`, Rust v0 mangling), as
# (suffix, substrings) like ROOTS: the adapter's `submit` closes a bench frame
# on the main thread, and `apply_message::<Raster>` handles one message on the
# render thread.
CPU_MARKER = ('6Engine6submit', ('16cherenkov_cpu_ad',))
CPU_RENDER = ('13apply_message', ('13cherenkov_cpu6Raster',))
# The crate whose scheduler dispatches every pool job. Which of its functions
# Callgrind reports as a job's caller depends on inlining and code layout
# (`wait_until_cold` at opt-level "z"; at 3, the `Sleep::sleep` instantiation
# for its closure, which never calls a job itself), so a job's caller is
# checked by crate, not by name.
CPU_POOL = '10rayon_core'
# Every rayon job's entry point: `<StackJob<..> as Job>::execute` and the
# other job kinds, one function per job type.
CPU_JOB = '*10rayon_core3job*3Job7execute*'
# The pool size the CPU leg runs at, and the backend's report of it.
CPU_THREADS = '1'
CPU_ADAPTER = f'cherenkov-cpu ({CPU_THREADS} threads)'
# The CPU leg's sampled frames: three samples, and one frame more so every
# call entered in them has returned.
CPU_SAMPLES = 3
# The build every profiled binary comes from. The profile overrides apply to
# any revision, so a baseline that predates the gate builds the same way.
# Use the system GNU linker for both arms so the profiler sees a consistent
# executable mapping layout alongside the unstripped function symbols.
BUILD_ENV = {'CARGO_PROFILE_RELEASE_DEBUG': '1', 'CARGO_PROFILE_RELEASE_STRIP': 'false',
             'RUSTFLAGS': '-C linker-features=-lld'}
# Callgrind name patterns (`*` matches any run) of the functions a sample
# subtracts, with the call count each one adds to; Callgrind separates these
# by caller chain and the gate classifies them, from this one table. Names
# are mangled (`--demangle=no`): a Rust allocator shim keeps its crate path
# ahead of the name, so it matches as a suffix.
ALLOCATOR = ([(f'*___rust_{name}', kind) for name, kind in
              (('alloc', 'allocs'), ('alloc_zeroed', 'allocs'), ('realloc', 'reallocs'), ('dealloc', 'deallocs'))]
             + [(f'*___rdl_{name}', None) for name in ('alloc', 'alloc_zeroed', 'realloc', 'dealloc')]
             + [('*___rust_no_alloc_shim_is_unstable*', None)]
             + [(name, 'allocs') for name in ('malloc', 'calloc', 'posix_memalign', 'aligned_alloc', 'memalign', 'valloc', 'pvalloc')]
             + [('realloc', 'reallocs'), ('free', 'deallocs')])
MEMORY = [pattern for name in ('memcpy', 'memmove', 'mempcpy', 'memset', 'memcmp', 'bcmp')
          for pattern in (name, f'{name}_*', f'__{name}', f'__{name}_*')]
# The CPU leg's cross-thread synchronization, by module path: rayon handing a
# job to a pool, its latches (waiting on one, setting it) and its sleeping and
# waking workers; the channels' wakers, blocking contexts and backoff spins;
# std's condvar and futex mutex and its channel's wakers, contexts and
# backoff; the libc entries std's futex calls and rayon's yields go through.
# One pattern per module where Callgrind's pattern trie would otherwise drop
# one of two (CallgrindPatterns).
SYNC = ['*10rayon_core8registry*8Registry6inject*', '*10rayon_core5latch*', '*10rayon_core5sleep*5Sleep*',
        '*17crossbeam_channel5waker*', '*17crossbeam_channel7context*', '*15crossbeam_utils7backoff*',
        '*3std4sync4mpmc7context*', '*3std4sync4mpmc5waker*', '*3std4sync4mpmc5utils*7Backoff*',
        '*3std4sync6poison7condvar*7Condvar*', '*5mutex5futex*5Mutex*', '*16parking_lot_core8spinwait*',
        'syscall', 'sched_yield']
KINDS = ('allocs', 'reallocs', 'deallocs')
# Caller-chain depth of every allocator and memory-primitive function: deep
# enough to reach the root from any call under it, or the run fails.
CALLERS = 256
LIBC = ctypes.CDLL(None, use_errno=True)


class Leg(typing.NamedTuple):
    """One leg of the gate: the engine it builds and profiles, and its scenes' phases."""
    name: str
    feature: str
    scenes: dict
    title: str


LEGS = {'gpu': Leg('gpu', 'cherenkov', SCENES, 'GPU engine'),
        'cpu': Leg('cpu', 'cherenkov-cpu', CPU_SCENES, 'CPU backend')}


def build_args(leg):
    """The cargo arguments of the leg's profiled binary."""
    return ['--release', '-p', 'cherenkov-bench', '--features', leg.feature, '--bin', 'cherenkov-bench']


class Timespec(ctypes.Structure):
    _fields_ = [('seconds', ctypes.c_long), ('nanos', ctypes.c_long)]


class Itimerspec(ctypes.Structure):
    _fields_ = [('interval', Timespec), ('value', Timespec)]


def timer(seconds):
    fd = LIBC.timerfd_create(1, os.O_CLOEXEC)
    if fd < 0:
        raise OSError(ctypes.get_errno(), 'timerfd_create')
    if LIBC.timerfd_settime(fd, 0, ctypes.byref(Itimerspec(Timespec(0, 0), Timespec(seconds, 0))), None):
        raise OSError(ctypes.get_errno(), 'timerfd_settime')
    return fd


def parse(path):
    """Function names and per-edge inclusive Ir and call counts."""
    names = {}; edge = collections.Counter(); calls = collections.Counter()
    current = callee = None; pending = None
    for line in path.read_text().splitlines():
        m = re.match(r'(c?fn)=\((\d+)\)(?: (.*))?$', line)
        if m:
            typ, n, name = m.groups()
            if name is not None:
                names[n] = name
            if typ == 'fn':
                current = n; pending = None
            else:
                callee = n
        elif line.startswith('calls='):
            pending = int(line[6:].split()[0])
        elif pending is not None and current is not None and re.match(r'^[0-9+*\-]', line):
            parts = line.split()
            if len(parts) != 2 or not parts[-1].isdigit():
                continue
            edge[(current, callee)] += int(parts[-1]); calls[(current, callee)] += pending; pending = None
    return names, edge, calls


def context(name):
    """A Callgrind context name, `fn['rec]'caller1'caller2…`, as the function and its caller chain."""
    parts = name.split("'")
    callers = parts[1:]
    if callers and callers[0].isdigit():
        callers = callers[1:]
    return parts[0], callers


def subtracted(function, sync=()):
    """('alloc', kind), ('mem', None) or ('sync', None) for a function a sample subtracts, else None."""
    for pattern, kind in ALLOCATOR:
        if fnmatch.fnmatchcase(function, pattern):
            return 'alloc', kind
    if any(fnmatch.fnmatchcase(function, pattern) for pattern in MEMORY):
        return 'mem', None
    if any(fnmatch.fnmatchcase(function, pattern) for pattern in sync):
        return 'sync', None
    return None


def share(names, edge, calls, symbol_of, sync=()):
    """Each subtracted call edge under a root, by root: Ir and call counts per share.

    `symbol_of` names the root a caller chain belongs to (the innermost root
    in it), or None. A call made inside another subtracted call is part of
    that call's cost and is skipped.
    """
    shares = collections.defaultdict(lambda: collections.Counter({k: 0 for k in ('alloc_ir', 'mem_ir', 'mem_calls', 'sync_ir', 'sync_calls', *KINDS)}))
    for (a, b), cost in edge.items():
        function, callers = context(names[b])
        kind = subtracted(function, sync)
        if kind is None:
            continue
        if not callers:
            # Every subtracted function is separated by caller chain, so an
            # empty chain means its thread's call stack was empty: the call
            # was entered before instrumentation switched on (a thread parked
            # in a futex wait), on a thread no root call had entered yet.
            continue
        found = [i for i, caller in enumerate(callers) if symbol_of(caller) is not None]
        if not found:
            assert len(callers) < CALLERS, (names[b], f'caller chain truncated at {CALLERS} without the root')
            continue
        index = found[0]
        # A call made inside another subtracted call is part of that call's
        # cost, whether its caller is the outer function or a frame the gate
        # keeps between them (glibc's realloc reaches memcpy through
        # `_int_realloc` and an unnamed local function).
        if any(subtracted(caller, sync) is not None for caller in callers[:index]):
            continue
        counter = shares[symbol_of(callers[index])]
        if kind[0] == 'alloc':
            counter['alloc_ir'] += cost
            if kind[1] is not None:
                counter[kind[1]] += calls[(a, b)]
        else:
            counter[f'{kind[0]}_ir'] += cost; counter[f'{kind[0]}_calls'] += calls[(a, b)]
    return shares


def measure(path, symbol):
    names, edge, calls = parse(path)
    ids = [k for k, v in names.items() if v == symbol]
    if len(ids) != 1:
        return None
    root = ids[0]
    ncalls = sum(c for (a, b), c in calls.items() if b == root)
    ir = sum(c for (a, b), c in edge.items() if b == root)
    if ir == 0:
        return None
    s = share(names, edge, calls, lambda caller: caller if caller == symbol else None)[symbol]
    return {'path': str(path), 'ir': ir, 'ncalls': ncalls, 'alloc_ir': s['alloc_ir'], 'mem_ir': s['mem_ir'],
            'mem_calls': s['mem_calls'], 'rust_ir': ir - s['alloc_ir'] - s['mem_ir'], **{k: s[k] for k in KINDS}}


def function_options(roots, sync=(), before=(), marker=None):
    """The Callgrind options that key every subtracted function by its caller
    chain and cut a dump at every return of a root (`roots`), at every entry of
    a root (`before`) and at every return of `marker`; each names one function."""
    named = [(f'--separate-callers{CALLERS}', pattern) for pattern in [p for p, _ in ALLOCATOR] + MEMORY + list(sync)]
    named += [('--dump-after', symbol) for symbol in roots]
    named += [('--dump-before', symbol) for symbol in before]
    if marker is not None:
        named.append(('--dump-after', marker))
    names = [name for _, name in named]
    assert len(set(names)) == len(names), ('Callgrind applies only the last option naming a function', names)
    return [f'{option}={name}' for option, name in named]


# Callgrind's function-pattern trie (callgrind/clo.c), whose child lists are
# hashed by first character modulo this degree.
TRIE_DEGREE = 30


class TrieNode:
    """One `config_node` of Callgrind's function-pattern trie."""

    def __init__(self, name):
        self.name = name; self.flags = None; self.next = None; self.star = None; self.char = None
        self.sub = [None] * TRIE_DEGREE


def literal_end(name, start):
    end = start
    while end < len(name) and name[end] not in '*?':
        end += 1
    return end


def bucket(name):
    return ord(name[0]) % TRIE_DEGREE if name else 0


class CallgrindPatterns:
    """The function options of a Callgrind command line, matched the way Callgrind matches them.

    A port of clo.c's `get_fnc2` (insertion, in command-line order) and
    `update_fn_config2` (lookup). Its matcher descends into only the first
    child whose text prefixes the name, while insertion files a pattern that
    continues past a whole node as a new first child of that node without
    merging it with a child of the same text, and reads a `*` right after such
    a node as a literal character. So an option can match a name by its glob
    and still never reach that function; `options_reaching` says which do.
    """

    def __init__(self, options):
        self.root = TrieNode('')
        for option in options:
            flag, _, pattern = option.partition('=')
            if flag.startswith('--separate-callers') or flag in ('--dump-before', '--dump-after'):
                self.insert(self.root, pattern).add(flag)

    def insert(self, node, name):
        if not name:
            if node.flags is None:
                node.flags = set()
            return node.flags
        if name[0] in '*?':
            if name[0] == '*':
                while name[1:2] == '*':
                    name = name[1:]
                if node.star is None:
                    node.star = TrieNode(name[0])
                return self.insert(node.star, name[1:])
            if node.char is None:
                node.char = TrieNode(name[0])
            return self.insert(node.char, name[1:])
        previous = None; child = node.sub[bucket(name)]; common = 0
        while child is not None:
            common = len(os.path.commonprefix([name, child.name]))
            if common:
                break
            previous = child; child = child.next
        if child is None:
            end = literal_end(name, 1)
            return self.add_child(node, name, end)
        if common < len(child.name):
            # Split the child at the common text; clo.c leaves the moved
            # child's sibling link as it was, so its old siblings stay
            # reachable behind it.
            split = TrieNode(child.name[:common])
            if previous is None:
                node.sub[bucket(child.name)] = split
            else:
                previous.next = split
            split.next = child.next
            split.sub[bucket(child.name[common:])] = child
            child.name = child.name[common:]
            name = name[common:]
            return self.add_child(split, name, literal_end(name, 0))
        name = name[len(child.name):]
        if not name:
            if child.flags is None:
                child.flags = set()
            return child.flags
        return self.add_child(child, name, literal_end(name, 1))

    def add_child(self, node, name, end):
        child = TrieNode(name[:end]); child.next = node.sub[bucket(name)]; node.sub[bucket(name)] = child
        if end == len(name):
            child.flags = set()
            return child.flags
        return self.insert(child, name[end:])

    def options_reaching(self, function):
        """The option flags Callgrind applies to `function`."""
        flags = set()
        self.lookup(self.root, function, flags)
        return flags

    def lookup(self, node, name, flags):
        if not name and node.flags is not None:
            flags.update(node.flags)
            return
        child = node.sub[bucket(name)]
        while child is not None and not name.startswith(child.name):
            child = child.next
        if child is not None:
            self.lookup(child, name[len(child.name):], flags)
        if node.char is not None and name:
            self.lookup(node.char, name[1:], flags)
        if node.star is not None:
            for start in range(len(name) + 1):
                self.lookup(node.star, name[start:], flags)


def unseparated(command, functions, sync=()):
    """The subtracted functions among `functions` that `command` does not key by caller chain in Callgrind."""
    patterns = CallgrindPatterns(command)
    flag = f'--separate-callers{CALLERS}'
    return sorted(f for f in functions if subtracted(f, sync) is not None and flag not in patterns.options_reaching(f))


def dump_functions(out, prefix):
    """Every function a profile's dumps name, without its caller chain."""
    functions = set()
    for path in out.glob(f'{prefix}.*'):
        if path.suffix[1:].isdigit():
            with path.open() as dump:
                for line in dump:
                    if line.startswith(('fn=(', 'cfn=(')) and ') ' in line:
                        functions.add(line.split(') ', 1)[1].rstrip('\n').split("'", 1)[0])
    return functions


def check_roots(command, roots, prefix):
    """Fails unless Callgrind applies to each root exactly the dump option it maps to.

    A root keyed by caller chain would lose its plain name, and one its
    trigger misses would merge two calls' dumps.
    """
    patterns = CallgrindPatterns(command)
    wrong = {root: sorted(patterns.options_reaching(root)) for root, flag in roots.items()
             if patterns.options_reaching(root) != {flag}}
    if wrong:
        raise RuntimeError(f'{prefix}: Callgrind would apply other options than the dump trigger to roots: {wrong}')


def check_separated(command, out, prefix, sync=()):
    """Fails the run when Callgrind did not key a subtracted function it ran by caller chain.

    Every share relies on it: a subtracted call without a caller chain is
    taken to have been entered on an empty call stack, outside every root.
    """
    missing = unseparated(command, dump_functions(out, prefix), sync)
    if missing:
        raise RuntimeError(f'{prefix}: Callgrind did not separate {missing} by caller chain: '
                           'its pattern trie never reaches them from the options; reshape the patterns')


def call_samples(out, prefix, symbol, after, count=3, allow_inflight=True, workers=()):
    """The root's first `count` calls, each merged over the dumps of its span.

    One call is the span after the root's previous --dump-after up to its
    next one, holding exactly one incoming call of it. Nested roots and the
    other roots cut dumps inside the span, so the root's incoming edge and
    its allocator and memory-primitive calls are spread over all of them.
    """
    samples = []; prev = 0
    for index, number in enumerate(sorted(after)):
        span = [out / f'{prefix}.{n}' for n in range(prev + 1, number + 1)]
        prev = number
        found = [m for m in (measure(p, symbol) for p in span) if m]
        ncalls = sum(m['ncalls'] for m in found)
        if allow_inflight and index == 0 and ncalls == 0:
            # The call in flight when instrumentation switched on was
            # entered unseen and has no incoming edge: not a sample. Only
            # the first span can be one; skipping a later one would shift
            # every sample to a later frame.
            continue
        assert ncalls == 1, (prefix, symbol, number, [m['ncalls'] for m in found], 'a span must hold exactly one call')
        merged = {k: sum(m[k] for m in found)
                  for k in ('ir', 'ncalls', 'alloc_ir', 'mem_ir', 'mem_calls', 'rust_ir', *KINDS)}
        worker_samples = [m for worker in workers for p in span if (m := measure(p, worker))]
        for m in worker_samples:
            for key in ('ir', 'alloc_ir', 'mem_ir', 'mem_calls', 'rust_ir', *KINDS):
                merged[key] += m[key]
        if workers:
            merged['worker_calls'] = sum(m['ncalls'] for m in worker_samples)
        merged['path'] = str(span[-1])
        samples.append(merged)
        if len(samples) == count:
            break
    return samples


class CpuRoots(typing.NamedTuple):
    """The CPU leg's symbols in one binary."""
    marker: str
    render: str
    jobs: tuple

    def root(self, function):
        """`function` when it is one of the leg's roots (a pool job or the render thread's message handler)."""
        return function if function == self.render or fnmatch.fnmatchcase(function, CPU_JOB) else None


def frame_samples(out, prefix, triggers, roots, frames=CPU_SAMPLES):
    """The CPU backend's work per bench frame: frames 1..`frames` after the pause.

    `triggers` maps each dump number to its (`before`|`after`, function). A
    frame is the span after the previous `submit` dump up to its own, and a
    root call belongs to the frame holding its `--dump-before` dump.

    A dump writes, on every thread, the Ir each call ran since the previous
    dump, whether or not it has returned, so a call's cost is spread over the
    dumps from its entry to its return. The call count is no guide to which
    call that is: it rides on the first nonzero write after the entry, and
    calls from one call site share that write. The windows are: calls of one
    root function run one after another (one render thread; one worker per
    pool, and each job type belongs to one pool), so the root's Ir in the
    dumps after one entry up to the next entry of the same function is that
    call's, and so are the subtracted calls under it in those dumps.
    """
    numbers = sorted(triggers)
    span = {}; frame = 1
    for number in numbers:
        span[number] = frame
        if triggers[number] == ('after', roots.marker):
            frame += 1
    assert frame > frames + 1, (prefix, f'{frame - 1} frames closed; {frames + 1} are needed')
    entries = collections.defaultdict(list); root_ir = {}; root_calls = {}; windows = {}
    for number in numbers:
        names, edge, ncalls = parse(out / f'{prefix}.{number}')
        root_ir[number] = collections.Counter(); root_calls[number] = collections.Counter()
        for (a, b), cost in edge.items():
            function, callers = context(names[b])
            if callers or roots.root(function) is None:
                continue
            if function != roots.render:
                caller = context(names[a])[0]
                assert CPU_POOL in caller and roots.root(caller) is None, (
                    prefix, number, names[a], function, 'a job ran inside another: the CPU leg needs one worker per pool')
            root_ir[number][function] += cost; root_calls[number][function] += ncalls[(a, b)]
        windows[number] = share(names, edge, ncalls, roots.root, SYNC)
        kind, function = triggers[number]
        if kind == 'before':
            assert roots.root(function) is not None, (prefix, number, function, 'dump-before on a non-root')
            entries[function].append(number)
    # A call that reached no `--dump-before`, or a trigger without a call,
    # would merge or split windows: the counts written up to each dump never
    # exceed the entries up to it, and fall short by at most the one entry
    # whose cost is not written yet.
    for function in set(entries) | {f for n in numbers for f in root_calls[n]}:
        written = 0; entered = 0
        for number in numbers:
            entered += triggers[number] == ('before', function)
            written += root_calls[number][function]
            assert written <= entered, (prefix, number, function, written, entered, 'a call entered without its dump')
        assert written >= entered - 1, (prefix, function, written, entered, 'a dump without its call')
    calls = []
    for function, starts in entries.items():
        for index, entry in enumerate(starts):
            last = starts[index + 1] if index + 1 < len(starts) else numbers[-1]
            window = [n for n in numbers if entry < n <= last]
            if index + 1 == len(starts) and span[entry] <= frames:
                # No later entry closes it: it must have finished before the
                # last dump, which then holds none of its Ir.
                assert not root_ir[numbers[-1]][function], (prefix, function, entry, 'a sampled call still running at the last dump')
            calls.append({'function': function, 'entry': entry, 'window': window,
                          'ir': sum(root_ir[n][function] for n in window)})
    samples = []
    for index in range(1, frames + 1):
        frame_calls = [c for c in calls if span[c['entry']] == index]
        sample = collections.Counter({k: 0 for k in ('ir', 'alloc_ir', 'mem_ir', 'mem_calls', 'sync_ir', 'sync_calls', *KINDS)})
        for call in frame_calls:
            assert call['ir'] > 0, (prefix, index, call, 'a sampled call ran no instructions')
            sample['ir'] += call['ir']
            for number in call['window']:
                sample.update(windows[number].get(call['function'], {}))
        sample = dict(sample)
        sample['jobs'] = sum(1 for c in frame_calls if c['function'] != roots.render)
        sample['messages'] = sum(1 for c in frame_calls if c['function'] == roots.render)
        assert sample['jobs'] and sample['messages'], (prefix, index, sample, 'a sampled frame ran no raster job')
        sample['rust_ir'] = sample['ir'] - sample['alloc_ir'] - sample['mem_ir'] - sample['sync_ir']
        sample['path'] = f'{prefix}.{max(c["window"][-1] for c in frame_calls)}'
        samples.append(sample)
    return samples


LAYER_NAME = 'VK_LAYER_CHERENKOV_no_raster'


def install_layer(layer_dir):
    """Builds ir_no_raster.c into `layer_dir` and writes its manifest; returns the library path.

    The manifest names the layer's entrypoints, so the library exports no Vulkan symbols.
    """
    source = pathlib.Path(__file__).with_name('ir_no_raster.c')
    digest = hashlib.sha256(source.read_bytes()).hexdigest()[:16]
    library = layer_dir / f'libVkLayerCherenkovNoRaster-{digest}.so'
    tmp = layer_dir / f'{library.name}.{os.getpid()}.tmp'
    subprocess.run(['cc', '-shared', '-fPIC', '-O2', '-Wall', '-Wextra', '-Werror', '-o', str(tmp), str(source)],
                   check=True)
    os.replace(tmp, library)
    (layer_dir / 'VkLayer_cherenkov_no_raster.json').write_text(json.dumps({
        'file_format_version': '1.0.0',
        'layer': {'name': LAYER_NAME, 'type': 'GLOBAL', 'library_path': str(library),
                  'functions': {'vkGetInstanceProcAddr': 'cherenkovGetInstanceProcAddr',
                                'vkGetDeviceProcAddr': 'cherenkovGetDeviceProcAddr'},
                  'api_version': '1.3.0', 'implementation_version': '1',
                  'description': 'No-op GPU-work vkCmd* entry points (gate only)'}}, indent=1) + '\n')
    return library


def icd_directories(environ=os.environ):
    """The directories the Vulkan loader reads driver manifests from on Linux, in its order.

    The XDG variables fall back to their specified defaults; the loader's
    build-time system configuration directory is /etc on every distribution.
    """
    home = pathlib.Path(environ.get('HOME', '~')).expanduser()
    config = [environ.get('XDG_CONFIG_HOME') or str(home / '.config'),
              *(environ.get('XDG_CONFIG_DIRS') or '/etc/xdg').split(':'), '/etc']
    data = [environ.get('XDG_DATA_HOME') or str(home / '.local' / 'share'),
            *(environ.get('XDG_DATA_DIRS') or '/usr/local/share:/usr/share').split(':')]
    return [pathlib.Path(root) / 'vulkan' / 'icd.d' for root in config + data if root]


def lavapipe_icd(requested=None, directories=None):
    """Select the one registered Lavapipe driver by its library, whatever the manifest is called."""
    if requested is not None:
        candidates = [pathlib.Path(requested)]
    else:
        candidates = sorted({path.resolve() for directory in (directories or icd_directories())
                             for path in directory.glob('*.json')})
    matches = [path for path in candidates
               if pathlib.Path(json.loads(path.read_text())['ICD']['library_path']).name.startswith('libvulkan_lvp.so')]
    if len(matches) != 1:
        raise RuntimeError(f'Expected one Lavapipe ICD, found {len(matches)} in {candidates}. '
                           'Install Mesa lavapipe or specify its manifest with --icd.')
    return matches[0].resolve(strict=True)


def layer_env(layer_dir, icd):
    """The environment that runs Lavapipe with the layer in `layer_dir` enabled."""
    return dict(VK_DRIVER_FILES=str(icd), VK_LAYER_PATH=str(layer_dir),
                VK_INSTANCE_LAYERS=LAYER_NAME, NODEVICE_SELECT='1')


def sampled(after, phases, label, returns=4):
    """Whether every root has returned `returns` times. Steady profiles allow
    one in-flight call before three samples; fresh profiles require one return.

    `after` holds every root's dump numbers. A root outside `phases` that
    returns, or a declared root still short after STALL_FRAMES frames, fails.
    """
    if undeclared := [phase for phase, dumps in after.items() if dumps and phase not in phases]:
        raise RuntimeError(f'{label}: {undeclared} run in steady state but are not among the scene\'s roots '
                           f'{phases}; declare them in SCENES.')
    done = all(len(after[phase]) >= returns for phase in phases)
    if not done and len(after[FRAME_ROOT]) > STALL_FRAMES:
        raise RuntimeError(f'{label}: after {len(after[FRAME_ROOT])} frames the roots returned '
                           f'{ {phase: len(after[phase]) for phase in phases} } times; a declared root does not '
                           'run every frame.')
    return done


def triggered(triggers, kind, symbol):
    """The dump numbers `--dump-<kind>=symbol` cut, in order."""
    return [number for number in sorted(triggers) if triggers[number] == (kind, symbol)]


def symbols_of(binary):
    return [line.split()[-1] for line in
            subprocess.run(['nm', '--defined-only', str(binary)], capture_output=True, text=True, check=True).stdout.splitlines()]


def one_symbol(symbols, suffix, parts, tag, what):
    hits = [s for s in symbols if s.endswith(suffix) and all(part in s for part in parts)]
    if not hits:
        # A generic function's symbol keeps the fn name inside its own
        # path, ahead of the `NC`-delimited instantiation suffix
        # (`…Shared<Gpu>E15run_transactionNC<closure>`), or ahead of its
        # generic arguments (`…13apply_messageNtCs…_13cherenkov_cpu6RasterE…`):
        # match the name there instead.
        hits = list(dict.fromkeys(s for s in symbols if suffix in s.split('NC', 1)[0] and all(part in s for part in parts)))
    if len(hits) != 1:
        raise RuntimeError(f'{tag}: expected one {what} symbol, found {hits}. '
                           'Build with CARGO_PROFILE_RELEASE_DEBUG=1 and CARGO_PROFILE_RELEASE_STRIP=false.')
    return hits[0]


def cpu_roots(symbols, tag):
    """The CPU leg's symbols in a binary; none of its roots may be a subtracted function."""
    roots = CpuRoots(marker=one_symbol(symbols, *CPU_MARKER, tag, 'submit'),
                     render=one_symbol(symbols, *CPU_RENDER, tag, 'apply_message'),
                     jobs=tuple(s for s in symbols if fnmatch.fnmatchcase(s, CPU_JOB)))
    if not roots.jobs:
        raise RuntimeError(f'{tag}: no rayon job entry matches {CPU_JOB}')
    for symbol in (roots.marker, roots.render, *roots.jobs):
        if subtracted(symbol, SYNC) is not None:
            raise RuntimeError(f'{tag}: {symbol} is both a root and a subtracted function')
    return roots


def run_callgrind(command, env, repo, out, prefix, pause_at, label, done, deadline):
    """Runs `command` under Callgrind until `done(triggers)`; returns every dump's trigger.

    `triggers` maps a dump number to its (`before`|`after`, function). The
    bench pauses before frame `pause_at`; instrumentation goes on there.
    """
    triggers = {}
    monitor = selectors.DefaultSelector(); fds = []
    watch = LIBC.inotify_init1(os.O_CLOEXEC)
    if watch < 0:
        raise OSError(ctypes.get_errno(), 'inotify_init1')
    assert LIBC.inotify_add_watch(watch, os.fsencode(out), 0x8 | 0x80) >= 0
    fds.append(watch); monitor.register(watch, selectors.EVENT_READ, 'files')
    expiry = timer(deadline); fds.append(expiry); monitor.register(expiry, selectors.EVENT_READ, 'deadline')
    with open(str(prefix) + '.log', 'w') as log:
        process = subprocess.Popen(command, cwd=repo, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, start_new_session=True)
        pidfd = os.pidfd_open(process.pid); fds.append(pidfd); monitor.register(pidfd, selectors.EVENT_READ, 'exit')
        monitor.register(process.stdout, selectors.EVENT_READ, 'stdout')
        pending = b''
        print(label, 'pid', process.pid, 'running to frame', pause_at, flush=True)

        try:
            while not done(triggers):
                for event, _ in monitor.select():
                    if event.data == 'stdout':
                        chunk = os.read(process.stdout.fileno(), 65536)
                        log.write(chunk.decode(errors='replace')); log.flush()
                        pending += chunk
                        while b'\n' in pending:
                            line, pending = pending.split(b'\n', 1)
                            if line.decode().strip() == PAUSE.format(pause_at):
                                subprocess.run(['callgrind_control', '-i', 'on', str(process.pid)], stdout=log, stderr=log, check=True, timeout=120)
                                process.stdin.write(b'\n'); process.stdin.flush()
                                print(label, 'instrumentation on at frame', pause_at, flush=True)
                    elif event.data == 'deadline':
                        raise TimeoutError((label, collections.Counter(triggers.values())))
                    elif event.data == 'exit':
                        raise RuntimeError((label, 'exited early', process.wait()))
                    else:
                        data = os.read(watch, 65536); offset = 0
                        while offset < len(data):
                            _, _, _, size = struct.unpack_from('iIII', data, offset); offset += 16
                            name = os.fsdecode(data[offset:offset + size].split(b'\0', 1)[0]); offset += size
                            match = re.fullmatch(re.escape(prefix.name) + r'\.(\d+)', name)
                            if not match or int(match[1]) in triggers:
                                continue
                            head = (out / name).read_text().split('\n\nob=', 1)[0]
                            trigger = re.search(r'^desc: Trigger: --dump-(before|after)=(.*)$', head, re.M)
                            assert trigger, (label, name, 'a dump without a function trigger')
                            triggers[int(match[1])] = (trigger[1], trigger[2])
        finally:
            process.terminate()
            try:
                process.wait(timeout=60)
            except subprocess.TimeoutExpired:
                process.kill(); process.wait()
            monitor.close()
            for fd in fds:
                os.close(fd)
    return triggers


def profile(binary, tag, scene, repo, out, pause_at, warmup, icd=None, phases=None, deadline=900, leg=LEGS['gpu'], fresh=False):
    """Profiles `scene` and samples each root in `phases` (the scene's declared roots by default)."""
    if fresh:
        assert leg.name == 'gpu' and pause_at == 0, 'fresh preparation starts before GPU frame zero'
    if leg.name == 'cpu':
        return profile_cpu(binary, tag, scene, repo, out, pause_at, warmup)
    phases = SCENES[scene] if phases is None else phases
    assert FRAME_ROOT in phases, (scene, phases, f'{FRAME_ROOT} runs every frame')
    binary = pathlib.Path(binary).resolve(); out = pathlib.Path(out).resolve(); out.mkdir(parents=True, exist_ok=True)
    scene_dir = (pathlib.Path(repo) / 'scenes' / 'perf' / scene).resolve()
    assert scene_dir.is_dir(), f'no perf scene at {scene_dir}; generate it with scenes/tools/generate.py'
    icd = lavapipe_icd(icd)
    prefix = out / f'{tag}-{scene}'
    assert not [p for p in out.glob(prefix.name + '.*') if not p.name.endswith('.progress.log')], f'existing run: {prefix}'
    symbols = symbols_of(binary)
    roots = {phase: one_symbol(symbols, suffix, parts, tag, phase) for phase, (suffix, parts) in ROOTS.items()}
    # The joined geometry batch finishes before lower_content returns. With
    # one pool worker its entire computation is below this worker root.
    workers = tuple(s for s in symbols if s.endswith('22prepare_geometry_batch') and 'cherenkov_gpu' in s)
    # Each profile owns its loader files; parallel scenes must not rewrite
    # a manifest while another process is loading it.
    layer_dir = out / (prefix.name + '.vk_layer')
    layer_dir.mkdir(exist_ok=True)
    install_layer(layer_dir)
    runtime_dir = out / (prefix.name + '.runtime')
    runtime_dir.mkdir(mode=0o700, exist_ok=True)
    env = os.environ.copy()
    env.pop('VK_ICD_FILENAMES', None)
    env.update(layer_env(layer_dir, icd), XDG_RUNTIME_DIR=str(runtime_dir), RUST_LOG='error',
               RAYON_NUM_THREADS=CPU_THREADS)
    command = ['valgrind', '--tool=callgrind', '--demangle=no', '--instr-atstart=no', '--separate-threads=no',
               *function_options(roots.values()),
               f'--callgrind-out-file={prefix}', str(binary), 'measure', '--engine', 'cherenkov',
               '--scene', str(scene_dir), '--frames', '100000', '--warmup', str(warmup),
               '--pause-at', str(pause_at), '--out', str(prefix) + '.unused-timing.json']
    check_roots(command, {symbol: '--dump-after' for symbol in roots.values()}, prefix.name)

    def after_of(triggers):
        return {phase: triggered(triggers, 'after', symbol) for phase, symbol in roots.items()}
    triggers = run_callgrind(command, env, repo, out, prefix, pause_at, f'{tag} {scene}',
                             lambda triggers: sampled(after_of(triggers), phases, f'{tag} {scene}',
                                                      returns=1 if fresh else 4), deadline)
    after = after_of(triggers)
    check_separated(command, out, prefix.name)
    result = {'tag': tag, 'scene': scene, 'leg': leg.name, 'binary': str(binary),
              'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'icd': str(icd), 'icd_sha256': hashlib.sha256(icd.read_bytes()).hexdigest(),
              'pause_at': pause_at, 'command': command, 'worker_roots': workers,
              'sample_mode': 'fresh' if fresh else 'retained'}
    for phase in phases:
        symbol = roots[phase]
        samples = call_samples(out, prefix.name, symbol, after[phase],
                               count=1 if fresh else 3, allow_inflight=not fresh,
                               workers=workers if phase == 'lower' else ())
        if fresh:
            assert len(samples) == 1, (tag, scene, phase, len(samples))
            result[phase] = {'root': symbol, 'first': samples[0]}
            print(tag, scene, phase, 'FIRST', samples[0], flush=True)
            continue
        assert len(samples) == 3, (tag, scene, phase, len(samples))
        second, third = samples[1], samples[2]
        result[phase] = {'root': symbol, 'first': samples[0], 'second': second, 'third': third,
                         'third_delta_percent': 100 * (third['rust_ir'] / second['rust_ir'] - 1)}
        print(tag, scene, phase, 'SECOND', second['ir'], 'rust', second['rust_ir'], 'allocs', second['allocs'],
              'reallocs', second['reallocs'], 'deallocs', second['deallocs'], 'mem', second['mem_calls'], 'THIRD rust', third['rust_ir'], flush=True)
    pathlib.Path(str(prefix) + '.result.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


def cpu_env():
    """The CPU leg's environment: one worker per rayon pool, errors-only logging."""
    return dict(os.environ, RAYON_NUM_THREADS=CPU_THREADS, RUST_LOG='error')


def check_threads(binary, scene_dir, out, prefix):
    """Fails unless the backend runs its pool at CPU_THREADS workers under the leg's environment."""
    report = pathlib.Path(str(prefix) + '.threads.json')
    subprocess.run([str(binary), 'measure', '--engine', 'cherenkov-cpu', '--scene', str(scene_dir), '--frames', '1',
                    '--warmup', '0', '--out', str(report)], env=cpu_env(), check=True, timeout=600,
                   stdout=subprocess.DEVNULL)
    adapter = json.loads(report.read_text())['device']['adapter']
    if adapter != CPU_ADAPTER:
        raise RuntimeError(f'{prefix.name}: the CPU backend reports {adapter!r}, the leg needs {CPU_ADAPTER!r}')
    return adapter


def profile_cpu(binary, tag, scene, repo, out, pause_at, warmup, deadline=1800):
    """Profiles `scene` on the CPU backend and samples its frames."""
    binary = pathlib.Path(binary).resolve(); out = pathlib.Path(out).resolve(); out.mkdir(parents=True, exist_ok=True)
    scene_dir = (pathlib.Path(repo) / 'scenes' / 'perf' / scene).resolve()
    assert scene_dir.is_dir(), f'no perf scene at {scene_dir}; generate it with scenes/tools/generate.py'
    prefix = out / f'{tag}-{scene}'
    assert not [p for p in out.glob(prefix.name + '.*') if not p.name.endswith('.progress.log')], f'existing run: {prefix}'
    roots = cpu_roots(symbols_of(binary), tag)
    adapter = check_threads(binary, scene_dir, out, prefix)
    command = ['valgrind', '--tool=callgrind', '--demangle=no', '--instr-atstart=no', '--separate-threads=no',
               *function_options([], SYNC, before=[roots.render, CPU_JOB], marker=roots.marker),
               f'--callgrind-out-file={prefix}', str(binary), 'measure', '--engine', 'cherenkov-cpu',
               '--scene', str(scene_dir), '--frames', '100000', '--warmup', str(warmup),
               '--pause-at', str(pause_at), '--out', str(prefix) + '.unused-timing.json']
    check_roots(command, {roots.marker: '--dump-after', roots.render: '--dump-before',
                          **{job: '--dump-before' for job in roots.jobs}}, prefix.name)
    label = f'{tag} {scene}'
    triggers = run_callgrind(command, cpu_env(), repo, out, prefix, pause_at, label,
                             lambda triggers: len(triggered(triggers, 'after', roots.marker)) > CPU_SAMPLES, deadline)
    check_separated(command, out, prefix.name, SYNC)
    samples = frame_samples(out, prefix.name, triggers, roots)
    second, third = samples[1], samples[2]
    result = {'tag': tag, 'scene': scene, 'leg': 'cpu', 'binary': str(binary),
              'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'adapter': adapter,
              'roots': roots._asdict(), 'pause_at': pause_at, 'command': command,
              CPU_FRAME: {'first': samples[0], 'second': second, 'third': third,
                          'third_delta_percent': 100 * (third['rust_ir'] / second['rust_ir'] - 1)}}
    print(label, CPU_FRAME, 'SECOND', second['ir'], 'rust', second['rust_ir'], 'allocs', second['allocs'],
          'reallocs', second['reallocs'], 'deallocs', second['deallocs'], 'mem', second['mem_calls'],
          'sync', second['sync_calls'], 'jobs', second['jobs'], 'THIRD rust', third['rust_ir'], flush=True)
    pathlib.Path(str(prefix) + '.result.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


def load(out, tag, scene):
    # `batch` writes each tag into its own subdirectory (--out per tag); a
    # flat out dir with the files side by side is accepted too.
    for root in (pathlib.Path(out) / tag, pathlib.Path(out)):
        candidate = root / f'{tag}-{scene}.result.json'
        if candidate.exists():
            return json.loads(candidate.read_text())
    raise FileNotFoundError(f'no result for {tag}/{scene} under {out}')


def verify_reused_profile(result, binary, icd):
    """A resumed profile must belong to the requested executable and driver."""
    assert result['sha256'] == hashlib.sha256(pathlib.Path(binary).read_bytes()).hexdigest()
    if icd is not None:
        assert result['icd_sha256'] == hashlib.sha256(icd.read_bytes()).hexdigest()


def batch(binary, tag, repo, out, pause_at, warmup, jobs, icd=None, leg=LEGS['gpu']):
    """Profiles every scene, `jobs` at a time, and writes the tag's report."""
    out = pathlib.Path(out); out.mkdir(parents=True, exist_ok=True)
    icd = lavapipe_icd(icd) if leg.name == 'gpu' else None

    def task(scene):
        path = out / f'{tag}-{scene}.result.json'
        if path.is_file():
            verify_reused_profile(json.loads(path.read_text()), binary, icd)
            return f'{tag} {scene} reused completed profile'
        progress = out / f'{tag}-{scene}.progress.log'
        with progress.open('w') as log:
            status = subprocess.run([sys.executable, __file__, 'profile', '--leg', leg.name, '--binary', str(binary),
                                     '--tag', tag, '--scene', scene, '--repo', str(repo), '--out', str(out),
                                     '--pause-at', str(pause_at), '--warmup', str(warmup),
                                     *(['--icd', str(icd)] if icd is not None else [])],
                                    stdout=log, stderr=subprocess.STDOUT).returncode
        if status:
            raise RuntimeError(f'{tag} {scene}: profile exited with {status}\n{progress.read_text()}')
        return f'{tag} {scene} done'
    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
        for line in pool.map(task, leg.scenes):
            print(line, flush=True)
    report = tag_report(out, tag, pause_at, leg)
    (out / f'{tag}-ir.md').write_text(report); print(report)


def tag_report(out, tag, pause_at, leg):
    """One tag's per-scene report at the second steady frame."""
    cpu = leg.name == 'cpu'
    lines = [f'# {leg.title}: Ir at the second steady frame: `{tag}`', '',
             f'Instrumentation on at frame {pause_at}; sample 2 of each root. Rust Ir excludes the allocator'
             + (', the libc memory primitives and cross-thread synchronization.' if cpu else ' and the libc memory primitives.'), '',
             '| Scene | Phase | Ir | Allocator Ir | Mem Ir |' + (' Sync Ir |' if cpu else '')
             + ' Rust Ir | allocs | reallocs | deallocs | mem calls |' + (' jobs |' if cpu else '') + ' Rust Ir 2nd → 3rd |',
             '|---|---|---:|---:|---:|' + ('---:|' if cpu else '') + '---:|---:|---:|---:|---:|' + ('---:|' if cpu else '') + '---:|']
    for scene, phases in leg.scenes.items():
        d = load(out, tag, scene)
        for phase in phases:
            s, t = d[phase]['second'], d[phase]['third']
            lines.append(f"| {scene} | {phase} | {s['ir']:,} | {s['alloc_ir']:,} | {s['mem_ir']:,} | "
                         + (f"{s['sync_ir']:,} | " if cpu else '')
                         + f"{s['rust_ir']:,} | {s['allocs']} | {s['reallocs']} | {s['deallocs']} | {s['mem_calls']} | "
                         + (f"{s['jobs']} | " if cpu else '')
                         + f"{t['rust_ir']:,} ({d[phase]['third_delta_percent']:+.3f}%) |")
    return '\n'.join(lines) + '\n'


def compare(out, base, head, leg=LEGS['gpu']):
    """The gate's verdict on `head` against `base`: the report and whether it failed."""
    lines = [f'# {leg.title}: Rust Ir and allocator calls: `{base}` → `{head}`', '',
             'Second steady frame. Fails when Rust Ir rises more than 1% or any allocator or memory-primitive call count rises.', '',
             '| Scene | Phase | Rust Ir base | Rust Ir head | Δ | allocs | reallocs | deallocs | mem calls | |', '|---|---|---:|---:|---:|---:|---:|---:|---:|---|']
    failed = False
    for scene, phases in leg.scenes.items():
        a, b = load(out, base, scene), load(out, head, scene)
        for phase in phases:
            x, y = a[phase]['second'], b[phase]['second']
            delta = 100 * (y['rust_ir'] / x['rust_ir'] - 1)
            ok = delta <= 1 and all(y[k] <= x[k] for k in ('allocs', 'reallocs', 'deallocs', 'mem_calls'))
            failed |= not ok
            counts = [f"{x[k]} → {y[k]}" for k in ('allocs', 'reallocs', 'deallocs', 'mem_calls')]
            lines.append(f"| {scene} | {phase} | {x['rust_ir']:,} | {y['rust_ir']:,} | {delta:+.3f}% | {' | '.join(counts)} | {'ok' if ok else 'FAIL'} |")
    return '\n'.join(lines) + '\n', failed


def run(command, **kwargs):
    print('+', ' '.join(map(str, command)), flush=True)
    return subprocess.run(command, check=True, **kwargs)


def output(command, **kwargs):
    return subprocess.run(command, check=True, capture_output=True, text=True, **kwargs).stdout.strip()


def commit(checkout, revision):
    """`revision` as a full commit SHA; fetched from origin when the checkout does not hold it."""
    found = subprocess.run(['git', 'rev-parse', '--verify', '--quiet', '--end-of-options', f'{revision}^{{commit}}'],
                           cwd=checkout, capture_output=True, text=True)
    if found.returncode == 0:
        return found.stdout.strip()
    run(['git', 'fetch', '--no-tags', 'origin', revision], cwd=checkout)
    return output(['git', 'rev-parse', '--verify', 'FETCH_HEAD^{commit}'], cwd=checkout)


def executable(messages):
    """The one executable in Cargo's JSON build messages."""
    found = [m['executable'] for m in map(json.loads, messages.splitlines())
             if m.get('reason') == 'compiler-artifact' and m.get('executable')]
    if len(found) != 1:
        raise RuntimeError(f'expected one executable from cargo, got {found}')
    return pathlib.Path(found[0])


def build_bench(tree, target_dir, destination, leg):
    """Builds `tree`'s cherenkov-bench with the leg's settings and copies it to `destination`.

    The tree's compile-time fonts come from its own generator, run with this
    interpreter. Every tree shares one target directory: dependencies build
    once, and each tree's workspace packages keep their own artifacts.
    """
    env = dict(os.environ, CARGO_TARGET_DIR=str(target_dir), **BUILD_ENV)
    run(['uv', 'run', '--python', sys.executable, '--with-requirements', 'scenes/fonts/tools/requirements.txt',
         'python', 'scenes/tools/generate.py', '--fonts-only'], cwd=tree / 'graphics' / 'cherenkov', env=env)
    print('+ cargo build --locked', *build_args(leg), flush=True)
    messages = subprocess.run(['cargo', 'build', '--locked', '--message-format=json-render-diagnostics', *build_args(leg)],
                              cwd=tree, env=env, check=True, stdout=subprocess.PIPE, text=True).stdout
    shutil.copyfile(executable(messages), destination)
    destination.chmod(0o755)


def digests(paths, relative_to):
    return {str(path.relative_to(relative_to)): hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}


def ab(args):
    """Builds the merge base of BASE and HEAD and HEAD, profiles both, and applies the gate."""
    leg = LEGS[args.leg]
    scripts = pathlib.Path(__file__).resolve().parent
    checkout = pathlib.Path(output(['git', 'rev-parse', '--show-toplevel'], cwd=scripts))
    cherenkov = scripts.parents[1]
    perf = cherenkov / 'scenes' / 'perf'
    missing = [scene for scene in leg.scenes if not (perf / scene).is_dir()]
    if missing:
        raise RuntimeError(f'{perf} lacks {missing}; generate it with scenes/tools/generate.py --corpus-only.')
    icd = lavapipe_icd(args.icd) if leg.name == 'gpu' else None
    head = commit(checkout, args.head)
    base = output(['git', 'merge-base', commit(checkout, args.base), head], cwd=checkout)
    work = pathlib.Path(args.work).resolve(); out = pathlib.Path(args.out).resolve()
    work.mkdir(parents=True); out.mkdir(parents=True)
    # Equal-length tree paths keep the source paths the binaries embed, and
    # with them the binaries' layout, the same length on both sides.
    trees = {'base': (base, work / 'base'), 'head': (head, work / 'head')}
    plan = {'leg': leg.name, 'requested': {'base': args.base, 'head': args.head}, 'base': base, 'head': head,
            'harness': output(['git', 'rev-parse', 'HEAD'], cwd=checkout),
            'harness_sha256': digests([scripts / 'ir_gate.py', scripts / 'ir_no_raster.c'], scripts),
            'scenes_sha256': digests(sorted(p for scene in leg.scenes for p in (perf / scene).rglob('*') if p.is_file()), perf),
            'build_env': BUILD_ENV, 'build': build_args(leg), 'pause_at': args.pause_at, 'warmup': args.warmup, 'jobs': args.jobs,
            'machine': platform.machine(), 'cpu': output(['lscpu']), 'rustc': output(['rustc', '-Vv'], cwd=checkout),
            'cargo': output(['cargo', '-V'], cwd=checkout), 'valgrind': output(['valgrind', '--version'])}
    if icd is not None:
        plan.update(icd=str(icd), icd_sha256=hashlib.sha256(icd.read_bytes()).hexdigest())
    else:
        plan['rayon_num_threads'] = CPU_THREADS
    binaries = {}
    for tag, (revision, tree) in trees.items():
        run(['git', 'worktree', 'add', '--detach', str(tree), revision], cwd=checkout)
        binaries[tag] = out / f'bench-{tag}'
        build_bench(tree, work / 'target', binaries[tag], leg)
    plan['binaries_sha256'] = digests(binaries.values(), out)
    (out / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    for tag, binary in binaries.items():
        batch(binary, tag, cherenkov, out / tag, args.pause_at, args.warmup, args.jobs, icd, leg)
    report, failed = compare(out, 'base', 'head', leg)
    (out / 'comparison.md').write_text(report); print(report)
    return 1 if failed else 0


def same(args):
    leg = LEGS[args.leg]
    failed = False
    for scene, phases in leg.scenes.items():
        ds = [load(args.out, tag, scene) for tag in args.tags]
        assert len({d['sha256'] for d in ds}) == 1, (scene, 'different binaries')
        for phase in phases:
            second = [d[phase]['second'] for d in ds]
            rust = [s['rust_ir'] for s in second]
            count = [(s['allocs'], s['reallocs'], s['deallocs'], s['mem_calls']) for s in second]
            ok = len(set(rust)) == 1 and len(set(count)) == 1
            failed |= not ok
            print(f"{scene:10s} {phase:6s} rust {rust} calls {count[0] if len(set(count)) == 1 else count} "
                  f"{'identical' if ok else 'DIFFERENT'}; inclusive {[s['ir'] for s in second]}")
    return 1 if failed else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='cmd', required=True)

    def leg(p):
        p.add_argument('--leg', required=True, choices=LEGS, help='gpu: the GPU engine on Lavapipe; cpu: the CPU backend')

    def sampling(p):
        p.add_argument('--pause-at', type=int, default=8); p.add_argument('--warmup', type=int, default=30)
        p.add_argument('--icd', type=pathlib.Path, help='gpu leg: explicit Lavapipe ICD manifest; otherwise require one installed driver')

    p = sub.add_parser('ab'); leg(p)
    p.add_argument('--base', required=True, help='revision whose merge base with --head is the baseline')
    p.add_argument('--head', required=True, help='candidate revision')
    p.add_argument('--work', required=True, help='new directory for both source trees and their target directory')
    p.add_argument('--out', required=True, help='new directory for binaries, provenance, profiles and the verdict')
    p.add_argument('--jobs', type=int, default=os.cpu_count()); sampling(p)
    for name in ('profile', 'batch'):
        p = sub.add_parser(name); leg(p)
        p.add_argument('--binary', required=True); p.add_argument('--tag', required=True)
        p.add_argument('--repo', required=True, help='checkout directory holding scenes/perf (the cherenkov tree)')
        p.add_argument('--out', required=True); sampling(p)
        if name == 'profile':
            p.add_argument('--scene', required=True, choices=SCENES)
        else:
            p.add_argument('--jobs', type=int, default=os.cpu_count())
    p = sub.add_parser('compare'); leg(p); p.add_argument('--out', required=True); p.add_argument('base'); p.add_argument('head')
    p = sub.add_parser('same'); leg(p); p.add_argument('--out', required=True); p.add_argument('tags', nargs='+')
    args = parser.parse_args()
    if args.cmd == 'ab':
        sys.exit(ab(args))
    elif args.cmd == 'profile':
        profile(args.binary, args.tag, args.scene, args.repo, args.out, args.pause_at, args.warmup, args.icd, leg=LEGS[args.leg])
    elif args.cmd == 'batch':
        batch(args.binary, args.tag, args.repo, args.out, args.pause_at, args.warmup, args.jobs, args.icd, LEGS[args.leg])
    elif args.cmd == 'compare':
        report, failed = compare(args.out, args.base, args.head, LEGS[args.leg])
        print(report, end='')
        sys.exit(1 if failed else 0)
    else:
        sys.exit(same(args))


if __name__ == '__main__':
    main()
