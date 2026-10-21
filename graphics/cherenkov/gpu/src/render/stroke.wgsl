// The compute stroker (water-rs/waterui#2544): expands the stroke records the
// CPU encoder emits into outline chords, for undashed and dashed strokes.
// The module is compiled behind the stroke ABI prelude and `coverage.wgsl`,
// whose bindings, `Params`, `GpuPath`, `xf`, `cubic_at`, `cubic_deriv`,
// `invocation` and `bin_segments` it uses. Every chord goes to the segment
// slots the CPU assigned (undashed) or the GPU scanned (dashed), so the
// existing bin, sort and integrate stages see an ordinary segment list.
//
// No result that decides a count, a slot or a tie is IEEE-exact f32:
// positions along a dashed subpath are 2^-32 fixed point and are compared
// exactly, and every float two passes need is computed once and stored.

// The stroke records, six vec4s each (`StrokeRecord`), followed by the dash
// tables: `2 * (entries + 1)` words per dashed path of fixed-point boundaries.
// One raw binding keeps the layout inside the eight storage buffers a default
// device allows per stage.
@group(0) @binding(7) var<storage, read> strokes: array<vec4<u32>>;

// `c` is read as raw bits: a CURVE's control points, a POINT_ARC's sweep
// (whose `yz` carry a dashed one's fixed-point length), a dashed LINE's
// fixed-point length — any bits are legal.
struct StrokeRecord {
    head: vec4<u32>,
    link: vec4<u32>,
    aux: vec4<u32>,
    p: vec4<f32>,
    q: vec4<f32>,
    c: vec4<u32>,
}

const PI: f32 = 3.14159265358979;
const NONE: u32 = 0xffffffffu;
const FRAG_FLAGS: u32 = (1u << FRAG_RECORD_SHIFT) - 1u;
// 2^-32, the weight of a fixed-point fraction.
const FX_UNIT: f32 = 2.3283064365386963e-10;

fn load_record(i: u32) -> StrokeRecord {
    let b = i * 6u;
    return StrokeRecord(
        strokes[b],
        strokes[b + 1u],
        strokes[b + 2u],
        bitcast<vec4<f32>>(strokes[b + 3u]),
        bitcast<vec4<f32>>(strokes[b + 4u]),
        strokes[b + 5u],
    );
}

// The end tangent of record `i`.
fn end_tangent(i: u32) -> vec2<f32> {
    return bitcast<vec2<f32>>(strokes[i * 6u + 4u].zw);
}

fn piece_kind(r: StrokeRecord) -> u32 { return r.head.x & PIECE_MASK; }
fn join_code(r: StrokeRecord) -> u32 { return (r.head.x >> JOIN_SHIFT) & JOIN_MASK; }
fn is_dashed(pi: u32) -> bool { return paths[pi].dash_base != NONE; }

fn cross2(a: vec2<f32>, b: vec2<f32>) -> f32 { return a.x * b.y - a.y * b.x; }

// kurbo's `norm`: hw / |T| * (-T.y, T.x). The forward side is P - norm
// (d = -hw), the backward side P + norm.
fn offset_point(p: vec2<f32>, t: vec2<f32>, d: f32) -> vec2<f32> {
    return p + (d / length(t)) * vec2<f32>(-t.y, t.x);
}

fn rot(v: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * v.x - s * v.y, s * v.x + c * v.y);
}

// kurbo `do_join`'s miter point: the offset lines of the two tangents meet
// at `fp_this - cd * h`.
fn miter_point(p0: vec2<f32>, tp: vec2<f32>, t0: vec2<f32>, d: f32) -> vec2<f32> {
    let fp_last = offset_point(p0, tp, d);
    let fp_this = offset_point(p0, t0, d);
    let h = cross2(tp, fp_this - fp_last) / cross2(tp, t0);
    return fp_this - t0 * h;
}

fn curve_c0(r: StrokeRecord) -> vec2<f32> { return bitcast<vec2<f32>>(r.c.xy); }
fn curve_c1(r: StrokeRecord) -> vec2<f32> { return bitcast<vec2<f32>>(r.c.zw); }

// ---- vertex scratch -------------------------------------------------------

// A piece-end vertex: the point in user space and under the path's raster.
struct Vtx {
    local: vec2<f32>,
    device: vec2<f32>,
}

fn make_vertex(pi: u32, local: vec2<f32>) -> Vtx {
    return Vtx(local, xf(pi, local));
}

// Slot `rel` of record `i` lives at `coverage[(SLOTS * i + slot) * SLOT_WORDS]`:
// local xy, then device xy. Every vertex two chains share is written once by
// `stroke_vertices` and only read afterwards, so contours close bit-exactly.
fn slot_word(pi: u32, rel: u32) -> u32 {
    return (paths[pi].stroke_base * SLOTS + rel) * SLOT_WORDS;
}

fn put_vertex(pi: u32, rel: u32, local: vec2<f32>) {
    let w = slot_word(pi, rel);
    let device = xf(pi, local);
    coverage[w] = bitcast<u32>(local.x);
    coverage[w + 1u] = bitcast<u32>(local.y);
    coverage[w + 2u] = bitcast<u32>(device.x);
    coverage[w + 3u] = bitcast<u32>(device.y);
}

