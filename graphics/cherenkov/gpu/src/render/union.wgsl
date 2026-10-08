// The union-field module of `shader.wgsl` — spliced into the `Union`
// shader variant only, so the member-cap arrays and the fold never reach
// pipelines for plain draws. `UNION_MAX_MEMBERS` arrives prepended like
// `VARIANT`: its single definition is `BackdropUnion::MAX_MEMBERS` in the
// core crate, formatted in by `build.rs` and `shaders.rs`.
//
// Backdrop union group data rides in the `stops` storage buffer: per
// union group one header `Stop` — `(smoothing k, member count n, -, -)`
// in its `color` — followed by `n` member records of exactly two `Stop`s:
// `stops[s]` carries `clip_inv[0]` in `color` and `clip_inv[1]` across
// `offset`/`pad`, `stops[s + 1]` the centred shape's `half`, `aspect`,
// `exponent` and `radii` the same way. A member instance's `uv.xy`
// bitcast to u32 carries `(record base, member index)`.

// The union field's evaluation at `pixel`: the folded signed distance,
// the unit outward normal, the ownership weight, the field's pixel width
// (the folded gradient's length) and the member's own signed distance.
struct UnionField {
    d: f32,
    n: vec2<f32>,
    w_own: f32,
    w: f32,
    own: f32,
}

// The signed distance and outward normal at `pixel` of the member whose
// clip record sits at `Stop` index `s` in `stops`: the member's own
// device-to-clip inverse and centred shape read back through the shared
// `device_sdf` math.
fn union_member_dist(s: u32, pixel: vec2<f32>) -> vec4<f32> {
    let ci = array<vec4<f32>, 2>(
        stops[s].color,
        vec4<f32>(stops[s].offset, stops[s].pad0, stops[s].pad1, stops[s].pad2),
    );
    let sa = stops[s + 1u];
    let shape = Shape(
        sa.color.xy,
        sa.color.z,
        sa.color.w,
        vec4<f32>(sa.offset, sa.pad0, sa.pad1, sa.pad2),
    );
    return device_sdf(ci, shape, pixel);
}

// The union field for the member at `ord` of the group whose run starts
// at `base`: every member's distance is folded into one field in
// ascending order by `m <- m - h^2 k / 4` with `h = max(k - (d_i - m), 0)
// / k`, the gradient folded `(1 - h/2) g + (h/2) g_i`. The member's
// ownership weight is `a_ord / Σ_j a_j` with
// `a_i = clamp(0.5 + f_i / |∇d₂ − ∇d_i|, 0, 1)` and `f_i = d₂ − d_i`
// against the member's nearest competitor (`order[0]`, or `order[1]`
// when `i` is the argmin itself): a hard step only where
// `|∇d₂ − ∇d_i| < 1e-6` (coincident shapes), an exact `f_i == 0` tie
// going to the earlier member in paint order.
fn union_field(base: u32, ord: u32, pixel: vec2<f32>) -> UnionField {
    let header = stops[base];
    let k = header.color.x;
    let n = u32(header.color.y);
    var ds: array<f32, UNION_MAX_MEMBERS>;
    var gs: array<vec2<f32>, UNION_MAX_MEMBERS>;
    var own = 0.0;
    for (var j = 0u; j < n; j = j + 1u) {
        let e = union_member_dist(base + 1u + j * 2u, pixel);
        ds[j] = e.x;
        gs[j] = e.yz;
        if (j == ord) {
            own = e.x;
        }
    }
    var order: array<u32, UNION_MAX_MEMBERS>;
    for (var j = 0u; j < n; j = j + 1u) {
        order[j] = j;
    }
    // Insertion sort by total order — `f32::total_cmp` semantics, so
    // -0.0 sorts before +0.0 exactly like the CPU and oracle folds:
    // equal distances keep paint order, and a signed-zero pair resolves
    // to the same owner everywhere.
    for (var a = 1u; a < n; a = a + 1u) {
        var b = a;
        loop {
            if (b == 0u) {
                break;
            }
            let prev = ds[order[b - 1u]];
            let cur = ds[order[b]];
            if (prev < cur || (prev == cur && bitcast<i32>(prev) <= bitcast<i32>(cur))) {
                break;
            }
            let t = order[b - 1u];
            order[b - 1u] = order[b];
            order[b] = t;
            b = b - 1u;
        }
    }
    var m = ds[order[0]];
    var g = gs[order[0]];
    for (var j = 1u; j < n; j = j + 1u) {
        let i = order[j];
        let h = max(k - (ds[i] - m), 0.0) / k;
        m = m - h * h * k * 0.25;
        g = (1.0 - 0.5 * h) * g + 0.5 * h * gs[i];
    }
    let w = max(length(g), 1e-6);
    var w_own = 1.0;
    if (n > 1u) {
        // `a_j` against the member's nearest competitor. `sum` can never
        // reach zero: the argmin member's competitor sits farther, so
        // its `a` is at least 0.5.
        var sum = 0.0;
        var a_ord = 0.0;
        for (var j = 0u; j < n; j = j + 1u) {
            let other = select(order[0], order[1], order[0] == j);
            let f = ds[other] - ds[j];
            let slope = length(gs[other] - gs[j]);
            var a: f32;
            if (slope < 1e-6) {
                a = select(0.0, 1.0, f > 0.0 || (f == 0.0 && j == order[0]));
            } else {
                a = clamp(0.5 + f / slope, 0.0, 1.0);
            }
            sum = sum + a;
            if (j == ord) {
                a_ord = a;
            }
        }
        w_own = a_ord / sum;
    }
    return UnionField(m, g / w, w_own, w, own);
}

// The union field at the fragment, evaluated once under FLAG_UNION: the
// coverage block's `w_own · AA(field < outer)` term and
// `paint_backdrop`'s `px.sdf`/`px.normal`/`px.own_sdf` share it.
var<private> backdrop_field: UnionField;

// The coverage a union member's composite gets from the shared field:
// `w_own` times the antialiased coverage of `field < outer`, `outer` in
// `params.x`. It replaces the member's clip coverage for the composite.
fn union_coverage(u: UnionField, outer: f32) -> f32 {
    return u.w_own * coverage(u.d - outer, u.w);
}
