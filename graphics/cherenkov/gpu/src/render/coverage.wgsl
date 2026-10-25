// Batched GPU fill coverage: flatten curves, bin into four-row strips,
// split only boundary cells, radix-sort their logical keys, and prefix
// winding across sparse columns. Integrate each local arrangement before
// applying the fill rule; integrated winding alone cannot represent area.
// Classify sparse runs and emit raster instances plus packed R8 coverage.
// Large full interiors remain implicit, so storage follows boundary size.

const STRIP_H: u32 = 4u;
const FULL_RUN_MIN: u32 = 8u;
const PARTIAL_GAP_MAX: u32 = 8u;
const SMALL_BBOX: f32 = 32.0;
const FLATTEN: f32 = 0.02;

const KIND_GLYPH: u32 = 4u;
const KIND_SPAN: u32 = 5u;

const F_INF: f32 = 3.4e38;
const F_NINF: f32 = -3.4e38;

// Encoded curve element, 3 vec4s per record:
//   head = (kind, n, path index, segment slot base relative to path)
//   lo   = (p0.x, p0.y, a.x, a.y)
//   hi   = (b.x, b.y, p1.x, p1.y)
// kind 0 line / 1 quad / 2 cubic; a quad pads b with a; a line's a/b are
// unused. n is the uniform subdivision count the bound pass chose.
struct Element {
    head: vec4<u32>,
    lo: vec4<f32>,
    hi: vec4<f32>,
}

// One GPU-routed path's dispatch record. atom[0..3] is the device bbox
// (seeded +/-inf by the uploader, reduced by `flatten`); atom[4..6]
// counts emitted instances and texel words; atom[7] is the packed work offset.
// `rect_*` is written by `finish` from the reduced bbox — the bound
// fields only size scratch. The stroke fields (from `stroke_base`) belong to
// the compute stroker (water-rs/waterui#2544) and are zero for a fill.
struct GpuPath {
    atom: array<atomic<u32>, 8>,
    elem_base: u32,
    elem_count: u32,
    seg_base: u32,
    seg_count: u32,
    grid_base: u32,
    texel_base: u32,
    inst_base: u32,
    rule: u32,
    grid_w: u32,
    grid_h: u32,
    row_base: u32,
    _pad: u32,
    affine: vec4<f32>,
    translate: vec2<f32>,
    clip_hi: vec2<f32>,
    tile_base: u32,
    bound_y: f32,
    rect_x: f32,
    rect_y: f32,
    rect_w: u32,
    rect_h: u32,
    clip_lo: vec2<f32>,
    // Absolute index of the path's first stroke record.
    stroke_base: u32,
    half_width: f32,
    miter_limit: f32,
    // start cap | end cap << 2 | merge_start << 4 | round-cap chords << 8
    cap_word: u32,
    // Word offset of the path's dash table behind the stroke records,
    // 0xffffffff when the stroke is undashed.
    dash_base: u32,
    // Entries of the effective dash pattern.
    dash_entries: u32,
    // The initial dash phase, 2^-32 fixed point (integer part, fraction).
    dash_phase: vec2<u32>,
    // The path's first entry in the dash staging area.
    stage_base: u32,
    _pad3a: u32,
    _pad3b: u32,
    _pad3c: u32,
}

struct Params {
    element_total: u32,
    row_total: u32,
    strip_total: u32,
    path_count: u32,
    tile_total: u32,
    classify_width: u32,
    grid_total: u32,
    bin_base: u32,
    tile_prefix: u32,
    fragment_base: u32,
    fragment_cap: u32,
    sort_a: u32,
    sort_b: u32,
    histogram: u32,
    groups: u32,
    tokens: u32,
    sorted: u32,
    radix_pass: u32,
    // The compute stroker's frame totals and scratch regions.
    stroke_total: u32,
    dash_total: u32,
    // Absolute index of the first dashed record; dashed records end the range.
    dash_first: u32,
    dash_blocks: u32,
    // 0: arc-length prefix, 1: fragment and chord offsets.
    scan_op: u32,
    dash_words_base: u32,
    scan_words_base: u32,
    staging_base: u32,
    list_base: u32,
    // Uniform structs round up to 16 bytes: the binding is 112.
    _pad: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> elements: array<Element>;
@group(0) @binding(2) var<storage, read_write> paths: array<GpuPath>;
@group(0) @binding(3) var<storage, read_write> segments: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> coverage: array<u32>;
// The coverage arena: one block per entry — a count header, the emitted
// 32-byte geometry records, then the packed R8 texels (water-rs/waterui#3081).
@group(0) @binding(5) var<storage, read_write> arena: array<u32>;
@group(0) @binding(8) var<storage, read_write> args: array<u32, 15>;
struct Work {
    bin_count: atomic<u32>,
    fragment_count: atomic<u32>,
    token_count: atomic<u32>,
    fragments: u32,
    // Dash fragments the compute stroker staged, written by `dash_args`.
    dash_fragments: u32,
    words: array<atomic<u32>>,
}
@group(0) @binding(6) var<storage, read_write> work: Work;

fn ordered(value: f32) -> u32 {
    let bits = bitcast<u32>(value);
    return select(~bits, bits ^ 0x80000000u, (bits & 0x80000000u) == 0u);
}
fn unordered(value: u32) -> f32 {
    return bitcast<f32>(select(~value, value ^ 0x80000000u, (value & 0x80000000u) != 0u));
}

fn xf(pi: u32, pt: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        paths[pi].affine.x * pt.x + paths[pi].affine.y * pt.y + paths[pi].translate.x,
        paths[pi].affine.z * pt.x + paths[pi].affine.w * pt.y + paths[pi].translate.y,
    );
}

fn cubic_at(p0: vec2<f32>, c0: vec2<f32>, c1: vec2<f32>, p1: vec2<f32>, t: f32) -> vec2<f32> {
    let u = 1.0 - t;
    return p0 * (u * u * u) + c0 * (3.0 * u * u * t) + c1 * (3.0 * u * t * t) + p1 * (t * t * t);
}

// The derivative of a cubic Bezier at `t`.
fn cubic_deriv(p0: vec2<f32>, c0: vec2<f32>, c1: vec2<f32>, p1: vec2<f32>, t: f32) -> vec2<f32> {
    let u = 1.0 - t;
    return 3.0 * (u * u * (c0 - p0) + 2.0 * u * t * (c1 - c0) + t * t * (p1 - c1));
}