fn vertex(pi: u32, rel: u32) -> Vtx {
    let w = slot_word(pi, rel);
    return Vtx(
        vec2<f32>(bitcast<f32>(coverage[w]), bitcast<f32>(coverage[w + 1u])),
        vec2<f32>(bitcast<f32>(coverage[w + 2u]), bitcast<f32>(coverage[w + 3u])),
    );
}

// The end vertices of a record: its own slots, except a JOIN_ONLY piece,
// whose ends are its start vertices.
fn end_slots(r: StrokeRecord, rel: u32) -> vec2<u32> {
    if piece_kind(r) == PIECE_JOIN_ONLY { return r.link.zw; }
    return vec2<u32>(rel * SLOTS + SLOT_FE, rel * SLOTS + SLOT_BE);
}

// ---- chains ---------------------------------------------------------------

// Writes consecutive chords of one outline side. A backward chain stores each
// chord reversed, so the contour keeps one orientation.
struct Chain {
    pi: u32,
    out: u32,
    prev: vec2<f32>,
    reverse: bool,
}

// A chain starting at the device point `from`; `out` is path-relative.
fn chain_start(pi: u32, out: u32, src: vec2<f32>, reverse: bool) -> Chain {
    reduce_bbox(pi, src);
    return Chain(pi, out, src, reverse);
}

fn chain_to(c: ptr<function, Chain>, device: vec2<f32>) {
    let pi = (*c).pi;
    let prev = (*c).prev;
    segments[paths[pi].seg_base + (*c).out] = select(
        vec4<f32>(prev, device),
        vec4<f32>(device, prev),
        (*c).reverse,
    );
    reduce_bbox(pi, device);
    (*c).prev = device;
    (*c).out = (*c).out + 1u;
}

fn chain_local(c: ptr<function, Chain>, local: vec2<f32>) {
    chain_to(c, xf((*c).pi, local));
}

// ---- vertices -------------------------------------------------------------

// One invocation per record: the vertices it owns (section 5.2).
@compute @workgroup_size(64)
fn stroke_vertices(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = invocation(gid);
    if i >= params.stroke_total { return; }
    let r = load_record(i);
    let pi = r.head.z;
    let hw = paths[pi].half_width;
    let rel = i - paths[pi].stroke_base;
    let owns_start = join_code(r) != JOIN_SHARED && (r.head.x & CLOSE) == 0u;
    if owns_start {
        put_vertex(pi, rel * SLOTS + SLOT_FS, offset_point(r.p.xy, r.p.zw, -hw));
        put_vertex(pi, rel * SLOTS + SLOT_BS, offset_point(r.p.xy, r.p.zw, hw));
    }
    if piece_kind(r) != PIECE_JOIN_ONLY {
        put_vertex(pi, rel * SLOTS + SLOT_FE, offset_point(r.q.xy, r.q.zw, -hw));
        put_vertex(pi, rel * SLOTS + SLOT_BE, offset_point(r.q.xy, r.q.zw, hw));
    }
}

// ---- emission -------------------------------------------------------------

fn start_cap(pi: u32) -> u32 { return paths[pi].cap_word & 3u; }
fn end_cap(pi: u32) -> u32 { return (paths[pi].cap_word >> 2u) & 3u; }
fn round_cap_chords(pi: u32) -> u32 { return paths[pi].cap_word >> 8u; }

fn cap_chords(pi: u32, kind: u32) -> u32 {
    return select(select(round_cap_chords(pi), 3u, kind == CAP_SQUARE), 1u, kind == CAP_BUTT);
}

// Chords one side of the record's join takes.
fn join_chords(r: StrokeRecord) -> u32 {
    let code = join_code(r);
    if code == JOIN_BEVEL { return 1u; }
    if code == JOIN_MITER { return 2u; }
    if code == JOIN_ROUND { return r.aux.x & 0xffffu; }
    return 0u;
}

// Whether the record's join draws chords: every join but START or SHARED
// takes at least one chord per side.
fn has_join(r: StrokeRecord) -> bool { return join_code(r) >= JOIN_BEVEL; }

// One side of the record's join, from the previous record's end vertex to this
// record's start vertex (kurbo `do_join`, decided in f32). Both sides take the
// same chord count whichever way the kernel decides.
fn emit_join_side(c: ptr<function, Chain>, r: StrokeRecord, tp: vec2<f32>, bwd: bool) {
    let pi = (*c).pi;
    let code = join_code(r);
    let prev = vertex(pi, select(r.link.x, r.link.y, bwd));
    let cur = vertex(pi, select(r.link.z, r.link.w, bwd));
    if code == JOIN_BEVEL {
        chain_to(c, cur.device);
        return;
    }
    let p0 = r.p.xy;
    let t0 = r.p.zw;
    let cr = cross2(tp, t0);
    let dt = dot(tp, t0);
    if code == JOIN_MITER {
        let hy = length(vec2<f32>(cr, dt));
        let ml = paths[pi].miter_limit;
        if 2.0 * hy < (hy + dt) * ml * ml && cr != 0.0 {
            // The outer side is the forward one for a left turn.
            if bwd == (cr < 0.0) {
                let d = select(-paths[pi].half_width, paths[pi].half_width, bwd);
                chain_local(c, miter_point(p0, tp, t0, d));
            } else {
                chain_local(c, p0);
            }
        } else {
            chain_local(c, 0.5 * (prev.local + cur.local));
        }
        chain_to(c, cur.device);
        return;
    }
    // JOIN_ROUND: the arc side turns with the tangent, the other side takes
    // the bevel chord in as many equal pieces.
    let k = r.aux.x & 0xffffu;
    // Not `atan2(cr, dt)`: it is wrong at `dt == 0` (perpendicular tangents)
    // on the fast-math shader path, and turned that round join into a bevel.
    let phi = select(-1.0, 1.0, cr > 0.0) * acos(clamp(dt / length(vec2<f32>(cr, dt)), -1.0, 1.0));
    let v0 = prev.local - p0;
    let arc = bwd == (phi <= 0.0);
    for (var j = 1u; j < k; j += 1u) {
        let u = f32(j) / f32(k);
        if arc {
            chain_local(c, p0 + rot(v0, phi * u));
        } else {
            chain_local(c, mix(prev.local, cur.local, u));
        }
    }
    chain_to(c, cur.device);
}

