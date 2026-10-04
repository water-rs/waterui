#!/usr/bin/env python3
"""Callgrind Ir gate for `lower` and `encode` on the five perf scenes.

Each profile runs `cherenkov-bench measure --pause-at P` under Callgrind with
instrumentation off. At the pause (a fixed frame index, not wall-clock time)
instrumentation is switched on and the bench resumes. `--dump-before` and
`--dump-after` on each root make every dump cover exactly one call of that
root on its thread. The k-th dump after a root returns is sample k; sample 2
is the gate's "second steady frame" and sample 3 its stability check.

Per sample it reports the root's inclusive Ir; the allocator's share of it
(the inclusive cost of every call from other code into the Rust allocator
shims and the libc malloc family) with the number of alloc, realloc and
dealloc calls; the libc memory primitives' share (memcpy, memmove, memset,
memcmp, bcmp), whose Ir depends on buffer addresses the allocator chooses,
with their call count; and the Rust Ir: inclusive Ir minus both shares.

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
import argparse, collections, concurrent.futures, ctypes, hashlib, json, os, pathlib, re, selectors, struct, subprocess, sys

SCENES = ['map', 'chart', 'text-page', 'ui-list', 'effects']
ROOTS = {'lower': ('13lower_content', 'cherenkov_gpu'), 'encode': ('6Engine6encode', 'cherenkov_ad')}
PAUSE = 'cherenkov-bench: paused before frame {}'
ALLOC = re.compile(r"(___rust_(alloc|dealloc|realloc|alloc_zeroed)|___rdl_(alloc|dealloc|realloc|alloc_zeroed)"
                   r"|___rust_no_alloc_shim_is_unstable\w*"
                   r"|^(malloc|free|realloc|calloc|posix_memalign|aligned_alloc|memalign|valloc|pvalloc))('\d+)?$")
KIND = [(re.compile(r"(___rust_alloc|___rust_alloc_zeroed|^malloc|^calloc|^posix_memalign|^aligned_alloc|^memalign|^valloc|^pvalloc)('\d+)?$"), 'allocs'),
        (re.compile(r"(___rust_realloc|^realloc)('\d+)?$"), 'reallocs'),
        (re.compile(r"(___rust_dealloc|^free)('\d+)?$"), 'deallocs')]
MEM = re.compile(r"^(__)?(memcpy|memmove|mempcpy|memset|memcmp|bcmp)(_\w+)?('\d+)?$")
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
    """Function names, self Ir, and per-edge inclusive Ir and call counts."""
    names = {}; own = collections.Counter(); edge = collections.Counter(); calls = collections.Counter(); total = None
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
        elif line.startswith('totals:') or line.startswith('summary:'):
            total = int(line.split()[1])
        elif current is not None and re.match(r'^[0-9+*\-]', line):
            parts = line.split()
            if len(parts) != 2 or not parts[-1].isdigit():
                continue
            if pending is not None:
                edge[(current, callee)] += int(parts[-1]); calls[(current, callee)] += pending; pending = None
            else:
                own[current] += int(parts[-1])
    return names, own, edge, calls, total


def measure(path, symbol):
    names, own, edge, calls, total = parse(path)
    ids = [k for k, v in names.items() if v == symbol]
    if len(ids) != 1:
        return None
    root = ids[0]
    ncalls = sum(c for (a, b), c in calls.items() if b == root)
    ir = sum(c for (a, b), c in edge.items() if b == root)
    assert ncalls == 1 and ir > 0, (path, ncalls, ir)
    is_alloc = lambda k: bool(ALLOC.search(names.get(k, '')))
    is_mem = lambda k: bool(MEM.search(names.get(k, '')))
    excluded = lambda k: is_alloc(k) or is_mem(k)
    kids = collections.defaultdict(set)
    for a, b in edge:
        kids[a].add(b)
    inside = set(); stack = [root]
    while stack:
        f = stack.pop()
        if f not in inside:
            inside.add(f); stack += [g for g in kids[f] if not excluded(g)]
    alloc_ir = mem_ir = mem_calls = 0; counts = collections.Counter({k: 0 for _, k in KIND})
    for (a, b), cost in edge.items():
        if a not in inside or excluded(a):
            continue
        if is_alloc(b):
            alloc_ir += cost
            for pattern, kind in KIND:
                if pattern.search(names[b]):
                    counts[kind] += calls[(a, b)]
        elif is_mem(b):
            mem_ir += cost; mem_calls += calls[(a, b)]
    return {'path': str(path), 'ir': ir, 'alloc_ir': alloc_ir, 'mem_ir': mem_ir, 'mem_calls': mem_calls,
            'rust_ir': ir - alloc_ir - mem_ir, 'outside_ir': total - ir if total is not None else None, **counts}


def dump_files(out, prefix, number):
    return sorted(out.glob(f'{prefix}.{number}-*'))


def profile(binary, tag, scene, repo, out, pause_at, warmup):
    binary = pathlib.Path(binary).resolve(); out = pathlib.Path(out).resolve(); out.mkdir(parents=True, exist_ok=True)
    prefix = out / f'{tag}-{scene}'
    assert not [p for p in out.glob(prefix.name + '.*') if not p.name.endswith('.progress.log')], f'existing run: {prefix}'
    symbols = subprocess.run(['nm', '--defined-only', str(binary)], capture_output=True, text=True, check=True).stdout.splitlines()
    roots = {}
    for phase, (suffix, crate) in ROOTS.items():
        hits = [line.split()[-1] for line in symbols if line.split()[-1].endswith(suffix) and crate in line]
        assert len(hits) == 1, (tag, phase, hits)
        roots[phase] = hits[0]
    layer_dir = out / 'vk_layer'
    layer_dir.mkdir(exist_ok=True)
    source = pathlib.Path(__file__).with_name('ir_no_raster.c')
    digest = hashlib.sha256(source.read_bytes()).hexdigest()[:16]
    library = layer_dir / f'libVkLayerCherenkovNoRaster-{digest}.so'
    tmp = layer_dir / f'{library.name}.{os.getpid()}.tmp'
    subprocess.run(['cc', '-shared', '-fPIC', '-O2', '-o', str(tmp), str(source)], check=True)
    os.replace(tmp, library)
    manifest = layer_dir / 'VkLayer_cherenkov_no_raster.json'
    manifest.write_text(json.dumps({
        'file_format_version': '1.0.0',
        'layer': {'name': 'VK_LAYER_CHERENKOV_no_raster', 'type': 'GLOBAL', 'library_path': str(library),
                  'api_version': '1.3.0', 'implementation_version': '1',
                  'description': 'No-op GPU-work vkCmd* entry points (gate only)'}}, indent=1) + '\n')
    env = os.environ.copy()
    env.update(VK_ICD_FILENAMES='/usr/share/vulkan/icd.d/lvp_icd.x86_64.json', XDG_RUNTIME_DIR='/tmp/runtime-ubuntu', RUST_LOG='error',
               VK_LAYER_PATH=str(layer_dir), VK_INSTANCE_LAYERS='VK_LAYER_CHERENKOV_no_raster', NODEVICE_SELECT='1')
    os.makedirs(env['XDG_RUNTIME_DIR'], exist_ok=True)
    dumps = [f'--dump-{when}={sym}' for sym in roots.values() for when in ('before', 'after')]
    command = ['valgrind', '--tool=callgrind', '--instr-atstart=no', '--separate-threads=yes', *dumps,
               f'--callgrind-out-file={prefix}', str(binary), 'measure', '--engine', 'cherenkov',
               '--scene', f'scenes/perf/{scene}', '--frames', '100000', '--warmup', str(warmup),
               '--pause-at', str(pause_at), '--out', str(prefix) + '.unused-timing.json']
    after = {phase: [] for phase in roots}; latest = -1
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

        def done():
            used = max(after[p][2] for p in after) if all(len(v) >= 3 for v in after.values()) else None
            return used is not None and latest > used
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
                            match = re.fullmatch(re.escape(prefix.name) + r'\.(\d+)-(\d+)', name)
                            if not match or int(match[1]) in seen:
                                continue
                            number = int(match[1]); seen.add(number); latest = max(latest, number)
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
        samples = []
        for number in sorted(after[phase])[:3]:
            found = [m for m in (measure(p, symbol) for p in dump_files(out, prefix.name, number)) if m]
            assert len(found) == 1, (tag, scene, phase, number, len(found))
            samples.append(found[0])
        second, third = samples[1], samples[2]
        result[phase] = {'root': symbol, 'first': samples[0], 'second': second, 'third': third,
                         'third_delta_percent': 100 * (third['rust_ir'] / second['rust_ir'] - 1)}
        print(tag, scene, phase, 'SECOND', second['ir'], 'rust', second['rust_ir'], 'allocs', second['allocs'],
              'reallocs', second['reallocs'], 'deallocs', second['deallocs'], 'mem', second['mem_calls'], 'THIRD rust', third['rust_ir'], flush=True)
    pathlib.Path(str(prefix) + '.result.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


def load(out, tag, scene):
    return json.loads((pathlib.Path(out) / f'{tag}-{scene}.result.json').read_text())


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