// The coverage grid rect the CPU sweep derives: the segment bbox
// inflated by 1 px and clipped to the generation frame's clip —
// `[0, surface]` for a clipped path, the same rect shifted by −offset
// for a retained placement-relative one (water-rs/waterui#3081).
fn grid_rect(pi: u32) -> vec4<f32> {
    let x0 = max(unordered(atomicLoad(&paths[pi].atom[0])) - 1.0, paths[pi].clip_lo.x);
    let y0 = max(unordered(atomicLoad(&paths[pi].atom[1])) - 1.0, paths[pi].clip_lo.y);
    let x1 = min(unordered(atomicLoad(&paths[pi].atom[2])) + 1.0, paths[pi].clip_hi.x);
    let y1 = min(unordered(atomicLoad(&paths[pi].atom[3])) + 1.0, paths[pi].clip_hi.y);
    let gx = floor(x0);
    let gy = floor(y0);
    let w = max(ceil(x1) - gx, 0.0);
    let h = max(ceil(y1) - gy, 0.0);
    return vec4<f32>(gx, gy, w, h);
}

// ---- flatten ---------------------------------------------------------------

// Folds a device point into the path's reduced bounding box.
fn reduce_bbox(pi: u32, p: vec2<f32>) {
    atomicMin(&paths[pi].atom[0], ordered(p.x));
    atomicMin(&paths[pi].atom[1], ordered(p.y));
    atomicMax(&paths[pi].atom[2], ordered(p.x));
    atomicMax(&paths[pi].atom[3], ordered(p.y));
}

@compute @workgroup_size(64)
fn flatten(@builtin(global_invocation_id) gid: vec3<u32>) {
    if invocation(gid) >= params.element_total {
        return;
    }
    let el = elements[invocation(gid)];
    let kind = u32(el.head.x);
    let n = u32(el.head.y);
    let path_i = u32(el.head.z);
    let slot = u32(el.head.w);
    let pi = path_i;
    var prev = xf(pi, el.lo.xy);
    reduce_bbox(pi, prev);
    for (var i = 1u; i <= n; i = i + 1u) {
        let t = f32(i) / f32(n);
        var lp: vec2<f32>;
        if kind == 0u {
            lp = mix(el.lo.xy, el.hi.zw, t);
        } else if kind == 1u {
            let u = 1.0 - t;
            lp = el.lo.xy * (u * u) + el.lo.zw * (2.0 * u * t) + el.hi.zw * (t * t);
        } else {
            lp = cubic_at(el.lo.xy, el.lo.zw, el.hi.xy, el.hi.zw, t);
        }
        let pt = xf(pi, lp);
        segments[paths[pi].seg_base + slot + i - 1u] = vec4<f32>(prev, pt);
        reduce_bbox(pi, pt);
        prev = pt;
    }
}

// Strip bins contain (path, segment, strip). Entries are bounded by the
// control polygon's vertical variation and subdivision count on the host.
@compute @workgroup_size(64)
fn bin(@builtin(global_invocation_id) gid: vec3<u32>) {
    if invocation(gid) >= params.element_total { return; }
    let el = elements[invocation(gid)];
    let pi = el.head.z;
    bin_segments(pi, paths[pi].seg_base + el.head.w, el.head.y);
}

// Bin `count` segments of path `pi` starting at segment `first`.
fn bin_segments(pi: u32, first: u32, count: u32) {
    let rect = grid_rect(pi);
    if rect.z == 0.0 || rect.w == 0.0 { return; }
    for (var k = 0u; k < count; k += 1u) {
        let si = first + k;
        let s = segments[si];
        let lo = max(min(s.y, s.w) - rect.y, 0.0);
        let hi = min(max(s.y, s.w) - rect.y, rect.w);
        if hi < lo || lo >= rect.w || hi < 0.0 { continue; }
        let first_strip = u32(floor(lo / 4.0));
        let last_strip = min(u32(floor(hi / 4.0)), (u32(rect.w) - 1u) / 4u);
        for (var strip = first_strip; strip <= last_strip; strip += 1u) {
            let slot = atomicAdd(&work.bin_count, 1u);
            let base = params.bin_base + slot * 3u;
            coverage[base] = pi;
            coverage[base + 1u] = si;
            coverage[base + 2u] = strip;
        }
    }
}

// Clip one segment to a closed unit cell. Horizontal edges are retained:
// their left-boundary crossing is an event in the backdrop winding.
fn clip_piece(s: vec4<f32>, lo: vec2<f32>, hi: vec2<f32>) -> vec4<f32> {
    // Identical reversed edges must produce identical clipped geometry.
    let reverse = s.x > s.z || (s.x == s.z && s.y > s.w);
    let result = clip_forward(select(s, s.zwxy, reverse), lo, hi);
    return select(result, result.zwxy, reverse);
}
fn clip_forward(s: vec4<f32>, lo: vec2<f32>, hi: vec2<f32>) -> vec4<f32> {
    let a = s.xy;
    let d = s.zw - a;
    var t0 = 0.0;
    var t1 = 1.0;
    for (var axis = 0u; axis < 2u; axis += 1u) {
        if d[axis] == 0.0 {
            if a[axis] < lo[axis] || a[axis] > hi[axis] { return vec4<f32>(-2.0); }
        } else {
            let u = (lo[axis] - a[axis]) / d[axis];
            let v = (hi[axis] - a[axis]) / d[axis];
            t0 = max(t0, min(u, v));
            t1 = min(t1, max(u, v));
        }
    }
    if t1 <= t0 { return vec4<f32>(-2.0); }
    var start = clamp(a + d * t0, lo, hi);
    var end = clamp(a + d * t1, lo, hi);
    // Reconstructing a boundary by multiply/add may leave a tiny positive
    // residual. Preserve exact boundary identity for left-edge events.
    for (var axis = 0u; axis < 2u; axis += 1u) {
        if d[axis] != 0.0 {
            let u = (lo[axis] - a[axis]) / d[axis];
            let v = (hi[axis] - a[axis]) / d[axis];
            if t0 == u { start[axis] = lo[axis]; }
            if t0 == v { start[axis] = hi[axis]; }
            if t1 == u { end[axis] = lo[axis]; }
            if t1 == v { end[axis] = hi[axis]; }
        }
    }
    return vec4<f32>(start, end);
}