fn write_join(pi: u32, out: u32, r: StrokeRecord, tp: vec2<f32>, bwd: bool) -> u32 {
    let src = vertex(pi, select(r.link.x, r.link.y, bwd));
    var c = chain_start(pi, out, src.device, bwd);
    emit_join_side(&c, r, tp, bwd);
    return c.out;
}

// One side's `n` offset chords over [ta, tb] of the record's piece, ending at
// `to`; the chain already sits at the start vertex.
fn emit_offset(c: ptr<function, Chain>, r: StrokeRecord, bwd: bool, ta: f32, tb: f32, n: u32, to: vec2<f32>) {
    let pi = (*c).pi;
    let d = select(-paths[pi].half_width, paths[pi].half_width, bwd);
    let kind = piece_kind(r);
    var v0 = vec2<f32>(0.0);
    if kind == PIECE_POINT_ARC { v0 = vertex(pi, select(r.link.z, r.link.w, bwd)).local - r.p.xy; }
    for (var j = 1u; j < n; j += 1u) {
        let t = ta + (tb - ta) * (f32(j) / f32(n));
        if kind == PIECE_CURVE {
            let c0 = curve_c0(r);
            let c1 = curve_c1(r);
            chain_local(c, offset_point(cubic_at(r.p.xy, c0, c1, r.q.xy, t), cubic_deriv(r.p.xy, c0, c1, r.q.xy, t), d));
        } else {
            chain_local(c, r.p.xy + rot(v0, bitcast<f32>(r.c.x) * t));
        }
    }
    chain_to(c, to);
}

fn write_offset(pi: u32, out: u32, r: StrokeRecord, bwd: bool, ta: f32, tb: f32, n: u32, src: Vtx, to: Vtx) -> u32 {
    var c = chain_start(pi, out, src.device, bwd);
    emit_offset(&c, r, bwd, ta, tb, n, to.device);
    return c.out;
}

// A cap from `from` to `to` around `ctr` (kurbo's butt, square and round
// constructions); the chain already sits at `from`.
fn emit_cap(c: ptr<function, Chain>, kind: u32, k: u32, src: Vtx, to: Vtx, ctr: vec2<f32>) {
    if kind == CAP_SQUARE {
        let nv = 0.5 * (src.local - to.local);
        let np = vec2<f32>(-nv.y, nv.x);
        chain_local(c, ctr + nv + np);
        chain_local(c, ctr - nv + np);
    } else if kind == CAP_ROUND {
        let v0 = src.local - ctr;
        for (var j = 1u; j < k; j += 1u) {
            chain_local(c, ctr + rot(v0, PI * f32(j) / f32(k)));
        }
    }
    chain_to(c, to.device);
}

fn write_cap(pi: u32, out: u32, kind: u32, src: Vtx, to: Vtx, ctr: vec2<f32>) -> u32 {
    var c = chain_start(pi, out, src.device, false);
    emit_cap(&c, kind, round_cap_chords(pi), src, to, ctr);
    return c.out;
}

// One invocation per undashed record, writing exactly `head.y` chords at its
// CPU-assigned slot in the order of section 5.2.
@compute @workgroup_size(64)
fn stroke_emit(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = invocation(gid);
    if i >= params.stroke_total { return; }
    let r = load_record(i);
    let pi = r.head.z;
    if is_dashed(pi) { return; }
    let rel = i - paths[pi].stroke_base;
    let kind = piece_kind(r);
    let ends = end_slots(r, rel);
    let fs = vertex(pi, r.link.z);
    let bs = vertex(pi, r.link.w);
    let fe = vertex(pi, ends.x);
    let be = vertex(pi, ends.y);
    let n = r.aux.x >> 16u;
    let joined = has_join(r);
    var tp = vec2<f32>(0.0);
    if joined { tp = end_tangent(i - 1u); }
    var out = r.head.w;
    if (r.head.x & CAP_START) != 0u {
        out = write_cap(pi, out, start_cap(pi), bs, fs, r.p.xy);
    }
    if joined { out = write_join(pi, out, r, tp, false); }
    if kind != PIECE_JOIN_ONLY { out = write_offset(pi, out, r, false, 0.0, 1.0, n, fs, fe); }
    if joined { out = write_join(pi, out, r, tp, true); }
    if kind != PIECE_JOIN_ONLY { out = write_offset(pi, out, r, true, 0.0, 1.0, n, bs, be); }
    if (r.head.x & CAP_END) != 0u {
        out = write_cap(pi, out, end_cap(pi), fe, be, r.q.xy);
    }
}

