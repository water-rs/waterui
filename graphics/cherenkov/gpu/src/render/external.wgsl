// Cherenkov external-frame tail: the producer plane decode and composite.
// `shared.wgsl` precedes this file in every build — the consts, instance
// layout, vertex stage, coverage machinery, clip-mask coverage and
// `srgb_decode` live there.
//
// One instanced quad per external layer samples the retained planes in
// place — never a copy or a raster fallback — decodes range, matrix and
// transfer, converts the source primaries into the extended-linear-P3
// working space with the absolute-level scale already folded into
// `params.prim`, then composites like any image: coverage, clip/mask,
// opacity, premultiplied src-over.

const KIND_EXT_RGB: u32 = 0u;
const KIND_EXT_NV12: u32 = 1u;
const KIND_EXT_P010: u32 = 2u;

// `params.info.w` bits, mirrored by `render::external`.
const EXT_FLAG_SHIFT6: u32 = 2u;

const EXT_ALPHA_OPAQUE: u32 = 0u;
const EXT_ALPHA_STRAIGHT: u32 = 1u;
const EXT_ALPHA_PREMULT: u32 = 2u;

// `params.info.y` transfer discriminants, mirrored by `interop::Transfer`.
const EXT_T_LINEAR: u32 = 0u;
const EXT_T_SRGB: u32 = 1u;
const EXT_T_709: u32 = 2u;
const EXT_T_PQ: u32 = 3u;
const EXT_T_HLG: u32 = 4u;

// The CPU-baked decode for one retained frame; `render::external::Params`
// declares the identical layout (192 bytes).
struct ExtParams {
    // x: plane kind, y: transfer, z: RGB alpha mode, w: EXT_FLAG_*.
    info: vec4<u32>,
    // Luma (or RGB plane) w, h, then chroma w, h.
    dims: vec4<f32>,
    // `code * scale + offset`, luma then chroma — the chroma offsets fold
    // in the -0.5 centring.
    norm: vec4<f32>,
    // x, y: the chroma siting s per axis (0 cosited, 0.5 centered);
    // z: the HLG OOTF gamma; w: unused.
    site: vec4<f32>,
    // Y'CbCr -> R'G'B' column vectors: colY, colCb, colCr.
    yuv0: vec4<f32>,
    yuv1: vec4<f32>,
    yuv2: vec4<f32>,
    pad0: vec4<f32>,
    // Source primaries -> extended linear P3, column-major, scaled to the
    // headroom space (PQ x 10000/reference_white, HLG x hlg_peak/
    // reference_white, relative transfers unscaled).
    prim0: vec4<f32>,
    prim1: vec4<f32>,
    prim2: vec4<f32>,
    // The Y row of the source primaries' RGB -> XYZ matrix: the luma the
    // HLG OOTF needs.
    luma: vec4<f32>,
}

@group(1) @binding(0) var ext_y: texture_2d<u32>;
@group(1) @binding(1) var ext_uv: texture_2d<u32>;
@group(1) @binding(2) var ext_rgb: texture_2d<f32>;
// `mask_tex` is group(1) binding(3), declared in `shared.wgsl`.
@group(1) @binding(4) var<uniform> params: ExtParams;

// The frame's encoded transfer back to nominal linear ([0,1]; for PQ, the
// [0,1] fraction of 10000 nits — the prim scale carries the level).
fn ext_decode(c: vec3<f32>, transfer: u32) -> vec3<f32> {
    switch transfer {
        case EXT_T_SRGB: {
            return srgb_decode(c);
        }
        case EXT_T_709: {
            // BT.1886 reference EOTF — a pure 2.4 power, black level 0:
            // display-referred light with reference white at 1.0. A display
            // presents BT.709-encoded video through this curve.
            return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.4));
        }
        case EXT_T_PQ: {
            // ST 2084 EOTF^-1; the signal domain is [0,1] of 10000 nits.
            let cp = pow(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)),
                         vec3<f32>(1.0 / 78.84375));
            let num = max(cp - vec3<f32>(0.8359375), vec3<f32>(0.0));
            let den = vec3<f32>(18.8515625) - vec3<f32>(18.6875) * cp;
            return pow(num / den, vec3<f32>(1.0 / 0.1593017578125));
        }
        case EXT_T_HLG: {
            // BT.2100 HLG inverse OETF: scene-linear [0,1].
            let lo = c * c / 3.0;
            let hi = (exp((c - vec3<f32>(0.55991073)) / vec3<f32>(0.17883277))
                      + vec3<f32>(0.28466892)) / vec3<f32>(12.0);
            return select(hi, lo, c <= vec3<f32>(0.5));
        }
        default: {
            return c;
        }
    }
}

