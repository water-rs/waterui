#!/usr/bin/env python3
"""Scaling of `lower` Ir with path segment count on the `road-network` series.

`ir_gate.py` answers "did this change cost more on these scenes"; this answers
"how does the cost grow with the size of one path". The `road-network` perf
scenes are the same geometry, canvas, pan and stroke at five grid sizes, so
the only thing that changes between them is the number of line segments the
coverage sweep sorts:

  road-network-n10    4,720 segments
  road-network-n14    9,400
  road-network-n20   19,040
  road-network-n28   37,840
  road-network       76,480

`measure` profiles each size exactly as the Ir gate does — `ir_gate.profile`,
instrumentation switched on at a fixed frame, the second steady call of
`lower` as the sample, Rust Ir excluding the allocator and the libc memory
primitives — and `report` fits `rust_ir ~ segments ** e` by least squares on
the logarithms, prints the per-size numbers, the local exponent of each
consecutive pair, and the fitted exponent over the whole range.

Why an exponent is the right shape of measurement: the CPU route's cost is
`O((n + k) log n)` in `n` segments and `k` crossings (#2536, section 3), and
`k` on a road network grows faster than `n`, so `lower` is superlinear in `n`
by construction. water-rs/waterui#2536's GPU coverage route replaces the
crossing sort with per-segment binning whose work is `O(n)` in the same
segment-bound pass that sizes its buffers, so the route's claim is exactly a
statement about this exponent.

**This is a report, not a gate, until the GPU coverage route exists.** The CPU
route cannot pass it today and is not expected to: `report` prints the
verdict and exits 0, saying so. #2543 lands the GPU route and turns it into a
gate by passing `--gate`, which exits non-zero when the fitted exponent
exceeds `--exponent`. Nothing here gates CI in the meantime.

Callgrind runs on Linux only (and `ir_gate`'s dump watcher is built on
`inotify` and `timerfd`), so `measure` refuses to run anywhere else instead of
reporting a number it did not measure. No measurement of this series exists
yet for the same reason: it is taken on Linux, on the lane that already runs
`ir_gate.py`, and the first real exponent for the CPU sweep comes from there.
The figures in `test_lower_scaling.py` are synthetic inputs chosen to exercise
the fit (`segments ** 1.4` and `40 * segments`), not measurements of anything.

  lower_scaling.py measure --binary B --tag T --repo R --out O [--jobs N]
  lower_scaling.py report  --out O --tag T [--exponent 1.1] [--gate]
"""
import argparse
import concurrent.futures
import importlib.util
import json
import math
import pathlib
import sys

SPEC = importlib.util.spec_from_file_location('ir_gate', pathlib.Path(__file__).with_name('ir_gate.py'))
ir_gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ir_gate)

# Scene name and its line segment count: `n * n * 40` horizontal roads plus
# `ceil(n / 5) * (n - 1) * 40` verticals, as `generate-corpus` builds them.
SIZES = [('road-network-n10', 4_720), ('road-network-n14', 9_400), ('road-network-n20', 19_040),
         ('road-network-n28', 37_840), ('road-network', 76_480)]
PHASE = 'lower'
# The exponent the GPU coverage route is claimed to reach: its coverage work
# is `O(n)` per segment plus a per-cell pass, so `lower` may grow a little
# faster than the segment count through the parts it does not move to the
# GPU (flattening, stroke expansion, cell emission), and no faster.
EXPONENT = 1.1


def fit(points):
    """The least-squares exponent of `y ~ x ** e` over `(x, y)` points."""
    assert len(points) >= 2, points
    xs = [math.log(x) for x, _ in points]
    ys = [math.log(y) for _, y in points]
    mx = sum(xs) / len(xs)
    my = sum(ys) / len(ys)
    sxy = sum((x - mx) * (y - my) for x, y in zip(xs, ys))
    sxx = sum((x - mx) ** 2 for x in xs)
    assert sxx > 0, points
    return sxy / sxx