// One invocation per undashed record: bins its chords.
@compute @workgroup_size(64)
fn bin_strokes(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = invocation(gid);
    if i >= params.stroke_total { return; }
    let head = strokes[i * 6u];
    let pi = head.z;
    if is_dashed(pi) { return; }
    bin_segments(pi, paths[pi].seg_base + head.w, head.y);
}

// ---- fixed point and arc length -------------------------------------------

// The 64-bit product of two u32 as (high, low).
fn mul32(a: u32, b: u32) -> vec2<u32> {
    let a0 = a & 0xffffu;
    let a1 = a >> 16u;
    let b0 = b & 0xffffu;
    let b1 = b >> 16u;
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 16u) + (p01 & 0xffffu) + (p10 & 0xffffu);
    return vec2<u32>(
        p11 + (p01 >> 16u) + (p10 >> 16u) + (mid >> 16u),
        (p00 & 0xffffu) | (mid << 16u),
    );
}

// Fixed point is 2^-32 user units: (integer part, fraction).
fn fx_add(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let lo = a.y + b.y;
    return vec2<u32>(a.x + b.x + select(0u, 1u, lo < a.y), lo);
}
// a >= b.
fn fx_sub(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    return vec2<u32>(a.x - b.x - select(0u, 1u, a.y < b.y), a.y - b.y);
}
fn fx_lt(a: vec2<u32>, b: vec2<u32>) -> bool {
    return a.x < b.x || (a.x == b.x && a.y < b.y);
}
fn fx_eq(a: vec2<u32>, b: vec2<u32>) -> bool { return a.x == b.x && a.y == b.y; }
// The product stays below 2^32 units.
fn fx_mul(a: vec2<u32>, k: u32) -> vec2<u32> {
    let m = mul32(a.y, k);
    return vec2<u32>(a.x * k + m.x, m.y);
}
// x >= 0; exact where ulp(x) >= 2^-32.
fn fx_from(x: f32) -> vec2<u32> {
    let whole = floor(x);
    return vec2<u32>(u32(whole), u32((x - whole) * 4294967296.0));
}
fn fx_to(a: vec2<u32>) -> f32 { return f32(a.x) + f32(a.y) * FX_UNIT; }

fn speed(r: StrokeRecord, t: f32) -> f32 {
    return length(cubic_deriv(r.p.xy, curve_c0(r), curve_c1(r), r.q.xy, t));
}

// The 8-point Gauss-Legendre arc length of a CURVE piece over [a, b].
fn gauss_length(r: StrokeRecord, a: f32, b: f32) -> f32 {
    var nodes = GL_NODES;
    var weights = GL_WEIGHTS;
    let radius = 0.5 * (b - a);
    let mid = 0.5 * (a + b);
    var sum = 0.0;
    for (var k = 0u; k < 8u; k += 1u) {
        sum += weights[k] * speed(r, mid + radius * nodes[k]);
    }
    return sum * radius;
}

// The arc length of a CURVE piece over [0, t], on `aux.y` equal sub-intervals.
fn arclen(r: StrokeRecord, t: f32) -> f32 {
    let n = r.aux.y;
    let width = t / f32(n);
    var sum = 0.0;
    for (var i = 0u; i < n; i += 1u) {
        sum += gauss_length(r, width * f32(i), width * f32(i + 1u));
    }
    return sum;
}

// The parameter at arc length `sigma`: sub-interval search, then four clamped
// Newton steps on the sub-interval's own length (|c'| >= m > 0).
fn inv_arclen(r: StrokeRecord, sigma: f32) -> f32 {
    let n = r.aux.y;
    var done = 0.0;
    for (var i = 0u; i < n; i += 1u) {
        let t0 = f32(i) / f32(n);
        let t1 = f32(i + 1u) / f32(n);
        let piece = gauss_length(r, t0, t1);
        if sigma <= done + piece || i + 1u == n {
            let rest = clamp(sigma - done, 0.0, piece);
            var t = t0 + (t1 - t0) * (rest / piece);
            for (var k = 0u; k < 4u; k += 1u) {
                t = clamp(t - (gauss_length(r, t0, t) - rest) / speed(r, t), t0, t1);
            }
            return t;
        }
        done += piece;
    }
    return 1.0;
}

// The length of a record's piece in fixed point. A LINE carries the
// encoder's exact f64 length in `c.xy`, a POINT_ARC its sub-cubic's in
// `c.yz`; a CURVE is integrated here in f32.
fn piece_length(r: StrokeRecord) -> vec2<u32> {
    let kind = piece_kind(r);
    if kind == PIECE_LINE { return r.c.xy; }
    if kind == PIECE_CURVE { return fx_from(arclen(r, 1.0)); }
    if kind == PIECE_POINT_ARC { return r.c.yz; }
    return vec2<u32>(0u);
}

// The centre and tangent of a record's piece at parameter `t`.
struct Frame {
    centre: vec2<f32>,
    tangent: vec2<f32>,
}

fn frame_at(r: StrokeRecord, t: f32) -> Frame {
    let kind = piece_kind(r);
    if kind == PIECE_CURVE {
        let c0 = curve_c0(r);
        let c1 = curve_c1(r);
        return Frame(cubic_at(r.p.xy, c0, c1, r.q.xy, t), cubic_deriv(r.p.xy, c0, c1, r.q.xy, t));
    }
    if kind == PIECE_POINT_ARC {
        return Frame(r.p.xy, rot(r.p.zw, bitcast<f32>(r.c.x) * t));
    }
    return Frame(mix(r.p.xy, r.q.xy, t), r.p.zw);
}

