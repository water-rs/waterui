#!/usr/bin/env python3
"""Callgrind Ir gate for `lower`, `encode` and `run_transaction` on the five
perf scenes.

Each profile runs `cherenkov-bench measure --pause-at P` under Callgrind with
instrumentation off. At the pause (a fixed frame index, not wall-clock time)
instrumentation is switched on and the bench resumes. `--dump-after` on each
root cuts a dump at every return. Callgrind keeps one cost table for all
threads (`--separate-threads=no`), so every dump holds every thread's events
since the previous one and the dumps partition the run. One call of a root is
the span between consecutive `--dump-after` triggers of that root, holding
exactly one incoming call of it: a root nested inside it (`run_transaction`
runs inside `encode`) and the other roots write their own dumps within the
span, so a sample sums the root's incoming edge and the allocator calls over
every dump in its span. The k-th call is sample k; sample 2 is the gate's
"second steady frame" and sample 3 its stability check. Only the first span
may hold no complete call — the call in flight when instrumentation switched
on — so any later skip is an error, never a silent shift of which frame is
sampled.

Every Callgrind option names a function at most once. Callgrind 3.18 keeps
function options in a prefix trie whose insertion, once another name has split
a node on the path, adds a second node for a name already present instead of
reusing the first, and a lookup applies only the newest: of `--dump-before=S`
and `--dump-after=S`, only the latter takes effect, so a `--dump-before` on a
root never fires.

Per sample it reports the root's inclusive Ir; the allocator's share of it
(the inclusive cost of every call from other code into the Rust allocator
shims and the libc malloc family) with the number of alloc, realloc and
dealloc calls; the libc memory primitives' share (memcpy, memmove, memset,
memcmp, bcmp), whose Ir depends on buffer addresses the allocator chooses,
with their call count; and the Rust Ir: inclusive Ir minus both shares.

A share counts only calls made under the root, decided by calling context:
`--separate-callers<N>` keys every allocator and memory-primitive function by
the N functions above it on the calling thread's stack, so each call edge into
one names its caller chain, and a call is the root's exactly when the root is
in that chain. A chain shorter than N reached the bottom of the thread's
stack, so a root missing from it is outside the root; a chain of full length
without the root is undecidable and fails the run. Neither the plain call
graph nor per-thread dumps can decide it: the graph merges every caller of a
shared function such as `RawVecInner::deallocate`, and `--separate-threads=yes`
does not isolate threads — after instrumentation switches on mid-run, a
thread's call-stack underflow can attach its calls to a record another thread
owns, so another thread's frees land in the root thread's dumps.

  ir_gate.py profile --binary B --tag T --scene S --repo R --out O
  ir_gate.py batch   --binary B --tag T --repo R --out O
  ir_gate.py compare --out O BASE HEAD      # the +1% / no-more-calls rule
  ir_gate.py same    --out O TAG TAG [TAG]  # identical Ir across re-profiles

For Lavapipe it sets VK_ICD_FILENAMES, XDG_RUNTIME_DIR and RUST_LOG, and it
builds and enables VK_LAYER_CHERENKOV_no_raster (ir_no_raster.c), a global
instance layer that no-ops the GPU-work vkCmd* entry points under Callgrind:
lavapipe rasterization dominates the profile wall time but every vkCmd* call
sits outside the measured lower/encode roots, so skipping it cannot change
the counted Ir. NODEVICE_SELECT disables the MESA device_select layer, whose
teardown crashes when another instance layer sits in front of it.
"""
import argparse, collections, concurrent.futures, ctypes, fnmatch, hashlib, json, os, pathlib, re, selectors, struct, subprocess, sys

SCENES = ['map', 'chart', 'text-page', 'ui-list', 'effects']
ROOTS = {'lower': ('13lower_content', 'cherenkov_gpu'), 'encode': ('6Engine6encode', 'cherenkov_ad'),
         'run_transaction': ('15run_transaction', 'cherenkov_record')}
PAUSE = 'cherenkov-bench: paused before frame {}'
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
KINDS = ('allocs', 'reallocs', 'deallocs')
# Caller-chain depth of every allocator and memory-primitive function: deep
# enough to reach the root from any call under it, or the run fails.
CALLERS = 256
LIBC = ctypes.CDLL(None, use_errno=True)


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