@compute @workgroup_size(64)
fn fragment_bins(@builtin(global_invocation_id) gid: vec3<u32>) {
    if invocation(gid) >= atomicLoad(&work.bin_count) { return; }
    let base = params.bin_base + invocation(gid) * 3u;
    let pi = coverage[base];
    let segment = coverage[base + 1u];
    let s = segments[segment];
    let strip = coverage[base + 2u];
    let rect = grid_rect(pi);
    let a = s.xy - rect.xy;
    let b = s.zw - rect.xy;
    let d = b - a;
    for (var row = strip * 4u; row < min(strip * 4u + 4u, u32(rect.w)); row += 1u) {
        let y = f32(row);
        // The edge's directed crossing of this row's top boundary. Its
        // column is not decided from the exact crossing point: the cells
        // read the clipped fragments below, whose column-boundary points
        // round, and an edge passing within rounding of a pixel corner
        // reaches the boundary on the other side of it there. A left edge
        // event then disagrees with the carried winding by one and fills
        // or empties the whole cell (water-rs/waterui#3158). The crossing
        // belongs to the cell whose fragment leaves the boundary into the row.
        let crosses = y >= min(a.y, b.y) && y < max(a.y, b.y);
        let crossing = bitcast<u32>(select(-1i, 1i, d.y > 0.0));
        var claimed = !crosses;
        if max(a.y, b.y) < y || min(a.y, b.y) >= y + 1.0 { continue; }
        // Clip to the row before traversing columns: O(touched cells),
        // never the two-dimensional bounding box of a diagonal segment.
        let piece = clip_piece(vec4<f32>(a, b), vec2<f32>(0.0, y), vec2<f32>(rect.z, y + 1.0));
        if piece.x != -2.0 {
            let first = min(u32(floor(min(piece.x, piece.z))), u32(rect.z) - 1u);
            let last = min(u32(floor(max(piece.x, piece.z))), u32(rect.z) - 1u);
            for (var col = first; col <= last; col += 1u) {
                let origin = vec2<f32>(f32(col), y);
                let clipped = clip_piece(piece, origin, origin + vec2<f32>(1.0));
                if clipped.x == -2.0 { continue; }
                let key = paths[pi].grid_base + strip * paths[pi].grid_w + col;
                let node = atomicAdd(&work.fragment_count, 1u);
                let dst = params.fragment_base + node * 8u;
                let local = clipped - vec4<f32>(origin, origin);
                // Rounding is monotone along the piece, so exactly one
                // fragment leaves the top boundary; the ones before it lie
                // on the boundary and carry no winding.
                let owns = !claimed && local.y != local.w && (local.y == 0.0 || local.w == 0.0);
                claimed = claimed || owns;
                coverage[dst] = bitcast<u32>(local.x);
                coverage[dst + 1u] = bitcast<u32>(local.y);
                coverage[dst + 2u] = bitcast<u32>(local.z);
                coverage[dst + 3u] = bitcast<u32>(local.w);
                coverage[dst + 5u] = segment;
                coverage[dst + 6u] = row % 4u;
                coverage[dst + 7u] = select(0u, crossing, owns);
                coverage[params.sort_a + node * 2u] = key;
                coverage[params.sort_a + node * 2u + 1u] = node + 1u;
            }
        }
        if !claimed {
            // The edge leaves the boundary outside the grid: left of it
            // when the crossing lies at x < 0 or the in-grid piece runs
            // along the boundary to the left edge, else right of it,
            // where no column reads it.
            let reverse = a.x > b.x || (a.x == b.x && a.y > b.y);
            let origin = select(a, b, reverse);
            let direction = select(d, -d, reverse);
            let x = origin.x + direction.x * ((y - origin.y) / direction.y);
            if x < 0.0 || (piece.x != -2.0 && min(piece.x, piece.z) == 0.0) {
                atomicAdd(&work.words[paths[pi].row_base + row], crossing);
            }
        }
    }
}

fn fragment(node: u32) -> vec4<f32> {
    let base = params.fragment_base + (node - 1u) * 8u;
    return bitcast<vec4<f32>>(vec4<u32>(coverage[base], coverage[base + 1u], coverage[base + 2u], coverage[base + 3u]));
}
fn segment_of(node: u32) -> u32 {
    return coverage[params.fragment_base + (node - 1u) * 8u + 5u];
}
fn next_fragment(node: u32) -> u32 {
    return coverage[params.fragment_base + (node - 1u) * 8u + 4u];
}
fn inside(winding: i32, rule: u32) -> f32 {
    if rule == 0u { return select(0.0, 1.0, winding != 0i); }
    return f32(u32(winding) & 1u);
}
fn left_winding(head: u32, seed: i32, y: f32) -> i32 {
    var winding = seed;
    var node = head;
    while node != 0u {
        let s = fragment(node);
        // A segment entering the cell at its left edge removes one
        // directed crossing from the left ray; leaving adds one.
        if s.x == 0.0 && s.y > 0.0 && s.y <= y { winding -= 1i; }
        if s.z == 0.0 && s.w > 0.0 && s.w <= y { winding += 1i; }
        node = next_fragment(node);
    }
    return winding;
}

fn left_area(head: u32, seed: i32, rule: u32) -> f32 {
    var area = 0.0;
    var y = 0.0;
    // Integrate the left edge's piecewise-constant inside state.
    while y < 1.0 {
        var end = 1.0;
        var node = head;
        while node != 0u {
            let s = fragment(node);
            if s.x == 0.0 && s.y > y { end = min(end, s.y); }
            if s.z == 0.0 && s.w > y { end = min(end, s.w); }
            node = next_fragment(node);
        }
        area += inside(left_winding(head, seed, (y + end) * 0.5), rule) * (end - y);
        y = end;
    }
    return area;
}

// A non-horizontal fragment's line at the cell's top and bottom edges: the
// key that orders it among the cell's other fragments.
fn line_key(s: vec4<f32>) -> vec2<f32> {
    let slope = (s.z - s.x) / (s.w - s.y);
    return vec2<f32>(s.x - s.y * slope, s.x + (1.0 - s.y) * slope);
}