// ---- dash passes ------------------------------------------------------------

// Per dashed record, `dash_words` (8 words): length (2), start s0 (2),
// fragment count, chord count, fragment offset, chord offset.
fn dash_word_index(j: u32) -> u32 { return params.dash_words_base + j * 8u; }

fn dash_fixed(w: u32) -> vec2<u32> {
    return vec2<u32>(coverage[w], coverage[w + 1u]);
}

@compute @workgroup_size(64)
fn dash_length(@builtin(global_invocation_id) gid: vec3<u32>) {
    let j = invocation(gid);
    if j >= params.dash_total { return; }
    let len = piece_length(load_record(params.dash_first + j));
    let w = dash_word_index(j);
    coverage[w] = len.x;
    coverage[w + 1u] = len.y;
}

// ---- the arc-length and offset scans ----------------------------------------

// A segmented-scan element: a restart flag and the value (a fixed-point
// length for op 0, fragments and chords for op 1).
struct ScanEl {
    flag: u32,
    v: vec2<u32>,
}

fn scan_identity() -> ScanEl { return ScanEl(0u, vec2<u32>(0u)); }

// Associative: op 0 restarts the fixed-point sum at a flagged element; op 1
// sums fragments globally and restarts only the chord sum.
fn scan_combine(l: ScanEl, r: ScanEl) -> ScanEl {
    let restart = r.flag != 0u;
    if params.scan_op == 0u {
        return ScanEl(l.flag | r.flag, select(fx_add(l.v, r.v), r.v, restart));
    }
    return ScanEl(l.flag | r.flag, vec2<u32>(l.v.x + r.v.x, select(l.v.y + r.v.y, r.v.y, restart)));
}

fn scan_element(j: u32) -> ScanEl {
    let w = dash_word_index(j);
    let i = params.dash_first + j;
    let head = strokes[i * 6u];
    if params.scan_op == 0u {
        let start = ((head.x >> JOIN_SHIFT) & JOIN_MASK) == JOIN_START;
        return ScanEl(select(0u, 1u, start), dash_fixed(w));
    }
    return ScanEl(select(0u, 1u, i == paths[head.z].stroke_base), vec2<u32>(coverage[w + 4u], coverage[w + 5u]));
}

// What the scan writes for an element given the running sum before it.
fn scan_exclusive(run: ScanEl, el: ScanEl) -> vec2<u32> {
    if params.scan_op == 0u {
        return select(run.v, vec2<u32>(0u), el.flag != 0u);
    }
    return vec2<u32>(run.v.x, select(run.v.y, 0u, el.flag != 0u));
}

var<workgroup> scan_flag: array<u32, 64>;
var<workgroup> scan_val: array<vec2<u32>, 64>;

// The inclusive scan of `mine` across the workgroup's 64 lanes (Hillis-Steele).
// Every lane must call it; on return the shared arrays hold the result.
fn workgroup_scan(lane: u32, mine: ScanEl) -> ScanEl {
    scan_flag[lane] = mine.flag;
    scan_val[lane] = mine.v;
    workgroupBarrier();
    var acc = mine;
    for (var offset = 1u; offset < 64u; offset = offset * 2u) {
        var other = scan_identity();
        if lane >= offset { other = ScanEl(scan_flag[lane - offset], scan_val[lane - offset]); }
        workgroupBarrier();
        acc = scan_combine(other, acc);
        scan_flag[lane] = acc.flag;
        scan_val[lane] = acc.v;
        workgroupBarrier();
    }
    return acc;
}

// A lane's four consecutive records of the block.
fn lane_fold(block: u32, lane: u32) -> ScanEl {
    var mine = scan_identity();
    for (var q = 0u; q < 4u; q += 1u) {
        let j = block * SCAN_BLOCK + lane * 4u + q;
        if j < params.dash_total { mine = scan_combine(mine, scan_element(j)); }
    }
    return mine;
}

fn scan_block_word(block: u32) -> u32 { return params.scan_words_base + block * 4u; }

fn read_block(block: u32) -> ScanEl {
    let w = scan_block_word(block);
    return ScanEl(coverage[w], vec2<u32>(coverage[w + 1u], coverage[w + 2u]));
}

fn write_block(block: u32, el: ScanEl) {
    let w = scan_block_word(block);
    coverage[w] = el.flag;
    coverage[w + 1u] = el.v.x;
    coverage[w + 2u] = el.v.y;
}

// One workgroup per block: the block's aggregate.
@compute @workgroup_size(64)
fn dash_scan_reduce(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let block = wg.x + wg.y * 256u;
    let total = workgroup_scan(lane, lane_fold(block, lane));
    if lane == 63u && block < params.dash_blocks { write_block(block, total); }
}

// One workgroup: each block's aggregate becomes the exclusive prefix over the
// blocks before it, in chunks of 64.
@compute @workgroup_size(64)
fn dash_scan_blocks(@builtin(local_invocation_index) lane: u32) {
    var carry = scan_identity();
    let chunks = (params.dash_blocks + 63u) / 64u;
    for (var chunk = 0u; chunk < chunks; chunk += 1u) {
        let block = chunk * 64u + lane;
        var mine = scan_identity();
        if block < params.dash_blocks { mine = read_block(block); }
        _ = workgroup_scan(lane, mine);
        var before = scan_identity();
        if lane > 0u { before = ScanEl(scan_flag[lane - 1u], scan_val[lane - 1u]); }
        let last = ScanEl(scan_flag[63], scan_val[63]);
        if block < params.dash_blocks { write_block(block, scan_combine(carry, before)); }
        carry = scan_combine(carry, last);
        workgroupBarrier();
    }
}

