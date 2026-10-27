#!/usr/bin/env python3
"""Scaling of `lower` Ir with path segment count on the `road-network` series.

`ir_gate.py` answers "did this change cost more on these scenes"; this answers
"how does the cost grow with the size of one path". The `road-network` perf
scenes are the same geometry, canvas, pan and stroke at five grid sizes, so
the only thing that changes between them is the number of input line segments:

  road-network-n10    4,720 segments
  road-network-n14    9,400
  road-network-n20   19,040
  road-network-n28   37,840
  road-network       76,480

A second series, `--series dash`, is the same five paths stroked with the
dash pattern `[4, 2]` at offset 1 (`road-network-dash-n10` ...
`road-network-dash`), whose segment axis is the same centreline count. It is
not part of the Ir or memory gates.

`measure` runs each size twice: before frame zero to capture fresh geometry
preparation, and after warmup to capture retained reuse. The report fits fresh
`lower` Rust Ir against segment count and reports retained costs separately.
Allocator and libc memory instructions are excluded in both intervals.

The GPU coverage route is implemented. This measures its CPU preparation, not
GPU execution or an end-to-end frame budget. Dense overlap can increase local
intersection work; no universal linear bound follows from using compute.
`--gate` applies the selected exponent bound to fresh preparation only.
Callgrind runs on Linux; the fit tests use synthetic data, not measurements.

  lower_scaling.py measure --binary B --tag T --repo R --out O [--series S] [--jobs N]
  lower_scaling.py report  --out O --tag T [--series S] [--exponent 1.1] [--gate]
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

# Grid size suffix and line segment count: `n * n * 40` horizontal roads plus
# `ceil(n / 5) * (n - 1) * 40` verticals, as `generate-corpus` builds them.
GRIDS = [('-n10', 4_720), ('-n14', 9_400), ('-n20', 19_040), ('-n28', 37_840), ('', 76_480)]
# Each series' scene stem: the solid stroke and the dashed one.
STEMS = {'solid': 'road-network', 'dash': 'road-network-dash'}
SERIES = {series: [(f'{stem}{suffix}', segments) for suffix, segments in GRIDS]
          for series, stem in STEMS.items()}
PHASE = 'lower'
# Acceptance bound for this road-network series, not arbitrary geometry.
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
        for mode in ('fresh', 'retained'):
            tag = f'{args.tag}-{mode}'
            path = out / f'{tag}-{scene}.result.json'
            pause = 0 if mode == 'fresh' else args.pause_at
            if path.is_file():
                result = json.loads(path.read_text())
                ir_gate.verify_reused_profile(result, args.binary, icd)
                assert result['sample_mode'] == mode and result['pause_at'] == pause
                continue
            ir_gate.profile(args.binary, tag, scene, args.repo, args.out, pause, args.warmup, icd,
                            ir_gate.EVERY_FRAME if mode == 'fresh' else ir_gate.RECORDED_ONCE,
                            fresh=mode == 'fresh')
        return f'{args.tag} {scene} fresh and retained done'

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for line in pool.map(task, [scene for scene, _ in SERIES[args.series]]):
            print(line, flush=True)


def report(args):
    points = []
    rows = []
    for scene, segments in SERIES[args.series]:
        fresh = ir_gate.load(args.out, f'{args.tag}-fresh', scene)
        retained = ir_gate.load(args.out, f'{args.tag}-retained', scene)
        assert fresh['sample_mode'] == 'fresh' and fresh['pause_at'] == 0
        assert retained['sample_mode'] == 'retained' and retained['pause_at'] > 0
        assert fresh['sha256'] == retained['sha256'], 'fresh and retained must use the same binary'
        sample = fresh[PHASE]['first']
        points.append((segments, sample['rust_ir']))
        rows.append((scene, segments, sample, retained[PHASE]['second']))
    exponent = fit(points)
    lines = [f'# `{PHASE}` Ir against path segment count: `{args.tag}`, {args.series} series', '',
             'First complete call of `lower` before any geometry reuse, Rust Ir (allocator and libc memory primitives excluded), '
             'one path per scene on the same canvas and pan.', '',
             '| Scene | Segments | Fresh Ir | Fresh Rust Ir | Fresh Rust Ir / segment | Retained Rust Ir |',
             '|---|---:|---:|---:|---:|---:|']
    for scene, segments, sample, retained in rows:
        lines.append(f"| {scene} | {segments:,} | {sample['ir']:,} | {sample['rust_ir']:,} | "
                     f"{sample['rust_ir'] / segments:,.1f} | {retained['rust_ir']:,} |")
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
        lines += ['', 'Report only: pass --gate to enforce the fresh-preparation exponent bound.']
    lines += ['', 'Retained Rust Ir is the second steady call from a separate run of the same binary. '
              'It is reported for comparison and does not decide the fresh-preparation gate. '
              'Neither interval measures GPU execution.']
    print('\n'.join(lines))
    pathlib.Path(args.out, f'{args.tag}-{args.series}-lower-scaling.md').write_text('\n'.join(lines) + '\n')
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
    p.add_argument('--series', choices=sorted(SERIES), default='solid')
    p.add_argument('--jobs', type=int, default=2)
    p.add_argument('--icd', type=pathlib.Path, help='explicit Lavapipe ICD manifest')
    p = sub.add_parser('report')
    p.add_argument('--out', required=True)
    p.add_argument('--tag', required=True)
    p.add_argument('--series', choices=sorted(SERIES), default='solid')
    p.add_argument('--exponent', type=float, default=EXPONENT)
    p.add_argument('--gate', action='store_true', help='exit non-zero when the fitted exponent exceeds the bound')
    args = parser.parse_args()
    if args.cmd == 'measure':
        measure(args)
    else:
        sys.exit(report(args))


if __name__ == '__main__':
    main()