// Integrating each fragment's transition against the winding on its left
// telescopes to the cell's area only when, at every height, "left of" is a
// strict total order of the fragments there that both fragments of each
// pair agree on. Rounding brings fragments within an ulp of each other —
// stroke outlines retrace offset lines, so several distinct segments can
// clip to one line — and a relation judged per fragment, or a tolerance
// that calls two fragments one line, breaks either property: a transition
// is then applied twice or never (water-rs/waterui#3158). Lines are
// therefore ordered by comparing their keys, which every fragment computes
// with the same function, so the order is transitive; equal keys are one
// line, ordered by segment; and two lines whose order differs at the top
// and the bottom swap at one height, computed for the pair in segment order
// so both fragments read the same bits.
// Integer predicates avoid mixed boolean/float aggregate casts, which crash
// Mali pipeline compilation even in a standalone validated shader (#3372).
struct Order {
    // `other` lies left of the edge above `swap`.
    above: u32,
    // `other` lies left of the edge below `swap`.
    below: u32,
    // The height where the order swaps, F_INF when it does not.
    swap: f32,
}

fn order(node: u32, key: vec2<f32>, other: u32, other_key: vec2<f32>) -> Order {
    let tie = segment_of(other) < segment_of(node);
    // Ordered at an edge of the cell; a tie there defers to the other edge.
    let top = select(
        other_key.x < key.x,
        other_key.y < key.y || (other_key.y == key.y && tie),
        other_key.x == key.x,
    );
    let bottom = select(
        other_key.y < key.y,
        other_key.x < key.x || (other_key.x == key.x && tie),
        other_key.y == key.y,
    );
    var swap = F_INF;
    if top != bottom {
        // The lines cross inside the cell: solve in segment order.
        let first = select(key, other_key, tie);
        let second = select(other_key, key, tie);
        let g0 = second.x - first.x;
        let g1 = second.y - first.y;
        swap = g0 / (g0 - g1);
    }
    return Order(select(0u, 1u, top), select(0u, 1u, bottom), swap);
}

// Each fragment integrates its own boundary contribution. The next dispatch
// reduces those contributions in linked-list order.
fn edge_area(node: u32, head: u32, seed: i32, rule: u32) -> f32 {
    var area = 0.0;
    let edge = fragment(node);
    let a = edge.xy;
    let d = edge.zw - a;
    if d.y == 0.0 { return 0.0; }
    let key = line_key(edge);
    var t = 0.0;
    while t < 1.0 {
        var end = 1.0;
        var other = head;
        while other != 0u {
            if other != node {
                let s = fragment(other);
                var hit = F_INF;
                if s.y == s.w {
                    // A horizontal edge carries no transition, but where it
                    // crosses this one the winding on its left changes.
                    let x = key.x + (key.y - key.x) * s.y;
                    if x >= min(s.x, s.z) && x <= max(s.x, s.z) {
                        hit = (s.y - a.y) / d.y;
                    }
                } else {
                    let swap = order(node, key, other, line_key(s)).swap;
                    if swap != F_INF { hit = (swap - a.y) / d.y; }
                }
                if hit > t && hit < end { end = hit; }
            }
            other = next_fragment(other);
        }
        let midpoint = a + d * ((t + end) * 0.5);
        var winding = left_winding(head, seed, midpoint.y);
        other = head;
        while other != 0u {
            if other != node {
                let s = fragment(other);
                if midpoint.y >= min(s.y, s.w) && midpoint.y < max(s.y, s.w) {
                    let o = order(node, key, other, line_key(s));
                    if select(o.below, o.above, midpoint.y < o.swap) != 0u {
                        winding += select(-1i, 1i, s.w > s.y);
                    }
                }
            }
            other = next_fragment(other);
        }
        let transition = select(-1i, 1i, d.y > 0.0);
        let change = inside(winding + transition, rule) - inside(winding, rule);
        area += change * (1.0 - midpoint.x) * abs(d.y) * (end - t);
        t = end;
    }
    return area;
}

fn sort_key(index: u32) -> u32 { return coverage[params.sorted + index * 2u]; }
fn sort_node(index: u32) -> u32 { return coverage[params.sorted + index * 2u + 1u]; }
fn lower_bound(key: u32) -> u32 {
    var lo = 0u;
    var hi = fragment_count();
    while lo < hi {
        let mid = (lo + hi) / 2u;
        if sort_key(mid) < key { lo = mid + 1u; } else { hi = mid; }
    }
    return lo;
}
fn invocation(gid: vec3<u32>) -> u32 { return gid.x + gid.y * 16384u; }

fn work_item(index: u32) -> vec2<u32> {
    let base = params.row_total + index * 2u;
    return vec2<u32>(atomicLoad(&work.words[base]), atomicLoad(&work.words[base + 1u]));
}

fn fragment_count() -> u32 { return work.fragments; }

// The (x, y, 1) indirect dispatch arguments of a workgroup-64 kernel over
// `count` invocations, at most 256 workgroups wide.
fn write_dispatch_args(base: u32, count: u32) {
    let groups = (count + 63u) / 64u;
    args[base] = min(groups, 256u);
    args[base + 1u] = (groups + 255u) / 256u;
    args[base + 2u] = 1u;
}

// The counters bound execution; reserved capacities only determine storage.
@compute @workgroup_size(64)
fn prepare_dispatch(@builtin(local_invocation_index) lane: u32) {
    if lane >= 3u { return; }
    var count = atomicLoad(&work.bin_count);
    let fragments = atomicLoad(&work.fragment_count);
    if lane == 1u {
        work.fragments = fragments;
        count = fragments;
    }
    if lane == 2u { count = ((fragments + 16383u) / 16384u) * 16u * 64u; }
    write_dispatch_args(lane * 3u, count);
}

var<workgroup> radix_masks: array<atomic<u32>, 32>;