// One workgroup per block: each record's exclusive value. Op 1 also compacts
// the record's staged fragments into the list.
@compute @workgroup_size(64)
fn dash_scan_apply(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let block = wg.x + wg.y * 256u;
    _ = workgroup_scan(lane, lane_fold(block, lane));
    var before = scan_identity();
    if lane > 0u { before = ScanEl(scan_flag[lane - 1u], scan_val[lane - 1u]); }
    if block >= params.dash_blocks { return; }
    var run = scan_combine(read_block(block), before);
    for (var q = 0u; q < 4u; q += 1u) {
        let j = block * SCAN_BLOCK + lane * 4u + q;
        if j >= params.dash_total { break; }
        let el = scan_element(j);
        let value = scan_exclusive(run, el);
        let w = dash_word_index(j);
        if params.scan_op == 0u {
            coverage[w + 2u] = value.x;
            coverage[w + 3u] = value.y;
        } else {
            coverage[w + 6u] = value.x;
            coverage[w + 7u] = value.y;
            compact_fragments(j, value.x, value.y);
        }
        run = scan_combine(run, el);
    }
}

// Copies record `j`'s staged fragments into the compact list: (stage index,
// path-relative chord slot).
fn compact_fragments(j: u32, offset: u32, chord_offset: u32) {
    let w = dash_word_index(j);
    let head = strokes[(params.dash_first + j) * 6u];
    let first = paths[head.z].stage_base + head.w;
    var slot = chord_offset;
    for (var k = 0u; k < coverage[w + 4u]; k += 1u) {
        let stage = first + k;
        let at = params.list_base + (offset + k) * LIST_WORDS;
        coverage[at] = stage;
        coverage[at + 1u] = slot;
        slot += coverage[params.staging_base + stage * STAGE_WORDS + 3u];
    }
}

// ---- dash boundaries ---------------------------------------------------------

// The path's dash table behind the stroke records.
struct DashTable {
    base: u32,
    entries: u32,
    period: vec2<u32>,
    phase: vec2<u32>,
}

fn dash_word(w: u32) -> u32 { return strokes[w / 4u][w % 4u]; }

fn dash_table(pi: u32) -> DashTable {
    let base = params.stroke_total * RECORD_WORDS + paths[pi].dash_base;
    let entries = paths[pi].dash_entries;
    let at = base + 2u * entries;
    return DashTable(base, entries, vec2<u32>(dash_word(at), dash_word(at + 1u)), paths[pi].dash_phase);
}

fn dash_cum(t: DashTable, e: u32) -> vec2<u32> {
    return vec2<u32>(dash_word(t.base + 2u * e), dash_word(t.base + 2u * e + 1u));
}

// The cycle of position `u` and its offset inside it, from a float estimate
// corrected by exact fixed-point steps.
struct Cycle {
    index: u32,
    offset: vec2<u32>,
}

fn dash_cycle(t: DashTable, u: vec2<u32>) -> Cycle {
    var c = u32(floor(fx_to(u) / fx_to(t.period)));
    var base = fx_mul(t.period, c);
    loop {
        if !fx_lt(u, base) { break; }
        c -= 1u;
        base = fx_sub(base, t.period);
    }
    var rest = fx_sub(u, base);
    loop {
        if fx_lt(rest, t.period) { break; }
        rest = fx_sub(rest, t.period);
        c += 1u;
    }
    return Cycle(c, rest);
}

// The entry whose [cum[e], cum[e + 1]) holds the in-cycle offset; the
// offset is inside the period by construction, so `e + 1` never leaves
// the table.
fn dash_entry(t: DashTable, offset: vec2<u32>) -> u32 {
    var e = 0u;
    loop {
        if fx_lt(offset, dash_cum(t, e + 1u)) { break; }
        e += 1u;
    }
    return e;
}

// Whether some positive-length on-interval [A, B] has A < u <= B, and whether
// it ends exactly at u: kurbo's "the dash is still running at u".
fn dash_probe(t: DashTable, u: vec2<u32>) -> vec2<bool> {
    let cycle = dash_cycle(t, u);
    var rest = cycle.offset;
    if fx_eq(rest, vec2<u32>(0u)) {
        // A position on a cycle boundary belongs to the cycle before it.
        if cycle.index == 0u { return vec2<bool>(false, false); }
        rest = t.period;
    }
    for (var e = 0u; e < t.entries; e += 2u) {
        let lo = dash_cum(t, e);
        let hi = dash_cum(t, e + 1u);
        if fx_lt(lo, rest) && !fx_lt(hi, rest) { return vec2<bool>(true, fx_eq(hi, rest)); }
    }
    return vec2<bool>(false, false);
}