def subtracted(function):
    """('alloc', kind) or ('mem', None) for a function a sample subtracts, else None."""
    for pattern, kind in ALLOCATOR:
        if fnmatch.fnmatchcase(function, pattern):
            return 'alloc', kind
    if any(fnmatch.fnmatchcase(function, pattern) for pattern in MEMORY):
        return 'mem', None
    return None


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
    alloc_ir = mem_ir = mem_calls = 0; counts = collections.Counter({kind: 0 for kind in KINDS})
    for (a, b), cost in edge.items():
        function, callers = context(names[b])
        share = subtracted(function)
        # A call from inside the allocator is part of the call into it.
        if share is None or subtracted(context(names[a])[0]) is not None:
            continue
        assert callers, (path, names[b], 'not separated by caller chain: add it to the Callgrind patterns')
        if symbol not in callers:
            assert len(callers) < CALLERS, (path, names[b], f'caller chain truncated at {CALLERS} without the root')
            continue
        if share[0] == 'alloc':
            alloc_ir += cost
            if share[1] is not None:
                counts[share[1]] += calls[(a, b)]
        else:
            mem_ir += cost; mem_calls += calls[(a, b)]
    return {'path': str(path), 'ir': ir, 'ncalls': ncalls, 'alloc_ir': alloc_ir, 'mem_ir': mem_ir, 'mem_calls': mem_calls,
            'rust_ir': ir - alloc_ir - mem_ir, **counts}


def function_options(roots):
    """The Callgrind options that key every subtracted function by its caller
    chain and cut a dump at every return of a root; each names one function."""
    named = [(f'--separate-callers{CALLERS}', pattern) for pattern in [p for p, _ in ALLOCATOR] + MEMORY]
    named += [('--dump-after', symbol) for symbol in roots]
    names = [name for _, name in named]
    assert len(set(names)) == len(names), ('Callgrind applies only the last option naming a function', names)
    return [f'{option}={name}' for option, name in named]