// Each lane contributes one bit to its digit's mask. The masks are local
// to a workgroup's range; completed masks become ordinary scatter input.
@compute @workgroup_size(64)
fn radix_count(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let i = invocation(gid);
    let group = i / 64u;
    if lane < 32u { atomicStore(&radix_masks[lane], 0u); }
    workgroupBarrier();
    if i < fragment_count() {
        let input = select(params.sort_a, params.sort_b, (params.radix_pass & 1u) != 0u);
        let digit = (coverage[input + i * 2u] >> (params.radix_pass * 4u)) & 15u;
        atomicOr(&radix_masks[digit * 2u + lane / 32u], 1u << (lane % 32u));
    }
    workgroupBarrier();
    if lane < 16u && group * 64u < fragment_count() {
        let low = atomicLoad(&radix_masks[lane * 2u]);
        let high = atomicLoad(&radix_masks[lane * 2u + 1u]);
        coverage[params.histogram + group * 16u + lane] = countOneBits(low) + countOneBits(high);
        let output = params.groups + group * 64u + lane * 2u;
        coverage[output] = low;
        coverage[output + 1u] = high;
    }
}
var<workgroup> radix_sums: array<u32, 64>;

@compute @workgroup_size(64)
fn radix_scan_blocks(@builtin(workgroup_id) gid: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let groups = (fragment_count() + 63u) / 64u;
    let blocks = (groups + 255u) / 256u;
    let i = gid.x + gid.y * 256u;
    let digit = i % 16u;
    let block = i / 16u;
    var counts = vec4<u32>(0u);
    for (var j = 0u; j < 4u; j += 1u) {
        let group = block * 256u + lane * 4u + j;
        if group < groups {
            counts[j] = coverage[params.histogram + group * 16u + digit];
        }
    }
    radix_sums[lane] = counts.x + counts.y + counts.z + counts.w;
    workgroupBarrier();
    for (var offset = 1u; offset < 64u; offset *= 2u) {
        var previous = 0u;
        if lane >= offset { previous = radix_sums[lane - offset]; }
        workgroupBarrier();
        radix_sums[lane] += previous;
        workgroupBarrier();
    }
    var sum = 0u;
    if lane > 0u { sum = radix_sums[lane - 1u]; }
    for (var j = 0u; j < 4u; j += 1u) {
        let group = block * 256u + lane * 4u + j;
        if group < groups {
            coverage[params.histogram + group * 16u + digit] = sum;
        }
        sum += counts[j];
    }
    if lane == 63u && block < blocks {
        coverage[params.groups + params.fragment_cap + 32u + i] = sum;
    }
}
var<workgroup> radix_totals: array<u32, 16>;

@compute @workgroup_size(64)
fn radix_prefix(@builtin(local_invocation_index) digit: u32) {
    let blocks = (fragment_count() + 16383u) / 16384u;
    let base = params.groups + params.fragment_cap + 32u;
    if digit < 16u {
        var sum = 0u;
        for (var block = 0u; block < blocks; block += 1u) {
            let index = base + block * 16u + digit;
            let count = coverage[index];
            coverage[index] = sum;
            sum += count;
        }
        radix_totals[digit] = sum;
    }
    workgroupBarrier();
    if digit < 16u {
        var start = 0u;
        for (var previous = 0u; previous < digit; previous += 1u) {
            start += radix_totals[previous];
        }
        // Bucket starts are shared by every fragment in the scatter.
        coverage[base + blocks * 16u + digit] = start;
    }
}
@compute @workgroup_size(64)
fn radix_scatter(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let i = invocation(gid);
    let input = select(params.sort_a, params.sort_b, (params.radix_pass & 1u) != 0u);
    let output = select(params.sort_b, params.sort_a, (params.radix_pass & 1u) != 0u);
    var key = 0xffffffffu;
    var node = 0u;
    if i < fragment_count() {
        key = coverage[input + i * 2u];
        node = coverage[input + i * 2u + 1u];
    }
    let digit = (key >> (params.radix_pass * 4u)) & 15u;
    if i >= fragment_count() { return; }
    let histogram = params.histogram + (i / 64u) * 16u;
    let masks = params.groups + (i / 64u) * 64u + digit * 2u;
    let low = coverage[masks];
    let high = coverage[masks + 1u];
    let preceding = (1u << (lane % 32u)) - 1u;
    var rank = countOneBits(low & preceding);
    if lane >= 32u { rank = countOneBits(low) + countOneBits(high & preceding); }
    let blocks = (fragment_count() + 16383u) / 16384u;
    let base = params.groups + params.fragment_cap + 32u;
    let bucket_start = coverage[base + blocks * 16u + digit];
    let block_start = coverage[base + (i / 16384u) * 16u + digit];
    let dst = bucket_start + block_start + coverage[histogram + digit] + rank;
    coverage[output + dst * 2u] = key;
    coverage[output + dst * 2u + 1u] = node;
}

fn group_address(index: u32) -> u32 {
    var slot = index;
    if params.grid_total < params.fragment_cap { slot = sort_key(index); }
    return params.groups + slot * 16u;
}

// One invocation per unique boundary group: link the group's fragments
// into per-row edge lists, record the next group's index and sum the
// group's per-row winding change. Only the winding carry across a strip's
// groups is left to the serial `prefix`.
@compute @workgroup_size(64)
fn link_groups(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = invocation(gid);
    if i >= fragment_count() { return; }
    let key = sort_key(i);
    if key == 0xffffffffu { return; }
    if i > 0u && sort_key(i - 1u) == key { return; }
    let group = group_address(i);
    // Radix scratch occupied this arena in earlier dispatches.
    for (var r = 0u; r < 4u; r += 1u) {
        coverage[group + 4u + r] = 0u;
    }
    var delta = vec4<u32>(0u);
    var end = i;
    while end < fragment_count() && sort_key(end) == key {
        let node = sort_node(end);
        let base = params.fragment_base + (node - 1u) * 8u;
        let row = coverage[base + 6u];
        // The frame memory bound leaves room for the two row bits.
        // Retain the group address for integration instead of searching again.
        coverage[base + 6u] = (group << 2u) | row;
        let previous = coverage[group + 4u + row];
        coverage[group + 4u + row] = node;
        coverage[base + 4u] = previous;
        delta[row] += coverage[base + 7u];
        end += 1u;
    }
    // Integration moves this link to the first consumed edge-head word.
    coverage[group] = end;
    // `prefix` replaces the change with the outgoing winding.
    for (var r = 0u; r < 4u; r += 1u) {
        coverage[group + 12u + r] = delta[r];
    }
}

