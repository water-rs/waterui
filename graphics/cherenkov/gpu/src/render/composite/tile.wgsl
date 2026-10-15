// Vulkan tile stage interfaces around the shared WGSL fragment bodies.
//
// `attachment.metal` is the same interface for Metal, where the entry
// points are hand-written MSL because the attachment inputs are stage-in
// arguments. Here the entry points are WGSL — naga emits them, and only
// the two corresponding-pixel reads are rewritten afterwards, from the
// private globals `build_tile.rs` leaves behind into SPIR-V input
// attachment reads (`build_tile_spirv.rs`). The shaded body is therefore
// the same naga emission the portable pipelines use.
//
// Every entry point writes `@location(0)` and reads input attachment
// index 0 (its own slot) and 1 (the source slot): the slot each one means
// is chosen per draw by `vkCmdSetRenderingAttachmentLocations` and
// `vkCmdSetRenderingInputAttachmentIndices`, so the slot pairs need no
// shader of their own.

// The viewport is translated per canonical pass within an epoch, so the
// fragment's device coordinate comes from its position in the attachment
// grid rather than from the interpolated vertex output.
fn tile_in(in: VsOut) -> VsOut {
    var out = in;
    out.device = in.position.xy + globals.attachment_origin;
    return out;
}

@fragment
fn tile_simple(in: VsOut) -> @location(0) vec4<f32> {
    return fs_simple(tile_in(in));
}

@fragment
fn tile_shadow(in: VsOut) -> @location(0) vec4<f32> {
    return fs_shadow(tile_in(in));
}

@fragment
fn tile_full(in: VsOut) -> @location(0) vec4<f32> {
    return fs_full(tile_in(in));
}

// Retiring a slot the epoch has finished with, over the region the
// planner records as the opening image's write set.
@vertex
fn tile_clear_vertex(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(corners[vi], 0.0, 1.0);
}

@fragment
fn tile_clear() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}