// Whether a closed subpath's last dash runs into its first (section 7.4.4).
// Every record of the subpath evaluates it from the same exact inputs.
fn subpath_merges(pi: u32, r: StrokeRecord, t: DashTable) -> bool {
    if ((paths[pi].cap_word >> 4u) & 1u) == 0u { return false; }
    let last = paths[pi].stroke_base + r.aux.w;
    if (strokes[last * 6u].x & CLOSE) == 0u { return false; }
    let w = dash_word_index(last - params.dash_first);
    let finish = fx_add(t.phase, fx_add(dash_fixed(w + 2u), dash_fixed(w)));
    return dash_probe(t, finish).x;
}

// A fragment's parameter at local arc length `sigma`.
fn arc_param(r: StrokeRecord, sigma: f32, span: f32) -> f32 {
    if piece_kind(r) == PIECE_CURVE { return inv_arclen(r, sigma); }
    return clamp(sigma / span, 0.0, 1.0);
}

fn stage_fragment(pi: u32, r: StrokeRecord, j: u32, k: u32, ta: f32, tb: f32, flags: u32, chords: u32) {
    let w = params.staging_base + (paths[pi].stage_base + r.head.w + k) * STAGE_WORDS;
    coverage[w] = bitcast<u32>(ta);
    coverage[w + 1u] = bitcast<u32>(tb);
    coverage[w + 2u] = flags | (j << FRAG_RECORD_SHIFT);
    coverage[w + 3u] = chords;
}

// The chords a fragment takes: the join, caps and `o_f` offset chords per side.
fn fragment_chords(pi: u32, r: StrokeRecord, flags: u32, ta: f32, tb: f32) -> u32 {
    var o = 0u;
    let kind = piece_kind(r);
    if kind == PIECE_LINE {
        o = 1u;
    } else if kind != PIECE_JOIN_ONLY {
        o = max(1u, u32(ceil(f32(r.aux.x >> 16u) * (tb - ta))));
    }
    var chords = 2u * o;
    if (flags & FRAG_JOIN) != 0u { chords += 2u * join_chords(r); }
    if (flags & FRAG_START_CAP) != 0u { chords += cap_chords(pi, start_cap(pi)); }
    if (flags & FRAG_END_CAP) != 0u { chords += cap_chords(pi, end_cap(pi)); }
    return chords;
}

// One invocation per dashed record: the dashes overlapping its window, with
// the joins and caps each draws, staged for compaction (section 7.4.3).
@compute @workgroup_size(64)
fn dash_fragments(@builtin(global_invocation_id) gid: vec3<u32>) {
    let j = invocation(gid);
    if j >= params.dash_total { return; }
    let i = params.dash_first + j;
    let r = load_record(i);
    let pi = r.head.z;
    let t = dash_table(pi);
    let w = dash_word_index(j);
    let span = dash_fixed(w);
    let u0 = fx_add(t.phase, dash_fixed(w + 2u));
    let u1 = fx_add(u0, span);
    let span_f = fx_to(span);
    let rel = i - paths[pi].stroke_base;
    let last_close = (strokes[(paths[pi].stroke_base + r.aux.w) * 6u].x & CLOSE) != 0u;
    let last_piece = select(r.aux.w, r.aux.w - 1u, last_close);
    let is_first = rel == r.aux.z;
    let is_last = rel == last_piece;
    let next_linear_end = !is_last && (strokes[(i + 1u) * 6u].x & LINEAR_END) != 0u;
    let merges = subpath_merges(pi, r, t);
    var count = 0u;
    var chords_total = 0u;

    if (r.head.x & CLOSE) != 0u {
        // The closing join of a merged dash.
        if merges {
            let chords = fragment_chords(pi, r, FRAG_JOIN, 0.0, 1.0);
            stage_fragment(pi, r, j, count, 0.0, 1.0, FRAG_JOIN, chords);
            count += 1u;
            chords_total += chords;
        }
    } else if (r.head.x & LINEAR_END) != 0u {
        // The final round join of a near-colinear cubic: drawn whenever a dash
        // reaches the vertex; capped when the dash ends there or the subpath does.
        let probe = dash_probe(t, u0);
        if probe.x {
            var flags = FRAG_JOIN;
            if (probe.y || is_last) && !(is_last && merges) { flags |= FRAG_END_CAP; }
            let chords = fragment_chords(pi, r, flags, 0.0, 1.0);
            stage_fragment(pi, r, j, count, 0.0, 1.0, flags, chords);
            count += 1u;
            chords_total += chords;
        }
    } else {
        let cycle = dash_cycle(t, u0);
        var e = dash_entry(t, cycle.offset);
        var cycle_base = fx_mul(t.period, cycle.index);
        loop {
            let a = fx_add(cycle_base, dash_cum(t, e));
            if !fx_lt(a, u1) { break; }
            let b = fx_add(cycle_base, dash_cum(t, e + 1u));
            if (e & 1u) == 0u && fx_lt(a, b) && fx_lt(u0, b) {
                var flags = 0u;
                var ta = 0.0;
                var tb = 1.0;
                // Start: a dash beginning inside the record caps there; one
                // beginning at or before the record's start caps only when the
                // record opens its subpath (or the dash starts exactly there),
                // and otherwise joins through the vertex.
                if fx_lt(u0, a) {
                    flags |= FRAG_START_CAP;
                    ta = arc_param(r, fx_to(fx_sub(a, u0)), span_f);
                } else if is_first {
                    if !merges { flags |= FRAG_START_CAP; }
                } else if fx_eq(a, u0) {
                    flags |= FRAG_START_CAP;
                } else {
                    flags |= FRAG_JOIN;
                }
                // End: a dash ending inside the record caps there; one reaching
                // the record's end caps when it ends exactly there (unless a
                // near-colinear cubic's closing join follows) or when the
                // record closes its subpath.
                if fx_lt(b, u1) {
                    flags |= FRAG_END_CAP;
                    tb = arc_param(r, fx_to(fx_sub(b, u0)), span_f);
                } else if is_last {
                    if !merges { flags |= FRAG_END_CAP; }
                } else if fx_eq(b, u1) && !next_linear_end {
                    flags |= FRAG_END_CAP;
                }
                let chords = fragment_chords(pi, r, flags, ta, tb);
                stage_fragment(pi, r, j, count, ta, tb, flags, chords);
                count += 1u;
                chords_total += chords;
            }
            e += 1u;
            if e == t.entries {
                e = 0u;
                cycle_base = fx_add(cycle_base, t.period);
            }
        }
    }
    coverage[w + 4u] = count;
    coverage[w + 5u] = chords_total;
}