// Local winding scans run in parallel across column tiles. Their four
// totals occupy the radix workspace until integration consumes the carries.
@compute @workgroup_size(64)
fn prefix_tiles(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = invocation(gid);
    if i >= params.tile_total { return; }
    let w = work_item(params.strip_total + i);
    let pi = w.x;
    let tiles = (paths[pi].grid_w + params.classify_width - 1u) / params.classify_width;
    let strip = w.y / tiles;
    let x = (w.y % tiles) * params.classify_width;
    let rect = grid_rect(pi);
    var winding = vec4<u32>(0u);
    if strip * 4u < u32(rect.w) && x < u32(rect.z) {
        let key0 = paths[pi].grid_base + strip * paths[pi].grid_w;
        let limit = lower_bound(key0 + min(x + params.classify_width, u32(rect.z)));
        var current = lower_bound(key0 + x);
        while current < limit {
            let group = group_address(current);
            for (var r = 0u; r < 4u; r += 1u) {
                coverage[group + 8u + r] = winding[r];
                winding[r] += coverage[group + 12u + r];
                coverage[group + 12u + r] = winding[r];
            }
            current = coverage[group];
        }
    }
    for (var r = 0u; r < 4u; r += 1u) {
        coverage[params.tile_prefix + i * 4u + r] = winding[r];
    }
}

// Only the short scan across tile totals remains serial per strip.
@compute @workgroup_size(64)
fn prefix(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = invocation(gid);
    if i >= params.strip_total { return; }
    let w = work_item(i);
    let pi = w.x;
    let rect = grid_rect(pi);
    let sy = w.y * 4u;
    if sy >= u32(rect.w) { return; }
    let tiles = (paths[pi].grid_w + params.classify_width - 1u) / params.classify_width;
    let base = paths[pi].tile_base + w.y * tiles;
    var winding = vec4<u32>(0u);
    for (var r = 0u; r < min(4u, u32(rect.w) - sy); r += 1u) {
        winding[r] = atomicLoad(&work.words[paths[pi].row_base + sy + r]);
    }
    for (var tile = 0u; tile < tiles; tile += 1u) {
        let offset = params.tile_prefix + (base + tile) * 4u;
        for (var r = 0u; r < 4u; r += 1u) {
            let delta = coverage[offset + r];
            coverage[offset + r] = winding[r];
            winding[r] += delta;
        }
    }
}

fn tile_carry_base(pi: u32, key: u32) -> u32 {
    let local = key - paths[pi].grid_base;
    let tiles = (paths[pi].grid_w + params.classify_width - 1u) / params.classify_width;
    let tile = paths[pi].tile_base + (local / paths[pi].grid_w) * tiles
        + (local % paths[pi].grid_w) / params.classify_width;
    return params.tile_prefix + tile * 4u;
}

fn path_for_key(key: u32) -> u32 {
    var lo = 0u;
    var hi = params.path_count;
    while lo + 1u < hi {
        let mid = (lo + hi) / 2u;
        if paths[mid].grid_base <= key { lo = mid; } else { hi = mid; }
    }
    return lo;
}

// The directed crossing delta at fragment word 7 has been consumed by
// link_groups and prefix. Reuse it without enlarging the coverage arena.
@compute @workgroup_size(64)
fn integrate_edges(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = invocation(gid);
    if i >= fragment_count() { return; }
    let key = sort_key(i);
    if key == 0xffffffffu { return; }
    let node = sort_node(i);
    let base = params.fragment_base + (node - 1u) * 8u;
    let edge = fragment(node);
    if edge.y == edge.w {
        coverage[base + 7u] = 0u;
        return;
    }
    let location = coverage[base + 6u];
    let row = location & 3u;
    let group = location >> 2u;
    let head = coverage[group + 4u + row];
    let pi = path_for_key(key);
    let seed = bitcast<i32>(coverage[group + 8u + row] + coverage[tile_carry_base(pi, key) + row]);
    let value = edge_area(node, head, seed, paths[pi].rule);
    coverage[base + 7u] = bitcast<u32>(value);
}

@compute @workgroup_size(64)
fn integrate(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = invocation(gid);
    // Token storage aliases the completed radix workspace.
    if i == 0u { atomicStore(&work.token_count, 0u); }
    if i >= fragment_count() { return; }
    let key = sort_key(i);
    if key == 0xffffffffu { return; }
    if i > 0u && sort_key(i - 1u) == key { return; }
    let pi = path_for_key(key);
    let group = group_address(i);
    let next = coverage[group];
    let carry_base = tile_carry_base(pi, key);
    for (var row = 0u; row < 4u; row += 1u) {
        let head = coverage[group + 4u + row];
        let carry = coverage[carry_base + row];
        let seed = bitcast<i32>(coverage[group + 8u + row] + carry);
        coverage[group + 12u + row] += carry;
        var value = inside(seed, paths[pi].rule);
        if head != 0u {
            value = left_area(head, seed, paths[pi].rule);
            var node = head;
            while node != 0u {
                value += bitcast<f32>(coverage[params.fragment_base + (node - 1u) * 8u + 7u]);
                node = next_fragment(node);
            }
            value = clamp(value, 0.0, 1.0);
        }
        coverage[group + row] = bitcast<u32>(value);
    }
    // No later stage reads edge-list heads; retain the sparse traversal link.
    coverage[group + 4u] = next;
}

fn group_words(index: u32, offset: u32) -> vec4<u32> {
    let base = group_address(index) + offset;
    return vec4<u32>(coverage[base], coverage[base + 1u],
        coverage[base + 2u], coverage[base + 3u]);
}
fn winding_values(winding: vec4<u32>, rule: u32) -> vec4<f32> {
    return vec4<f32>(inside(bitcast<i32>(winding.x), rule), inside(bitcast<i32>(winding.y), rule),
        inside(bitcast<i32>(winding.z), rule), inside(bitcast<i32>(winding.w), rule));
}