// Nearest-texel sample of `tex` at `pos` in pixel coordinates, clamped to
// the edge like the image sampler — a frame shown at another size aliases
// honestly rather than sampling out of bounds.
fn ext_texel_u32(tex: texture_2d<u32>, pos: vec2<f32>, dims: vec2<f32>) -> vec4<u32> {
    let xy = clamp(vec2<i32>(round(pos - vec2<f32>(0.5))), vec2<i32>(0),
                   vec2<i32>(dims) - 1);
    return textureLoad(tex, xy, 0);
}

fn ext_texel_f32(tex: texture_2d<f32>, pos: vec2<f32>, dims: vec2<f32>) -> vec4<f32> {
    let xy = clamp(vec2<i32>(round(pos - vec2<f32>(0.5))), vec2<i32>(0),
                   vec2<i32>(dims) - 1);
    return textureLoad(tex, xy, 0);
}

// The HLG OOTF: scene luminance Ys scaled by `gamma - 1` before the
// primaries matrix (folded into `rgb` post-multiply since it is scalar).
fn ext_hlg(rgb: vec3<f32>) -> vec3<f32> {
    if params.info.y != EXT_T_HLG {
        return rgb;
    }
    let ys = dot(params.luma.xyz, rgb);
    return rgb * pow(max(ys, 0.0), params.site.z - 1.0);
}

// The YUV plane decode at frame pixel `px` (pixel centres are at
// `k + 0.5`): luma at the pixel's own coordinate, chroma at the
// siting-offset subsampled coordinate — chroma texel j centres at frame
// position 2j + 0.5 + s, so texel space is `p / 2 + 0.25 - s / 2`.
fn ext_frame_yuv(px: vec2<f32>) -> vec4<f32> {
    let y4 = ext_texel_u32(ext_y, px, params.dims.xy);
    let c4 = ext_texel_u32(ext_uv,
        px * 0.5 + vec2<f32>(0.25) - params.site.xy * 0.5,
        params.dims.zw);
    // P010 keeps the 10-bit code in the high bits of a 16-bit
    // word: the read value is the code times 64, and `norm`
    // expects the code.
    var shift = 1.0;
    if (params.info.w & EXT_FLAG_SHIFT6) != 0u {
        shift = 64.0;
    }
    let yn = f32(y4.x) / shift * params.norm.x + params.norm.y;
    let cbn = f32(c4.x) / shift * params.norm.z + params.norm.w;
    let crn = f32(c4.y) / shift * params.norm.z + params.norm.w;
    let encoded = mat3x3<f32>(params.yuv0.xyz, params.yuv1.xyz,
                              params.yuv2.xyz) * vec3<f32>(yn, cbn, crn);
    let lin = ext_decode(encoded, params.info.y);
    let rgb = mat3x3<f32>(params.prim0.xyz, params.prim1.xyz,
                          params.prim2.xyz) * ext_hlg(lin);
    return vec4<f32>(rgb, 1.0);
}

@fragment
fn fs_external(in: VsOut) -> @location(0) vec4<f32> {
    var cov: f32;
    switch in.meta_.x {
        case KIND_SPAN: {
            cov = 1.0;
        }
        default: {
            let s = Shape(in.shape_a.xy, in.shape_a.z, in.shape_a.w, in.shape_radii);
            let m = array<vec4<f32>, 2>(in.affine0, in.affine1);
            cov = shape_coverage(s, in.local, m, false);
        }
    }
    cov *= clip_mask_coverage(in);
    cov = clamp(cov, 0.0, 1.0) * in.params.y;

    // `in.local` is centred on the quad. The quad spans the binding's
    // own size — `in.cell.zw` carries it — which need not be the frame's:
    // the pixel coordinate scales by `params.dims / quad`.
    let px = (in.local + in.cell.zw * 0.5) * (params.dims.xy / in.cell.zw);
    var color: vec4<f32>;
    switch params.info.x {
        case KIND_EXT_NV12, KIND_EXT_P010: {
            color = ext_frame_yuv(px);
        }
        default: {
            // KIND_EXT_RGB.
            var c = ext_texel_f32(ext_rgb, px, params.dims.xy);
            let alpha = select(c.a, 1.0, params.info.z == EXT_ALPHA_OPAQUE);
            var lin = ext_decode(c.rgb, params.info.y);
            lin = ext_hlg(lin);
            if params.info.z == EXT_ALPHA_STRAIGHT {
                lin = lin * alpha;
            }
            let rgb = mat3x3<f32>(params.prim0.xyz, params.prim1.xyz,
                                  params.prim2.xyz) * lin;
            color = vec4<f32>(rgb, alpha);
        }
    }
    return move_space(color * cov, SPACE_LINEAR, globals.space);
}