// A single invocation: the staged fragment total and the indirect dispatch
// arguments of `dash_emit` and `bin_dashes`, written at args[12..15].
@compute @workgroup_size(64)
fn dash_args(@builtin(local_invocation_index) lane: u32) {
    if lane != 0u { return; }
    let w = dash_word_index(params.dash_total - 1u);
    let total = coverage[w + 6u] + coverage[w + 4u];
    work.dash_fragments = total;
    write_dispatch_args(12u, total);
}

// One invocation per fragment, indirect: the dash's joins, caps and both
// offsets between vertices computed once and shared by every chain
// (section 7.4.7).
@compute @workgroup_size(64)
fn dash_emit(@builtin(global_invocation_id) gid: vec3<u32>) {
    let f = invocation(gid);
    if f >= work.dash_fragments { return; }
    let entry = params.list_base + f * LIST_WORDS;
    let stage = params.staging_base + coverage[entry] * STAGE_WORDS;
    var out = coverage[entry + 1u];
    let ta = bitcast<f32>(coverage[stage]);
    let tb = bitcast<f32>(coverage[stage + 1u]);
    let flags = coverage[stage + 2u] & FRAG_FLAGS;
    let chords = coverage[stage + 3u];
    let i = params.dash_first + (coverage[stage + 2u] >> FRAG_RECORD_SHIFT);
    let r = load_record(i);
    let pi = r.head.z;
    let hw = paths[pi].half_width;
    let rel = i - paths[pi].stroke_base;
    let kind = piece_kind(r);
    let ends = end_slots(r, rel);

    var cap_s = 0u;
    var cap_e = 0u;
    var join_n = 0u;
    if (flags & FRAG_START_CAP) != 0u { cap_s = cap_chords(pi, start_cap(pi)); }
    if (flags & FRAG_END_CAP) != 0u { cap_e = cap_chords(pi, end_cap(pi)); }
    if (flags & FRAG_JOIN) != 0u { join_n = 2u * join_chords(r); }
    let n = (chords - join_n - cap_s - cap_e) / 2u;

    // A shared joint (a record that continues the previous piece smoothly)
    // takes no join chords, though the dash runs through it.
    if (flags & FRAG_JOIN) != 0u && has_join(r) {
        let tp = end_tangent(i - 1u);
        out = write_join(pi, out, r, tp, false);
        out = write_join(pi, out, r, tp, true);
    }
    // The dash's end vertices: the record's own slots at its ends, otherwise
    // computed once here.
    var sf = vertex(pi, r.link.z);
    var sb = vertex(pi, r.link.w);
    var start_centre = r.p.xy;
    if ta != 0.0 {
        let fr = frame_at(r, ta);
        sf = make_vertex(pi, offset_point(fr.centre, fr.tangent, -hw));
        sb = make_vertex(pi, offset_point(fr.centre, fr.tangent, hw));
        start_centre = fr.centre;
    }
    var ef = vertex(pi, ends.x);
    var eb = vertex(pi, ends.y);
    var end_centre = r.q.xy;
    if tb != 1.0 {
        let fr = frame_at(r, tb);
        ef = make_vertex(pi, offset_point(fr.centre, fr.tangent, -hw));
        eb = make_vertex(pi, offset_point(fr.centre, fr.tangent, hw));
        end_centre = fr.centre;
    }
    if cap_s != 0u { out = write_cap(pi, out, start_cap(pi), sb, sf, start_centre); }
    if kind != PIECE_JOIN_ONLY { out = write_offset(pi, out, r, false, ta, tb, n, sf, ef); }
    if cap_e != 0u { out = write_cap(pi, out, end_cap(pi), ef, eb, end_centre); }
    if kind != PIECE_JOIN_ONLY { out = write_offset(pi, out, r, true, ta, tb, n, sb, eb); }
}

// One invocation per fragment, indirect: bins its chords.
@compute @workgroup_size(64)
fn bin_dashes(@builtin(global_invocation_id) gid: vec3<u32>) {
    let f = invocation(gid);
    if f >= work.dash_fragments { return; }
    let entry = params.list_base + f * LIST_WORDS;
    let stage = params.staging_base + coverage[entry] * STAGE_WORDS;
    let i = params.dash_first + (coverage[stage + 2u] >> FRAG_RECORD_SHIFT);
    let pi = strokes[i * 6u].z;
    bin_segments(pi, paths[pi].seg_base + coverage[entry + 1u], coverage[stage + 3u]);
}