// Tokens describe maximal stretches with one column classification. A
// long interior gap costs one token, never width*height scratch storage.
fn token_word(base: u32, token: u32, field: u32) -> u32 { return params.tokens + (base + token) * 3u + field; }
fn token_get(base: u32, token: u32, field: u32) -> u32 { return coverage[token_word(base, token, field)]; }
fn token_class(base: u32, token: u32) -> u32 { return token_get(base, token, 2u); }
fn append_token(base: u32, count: u32, x0: u32, x1: u32, class_: u32) {
    coverage[token_word(base, count, 0u)] = x0;
    coverage[token_word(base, count, 1u)] = x1;
    coverage[token_word(base, count, 2u)] = class_;
}
fn coverage_class(values: vec4<f32>, sh: u32) -> u32 {
    var empty = true;
    var full = true;
    for (var r = 0u; r < sh; r += 1u) {
        let v = values[r];
        empty = empty && v == 0.0;
        full = full && v == 1.0;
    }
    return select(select(2u, 1u, full), 0u, empty);
}
@compute @workgroup_size(64)
fn classify(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = invocation(gid);
    if i >= params.tile_total { return; }
    let w = work_item(params.strip_total + i);
    let pi = w.x;
    let tiles_per_strip = (paths[pi].grid_w + params.classify_width - 1u) / params.classify_width;
    let strip = w.y / tiles_per_strip;
    let tile = w.y % tiles_per_strip;
    let rect = grid_rect(pi);
    var width = u32(rect.z);
    let height = u32(rect.w);
    let sy = strip * 4u;
    if width == 0u || sy >= height || tile * params.classify_width >= width { return; }
    if width <= 32u && height <= 32u {
        if strip == 0u { emit_cell(pi, pi, rect, 0u, width, 0u, height, 0u); }
        return;
    }
    let sh = min(4u, height - sy);
    let key0 = paths[pi].grid_base + strip * paths[pi].grid_w;
    let strip_start = lower_bound(key0);
    let strip_limit = lower_bound(key0 + width);
    let start = lower_bound(key0 + tile * params.classify_width);
    let limit = lower_bound(key0 + min((tile + 1u) * params.classify_width, width));
    if start == limit && (strip_start != strip_limit || tile != 0u) { return; }
    // A chunk owns its boundary columns and the following gap, so large
    // full interiors stay single spans even when they cross many tiles.
    if limit < strip_limit { width = sort_key(limit) - key0; }
    var columns = 0u;
    var cursor = start;
    while cursor < limit {
        columns += 1u;
        cursor = coverage[group_address(cursor) + 4u];
    }
    let leading = u32(start == strip_start);
    let base = atomicAdd(&work.token_count, columns * 2u + leading);
    var count = 0u;
    var x = 0u;
    if start != strip_start { x = sort_key(start) - key0; }
    var index = start;
    var winding = vec4<u32>(0u);
    for (var r = 0u; r < sh; r += 1u) {
        winding[r] = atomicLoad(&work.words[paths[pi].row_base + sy + r]);
    }
    if start > strip_start {
        winding = group_words(lower_bound(sort_key(start - 1u)), 12u);
    }
    while index < limit {
        let col = sort_key(index) - key0;
        if x < col {
            append_token(base, count, x, col, coverage_class(winding_values(winding, paths[pi].rule), sh));
            count += 1u;
        }
        append_token(base, count, col, col + 1u, coverage_class(bitcast<vec4<f32>>(group_words(index, 0u)), sh));
        count += 1u;
        x = col + 1u;
        winding = group_words(index, 12u);
        index = coverage[group_address(index) + 4u];
    }
    if x < width {
        append_token(base, count, x, width, coverage_class(winding_values(winding, paths[pi].rule), sh));
        count += 1u;
    }
    // Demote short full runs, including runs spanning several tokens.
    var token = 0u;
    while token < count {
        if token_class(base, token) != 1u { token += 1u; continue; }
        var end = token + 1u;
        while end < count && token_class(base, end) == 1u { end += 1u; }
        if token_get(base, end - 1u, 1u) - token_get(base, token, 0u) < 8u {
            for (var j = token; j < end; j += 1u) { coverage[token_word(base, j, 2u)] = 2u; }
        }
        token = end;
    }
    token = 0u;
    while token < count {
        if token_class(base, token) != 0u { token += 1u; continue; }
        var end = token + 1u;
        while end < count && token_class(base, end) == 0u { end += 1u; }
        if token > 0u && end < count && token_class(base, token - 1u) == 2u && token_class(base, end) == 2u
            && token_get(base, end - 1u, 1u) - token_get(base, token, 0u) <= 8u {
            for (var j = token; j < end; j += 1u) { coverage[token_word(base, j, 2u)] = 2u; }
        }
        token = end;
    }
    token = 0u;
    while token < count {
        let class_ = token_class(base, token);
        if class_ == 0u { token += 1u; continue; }
        let x0 = token_get(base, token, 0u);
        var end = token + 1u;
        while end < count && token_class(base, end) == class_ { end += 1u; }
        if class_ == 1u {
            emit_span(pi, pi, rect, x0, token_get(base, end - 1u, 1u), sy, sh);
            token = end;
            continue;
        }
        var interior = 0u;
        // The CPU emission's greedy partial/full/partial merge retains
        // exactly one full interval; it is implicit in buffer storage.
        if end < count && token_class(base, end) == 1u {
            let full_start = end;
            var full_end = end + 1u;
            while full_end < count && token_class(base, full_end) == 1u { full_end += 1u; }
            if full_end < count && token_class(base, full_end) == 2u {
                interior = (token_get(base, full_start, 0u) - x0) | ((token_get(base, full_end - 1u, 1u) - x0) << 16u);
                end = full_end + 1u;
                while end < count && token_class(base, end) == 2u { end += 1u; }
            }
        }
        emit_cell(pi, pi, rect, x0, token_get(base, end - 1u, 1u), sy, sy + sh, interior);
        token = end;
    }
}

// One emitted span or cell's 32-byte geometry record in the path's
// arena block: the device bounds vec4, then the block-relative texel
// offset, pixel width, packed interior run and kind before the draw's
// REGION promotion — the vertex stage joins these to the draw's anchor.
fn emit_geometry(pi: u32, slot: u32, bounds: vec4<f32>, uv: vec2<u32>, kind: u32, interior: u32) {
    let dst = paths[pi].inst_base + slot * 8u;
    arena[dst] = bitcast<u32>(bounds.x);
    arena[dst + 1u] = bitcast<u32>(bounds.y);
    arena[dst + 2u] = bitcast<u32>(bounds.z);
    arena[dst + 3u] = bitcast<u32>(bounds.w);
    arena[dst + 4u] = uv.x;
    arena[dst + 5u] = uv.y;
    arena[dst + 6u] = interior;
    arena[dst + 7u] = kind;
}

