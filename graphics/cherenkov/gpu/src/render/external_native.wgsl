// Cherenkov external-frame tail: the external-format YCbCr binding.
// `shared.wgsl` and `external.wgsl` precede this file in the build — the
// coverage machinery, clip-mask coverage, decode and primaries helpers
// live there. This file only exists for the Vulkan native module
// (`external_native.spv`); the ordinary external module never sees it.
//
// `ext_image` + `ext_sampler` are the designated pair the build-time
// lowering merges into one combined sampled image at binding 5, carrying
// the immutable sampler with the image's `VkSamplerYcbcrConversion`. The
// conversion reconstructs Y'CbCr and yields the encoded R'G'B' signal —
// the shader performs only #165's transfer decoding and primaries
// conversion, never the Y'CbCr matrix a second time.

@group(1) @binding(5) var ext_image: texture_2d<f32>;
@group(1) @binding(6) var ext_sampler: sampler;

@fragment
fn fs_external_format(in: VsOut) -> @location(0) vec4<f32> {
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
    // The conversion sampler is NEAREST: sampling at the texel centre the
    // texel contract (`round(pos - 0.5)`, edge-clamped) gives the same
    // sample ext_texel_f32 would. UV = centre / dims in [0, 1].
    let xy = clamp(round(px - vec2<f32>(0.5)), vec2<f32>(0.0),
                   params.dims.xy - vec2<f32>(1.0));
    let c = textureSample(ext_image, ext_sampler,
                          (xy + vec2<f32>(0.5)) / params.dims.xy);
    var lin = ext_decode(c.rgb, params.info.y);
    lin = ext_hlg(lin);
    let rgb = mat3x3<f32>(params.prim0.xyz, params.prim1.xyz,
                          params.prim2.xyz) * lin;
    // The conversion's alpha comes from the driver's component mapping;
    // the frame contract is opaque.
    return move_space(vec4<f32>(rgb, 1.0) * cov, SPACE_LINEAR, globals.space);
}