def call_samples(out, prefix, symbol, after):
    """The root's first three calls, each merged over the dumps of its span.

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
        if index == 0 and ncalls == 0:
            # The call in flight when instrumentation switched on was
            # entered unseen and has no incoming edge: not a sample. Only
            # the first span can be one; skipping a later one would shift
            # every sample to a later frame.
            continue
        assert ncalls == 1, (prefix, symbol, number, [m['ncalls'] for m in found], 'a span must hold exactly one call')
        merged = {k: sum(m[k] for m in found)
                  for k in ('ir', 'ncalls', 'alloc_ir', 'mem_ir', 'mem_calls', 'rust_ir', *KINDS)}
        merged['path'] = str(span[-1])
        samples.append(merged)
        if len(samples) == 3:
            break
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


def layer_env(layer_dir):
    """The environment that runs Lavapipe with the layer in `layer_dir` enabled."""
    return dict(VK_ICD_FILENAMES='/usr/share/vulkan/icd.d/lvp_icd.x86_64.json', VK_LAYER_PATH=str(layer_dir),
                VK_INSTANCE_LAYERS=LAYER_NAME, NODEVICE_SELECT='1')


def profile(binary, tag, scene, repo, out, pause_at, warmup):
    binary = pathlib.Path(binary).resolve(); out = pathlib.Path(out).resolve(); out.mkdir(parents=True, exist_ok=True)
    prefix = out / f'{tag}-{scene}'
    assert not [p for p in out.glob(prefix.name + '.*') if not p.name.endswith('.progress.log')], f'existing run: {prefix}'
    symbols = subprocess.run(['nm', '--defined-only', str(binary)], capture_output=True, text=True, check=True).stdout.splitlines()
    roots = {}
    for phase, (suffix, crate) in ROOTS.items():
        hits = [line.split()[-1] for line in symbols if line.split()[-1].endswith(suffix) and crate in line]
        if not hits:
            # A generic method's symbol keeps the fn name inside its own
            # path, ahead of the `NC`-delimited instantiation suffix
            # (`…Shared<Gpu>E15run_transactionNC<closure>`): match the
            # name there instead.
            hits = [s for s in (line.split()[-1] for line in symbols)
                    if suffix in s.split('NC', 1)[0] and crate in s]
            hits = list(dict.fromkeys(hits))
        assert len(hits) == 1, (tag, phase, hits)
        roots[phase] = hits[0]
    layer_dir = out / 'vk_layer'
    layer_dir.mkdir(exist_ok=True)
    install_layer(layer_dir)
    env = os.environ.copy()
    env.update(layer_env(layer_dir), XDG_RUNTIME_DIR='/tmp/runtime-ubuntu', RUST_LOG='error')
    os.makedirs(env['XDG_RUNTIME_DIR'], exist_ok=True)
    command = ['valgrind', '--tool=callgrind', '--demangle=no', '--instr-atstart=no', '--separate-threads=no',
               *function_options(roots.values()),
               f'--callgrind-out-file={prefix}', str(binary), 'measure', '--engine', 'cherenkov',
               '--scene', f'scenes/perf/{scene}', '--frames', '100000', '--warmup', str(warmup),
               '--pause-at', str(pause_at), '--out', str(prefix) + '.unused-timing.json']
    after = {phase: [] for phase in roots}
    monitor = selectors.DefaultSelector(); fds = []
    watch = LIBC.inotify_init1(os.O_CLOEXEC)
    if watch < 0:
        raise OSError(ctypes.get_errno(), 'inotify_init1')
    assert LIBC.inotify_add_watch(watch, os.fsencode(out), 0x8 | 0x80) >= 0
    fds.append(watch); monitor.register(watch, selectors.EVENT_READ, 'files')
    deadline = timer(5400); fds.append(deadline); monitor.register(deadline, selectors.EVENT_READ, 'deadline')
    seen = set()
    with open(str(prefix) + '.log', 'w') as log:
        process = subprocess.Popen(command, cwd=repo, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, start_new_session=True)
        pidfd = os.pidfd_open(process.pid); fds.append(pidfd); monitor.register(pidfd, selectors.EVENT_READ, 'exit')
        monitor.register(process.stdout, selectors.EVENT_READ, 'stdout')
        pending = b''
        print(tag, scene, 'pid', process.pid, 'running to frame', pause_at, flush=True)

        # Three calls of every root, and the one in flight when
        # instrumentation switched on. Callgrind writes its dumps one after
        # another, so every dump up to a --dump-after is closed once it is.
        def done():
            return all(len(v) > 3 for v in after.values())
        try:
            while not done():
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
                                print(tag, scene, 'instrumentation on at frame', pause_at, flush=True)
                    elif event.data == 'deadline':
                        raise TimeoutError((tag, scene, {k: len(v) for k, v in after.items()}))
                    elif event.data == 'exit':
                        raise RuntimeError((tag, scene, 'exited early', process.wait()))
                    else:
                        data = os.read(watch, 65536); offset = 0
                        while offset < len(data):
                            _, _, _, size = struct.unpack_from('iIII', data, offset); offset += 16
                            name = os.fsdecode(data[offset:offset + size].split(b'\0', 1)[0]); offset += size
                            match = re.fullmatch(re.escape(prefix.name) + r'\.(\d+)', name)
                            if not match or int(match[1]) in seen:
                                continue
                            number = int(match[1]); seen.add(number)
                            head = (out / name).read_text().split('\n\nob=', 1)[0]
                            for phase, symbol in roots.items():
                                if f'desc: Trigger: --dump-after={symbol}\n' in head:
                                    after[phase].append(number)
        finally:
            process.terminate()
            try:
                process.wait(timeout=60)
            except subprocess.TimeoutExpired:
                process.kill(); process.wait()
            monitor.close()
            for fd in fds:
                os.close(fd)
    result = {'tag': tag, 'scene': scene, 'binary': str(binary), 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'pause_at': pause_at, 'command': command}
    for phase, symbol in roots.items():
        samples = call_samples(out, prefix.name, symbol, after[phase])
        assert len(samples) == 3, (tag, scene, phase, len(samples))
        second, third = samples[1], samples[2]
        result[phase] = {'root': symbol, 'first': samples[0], 'second': second, 'third': third,
                         'third_delta_percent': 100 * (third['rust_ir'] / second['rust_ir'] - 1)}
        print(tag, scene, phase, 'SECOND', second['ir'], 'rust', second['rust_ir'], 'allocs', second['allocs'],
              'reallocs', second['reallocs'], 'deallocs', second['deallocs'], 'mem', second['mem_calls'], 'THIRD rust', third['rust_ir'], flush=True)
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


def batch(args):
    out = pathlib.Path(args.out); out.mkdir(parents=True, exist_ok=True)

    def task(scene):
        path = out / f'{args.tag}-{scene}.result.json'
        if path.is_file():
            d = json.loads(path.read_text())
            assert d['sha256'] == hashlib.sha256(pathlib.Path(d['binary']).read_bytes()).hexdigest()
            return f'{args.tag} {scene} reused completed profile'
        with (out / f'{args.tag}-{scene}.progress.log').open('w') as log:
            subprocess.run([sys.executable, __file__, 'profile', '--binary', args.binary, '--tag', args.tag, '--scene', scene,
                            '--repo', args.repo, '--out', args.out, '--pause-at', str(args.pause_at), '--warmup', str(args.warmup)],
                           stdout=log, stderr=subprocess.STDOUT, check=True)
        return f'{args.tag} {scene} done'
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for line in pool.map(task, SCENES):
            print(line, flush=True)
    lines = [f'# Ir at the second steady frame: `{args.tag}`', '',
             f'Instrumentation on at frame {args.pause_at}; sample 2 of each root. Rust Ir excludes the allocator and the libc memory primitives.', '',
             '| Scene | Phase | Ir | Allocator Ir | Mem Ir | Rust Ir | allocs | reallocs | deallocs | mem calls | Rust Ir 2nd → 3rd |',
             '|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|']
    for scene in SCENES:
        d = load(args.out, args.tag, scene)
        for phase in ROOTS:
            s, t = d[phase]['second'], d[phase]['third']
            lines.append(f"| {scene} | {phase} | {s['ir']:,} | {s['alloc_ir']:,} | {s['mem_ir']:,} | {s['rust_ir']:,} | {s['allocs']} | {s['reallocs']} | {s['deallocs']} | {s['mem_calls']} | "
                         f"{t['rust_ir']:,} ({d[phase]['third_delta_percent']:+.3f}%) |")
    report = '\n'.join(lines) + '\n'
    (out / f'{args.tag}-ir.md').write_text(report); print(report)


def compare(args):
    lines = [f'# Rust Ir and allocator calls: `{args.base}` → `{args.head}`', '',
             'Second steady frame. Fails when Rust Ir rises more than 1% or any allocator or memory-primitive call count rises.', '',
             '| Scene | Phase | Rust Ir base | Rust Ir head | Δ | allocs | reallocs | deallocs | mem calls | |', '|---|---|---:|---:|---:|---:|---:|---:|---:|---|']
    failed = False
    for scene in SCENES:
        a, b = load(args.out, args.base, scene), load(args.out, args.head, scene)
        for phase in ROOTS:
            x, y = a[phase]['second'], b[phase]['second']
            delta = 100 * (y['rust_ir'] / x['rust_ir'] - 1)
            ok = delta <= 1 and all(y[k] <= x[k] for k in ('allocs', 'reallocs', 'deallocs', 'mem_calls'))
            failed |= not ok
            counts = [f"{x[k]} → {y[k]}" for k in ('allocs', 'reallocs', 'deallocs', 'mem_calls')]
            lines.append(f"| {scene} | {phase} | {x['rust_ir']:,} | {y['rust_ir']:,} | {delta:+.3f}% | {' | '.join(counts)} | {'ok' if ok else 'FAIL'} |")
    print('\n'.join(lines))
    return 1 if failed else 0


def same(args):
    failed = False
    for scene in SCENES:
        ds = [load(args.out, tag, scene) for tag in args.tags]
        assert len({d['sha256'] for d in ds}) == 1, (scene, 'different binaries')
        for phase in ROOTS:
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
    for name in ('profile', 'batch'):
        p = sub.add_parser(name)
        p.add_argument('--binary', required=True); p.add_argument('--tag', required=True)
        p.add_argument('--repo', required=True, help='checkout holding scenes/perf')
        p.add_argument('--out', required=True); p.add_argument('--pause-at', type=int, default=8); p.add_argument('--warmup', type=int, default=30)
        if name == 'profile':
            p.add_argument('--scene', required=True, choices=SCENES)
        else:
            p.add_argument('--jobs', type=int, default=2)
    p = sub.add_parser('compare'); p.add_argument('--out', required=True); p.add_argument('base'); p.add_argument('head')
    p = sub.add_parser('same'); p.add_argument('--out', required=True); p.add_argument('tags', nargs='+')
    args = parser.parse_args()
    if args.cmd == 'profile':
        profile(args.binary, args.tag, args.scene, args.repo, args.out, args.pause_at, args.warmup)
    elif args.cmd == 'batch':
        batch(args)
    elif args.cmd == 'compare':
        sys.exit(compare(args))
    else:
        sys.exit(same(args))


if __name__ == '__main__':
    main()