def local(points):
    """The exponent between each consecutive pair of `(x, y)` points."""
    return [(a[0], b[0], math.log(b[1] / a[1]) / math.log(b[0] / a[0]))
            for a, b in zip(points, points[1:])]


def measure(args):
    if sys.platform != 'linux':
        sys.exit(f'Callgrind runs on Linux only; this is {sys.platform}. Run the scaling report on a Linux host.')
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    icd = ir_gate.lavapipe_icd(args.icd)

    def task(scene):
        path = out / f'{args.tag}-{scene}.result.json'
        if path.is_file():
            ir_gate.verify_reused_profile(json.loads(path.read_text()), args.binary, icd)
            return f'{args.tag} {scene} reused completed profile'
        # Every size is the road-network scene, with its roots.
        ir_gate.profile(args.binary, args.tag, scene, args.repo, args.out, args.pause_at, args.warmup, icd,
                        ir_gate.SCENES['road-network'])
        return f'{args.tag} {scene} done'

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for line in pool.map(task, [scene for scene, _ in SIZES]):
            print(line, flush=True)


def report(args):
    points = []
    rows = []
    for scene, segments in SIZES:
        sample = ir_gate.load(args.out, args.tag, scene)[PHASE]['second']
        points.append((segments, sample['rust_ir']))
        rows.append((scene, segments, sample))
    exponent = fit(points)
    lines = [f'# `{PHASE}` Ir against path segment count: `{args.tag}`', '',
             'Second steady call of `lower`, Rust Ir (allocator and libc memory primitives excluded), '
             'one path per scene on the same canvas and pan.', '',
             '| Scene | Segments | Ir | Rust Ir | Rust Ir / segment |', '|---|---:|---:|---:|---:|']
    for scene, segments, sample in rows:
        lines.append(f"| {scene} | {segments:,} | {sample['ir']:,} | {sample['rust_ir']:,} | "
                     f"{sample['rust_ir'] / segments:,.1f} |")
    lines += ['', '| From | To | Local exponent |', '|---|---|---:|']
    for a, b, e in local(points):
        lines.append(f'| {a:,} | {b:,} | {e:.3f} |')
    ok = exponent <= args.exponent
    lines += ['', f'Fitted exponent over {points[0][0]:,}–{points[-1][0]:,} segments: '
                  f'**{exponent:.3f}** against a stated bound of {args.exponent:.3f} — '
                  f"{'within' if ok else 'above'} the bound."]
    if args.gate:
        lines.append('')
        lines.append(f"Gate: {'pass' if ok else 'FAIL'}.")
    else:
        lines += ['',
                  'Report only: the CPU coverage sweep is `O((n + k) log n)` in segments and crossings, so it '
                  'is expected to exceed the bound, and no result here gates anything. '
                  'water-rs/waterui#2543 turns this into a gate by passing `--gate` once the GPU coverage '
                  'route selects these scenes.']
    print('\n'.join(lines))
    pathlib.Path(args.out, f'{args.tag}-lower-scaling.md').write_text('\n'.join(lines) + '\n')
    return 1 if args.gate and not ok else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='cmd', required=True)
    p = sub.add_parser('measure')
    p.add_argument('--binary', required=True)
    p.add_argument('--tag', required=True)
    p.add_argument('--repo', required=True, help='checkout directory holding scenes/perf (the cherenkov tree)')
    p.add_argument('--out', required=True)
    p.add_argument('--pause-at', type=int, default=8)
    p.add_argument('--warmup', type=int, default=30)
    p.add_argument('--jobs', type=int, default=2)
    p.add_argument('--icd', type=pathlib.Path, help='explicit Lavapipe ICD manifest')
    p = sub.add_parser('report')
    p.add_argument('--out', required=True)
    p.add_argument('--tag', required=True)
    p.add_argument('--exponent', type=float, default=EXPONENT)
    p.add_argument('--gate', action='store_true', help='exit non-zero when the fitted exponent exceeds the bound')
    args = parser.parse_args()
    if args.cmd == 'measure':
        measure(args)
    else:
        sys.exit(report(args))


if __name__ == '__main__':
    main()