fn emit_span(pi: u32, path_i: u32, rect: vec4<f32>, x0: u32, x1: u32, sy: u32, sh: u32) {
    let slot = atomicAdd(&paths[path_i].atom[4], 1u);
    let b = vec4<f32>(rect.x + f32(x0), rect.y + f32(sy), rect.x + f32(x1), rect.y + f32(sy + sh));
    emit_geometry(pi, slot, b, vec2<u32>(0u), KIND_SPAN, 0u);
}

// Classification only reserves a cell's geometry slot and texel words;
// `fill_cells` writes the texels with one workgroup per cell, so a strip's
// cells no longer serialize behind its owner invocation.
fn emit_cell(pi: u32, path_i: u32, rect: vec4<f32>, x0: u32, x1: u32, y0: u32, y1: u32, interior: u32) {
    let slot = atomicAdd(&paths[path_i].atom[4], 1u);
    let cw = x1 - x0;
    let stored_width = cw - ((interior >> 16u) - (interior & 65535u));
    let words = (stored_width * (y1 - y0) + 3u) / 4u;
    let tex_off = atomicAdd(&paths[path_i].atom[6], words);
    let b = vec4<f32>(rect.x + f32(x0), rect.y + f32(y0), rect.x + f32(x1), rect.y + f32(y1));
    emit_geometry(pi, slot, b, vec2<u32>(tex_off, cw), KIND_GLYPH, interior);
}

// Compact the per-path instance counts into a dispatch domain. Geometry
// stays in each path's reserved range; only the workgroup indices are packed.
@compute @workgroup_size(64)
fn prepare_cells(@builtin(local_invocation_index) lane: u32) {
    if lane != 0u { return; }
    var total = 0u;
    for (var pi = 0u; pi < params.path_count; pi += 1u) {
        atomicStore(&paths[pi].atom[7], total);
        total += atomicLoad(&paths[pi].atom[4]);
    }
    let base = 9u;
    args[base] = min(total, 256u);
    args[base + 1u] = (total + 255u) / 256u;
    args[base + 2u] = 1u;
}

// One workgroup per emitted instance. Spans return without writing texels.
@compute @workgroup_size(64)
fn fill_cells(@builtin(workgroup_id) gid: vec3<u32>, @builtin(local_invocation_index) lane_index: u32) {
    let i = gid.x + gid.y * 256u;
    var lo = 0u;
    var hi = params.path_count;
    while lo + 1u < hi {
        let mid = (lo + hi) / 2u;
        if atomicLoad(&paths[mid].atom[7]) <= i { lo = mid; } else { hi = mid; }
    }
    let pi = lo;
    let local = i - atomicLoad(&paths[pi].atom[7]);
    if local >= atomicLoad(&paths[pi].atom[4]) { return; }
    let dst = paths[pi].inst_base + local * 8u;
    if arena[dst + 7u] == KIND_SPAN { return; }
    let rect = grid_rect(pi);
    let bounds = vec4<f32>(
        bitcast<f32>(arena[dst]), bitcast<f32>(arena[dst + 1u]),
        bitcast<f32>(arena[dst + 2u]), bitcast<f32>(arena[dst + 3u]));
    let x0 = u32(bounds.x - rect.x);
    let y0 = u32(bounds.y - rect.y);
    let ch = u32(bounds.w - rect.y) - y0;
    let interior = vec2<u32>(arena[dst + 6u] & 0xffffu, arena[dst + 6u] >> 16u);
    let interior_start = interior.x;
    let interior_end = interior.y;
    let stored_width = arena[dst + 5u] - (interior_end - interior_start);
    let tex = paths[pi].texel_base + arena[dst + 4u];
    let words = (stored_width * ch + 3u) / 4u;
    // Each lane walks a contiguous range so adjacent writes reuse the sparse
    // cursor without traversing columns assigned to other lanes.
    var previous_row = 0xffffffffu;
    var index = 0u;
    var winding = 0u;
    var key0 = 0u;
    let words_per_lane = (words + 63u) / 64u;
    let first_word = lane_index * words_per_lane;
    let last_word = min(first_word + words_per_lane, words);
    for (var word = first_word; word < last_word; word += 1u) {
        var packed = 0u;
        for (var lane = 0u; lane < 4u; lane += 1u) {
            let pixel = word * 4u + lane;
            if pixel < stored_width * ch {
                let row = pixel / stored_width;
                let packed_col = pixel % stored_width;
                let col = packed_col + select(0u, interior_end - interior_start, packed_col >= interior_start);
                let strip = (y0 + row) / 4u;
                let strip_row = (y0 + row) % 4u;
                if row != previous_row {
                    key0 = paths[pi].grid_base + strip * paths[pi].grid_w;
                    index = lower_bound(key0 + x0 + col);
                    winding = atomicLoad(&work.words[paths[pi].row_base + y0 + row]);
                    if index > 0u && sort_key(index - 1u) >= key0 {
                        let previous = lower_bound(sort_key(index - 1u));
                        winding = coverage[group_address(previous) + 12u + strip_row];
                    }
                    previous_row = row;
                }
                let key = key0 + x0 + col;
                while index < fragment_count() && sort_key(index) < key {
                    let group = group_address(index);
                    winding = coverage[group + 12u + strip_row];
                    index = coverage[group + 4u];
                }
                var v = inside(bitcast<i32>(winding), paths[pi].rule);
                if index < fragment_count() && sort_key(index) == key {
                    v = bitcast<f32>(coverage[group_address(index) + strip_row]);
                }
                let q = u32(clamp(v, 0.0, 1.0) * 255.0 + 0.5);
                packed |= q << (lane * 8u);
            }
        }
        arena[tex + word] = packed;
    }
}

// ---- block headers ---------------------------------------------------------

// One invocation per path: the block's count word supplies the frame's
// per-draw indirect records through the prologue's 4-byte copies
// (water-rs/waterui#3081).
@compute @workgroup_size(64)
fn emit_counts(@builtin(global_invocation_id) gid: vec3<u32>) {
    if invocation(gid) >= params.path_count {
        return;
    }
    let pi = invocation(gid);
    arena[paths[pi].inst_base - 4u] = atomicLoad(&paths[pi].atom[4]);
}