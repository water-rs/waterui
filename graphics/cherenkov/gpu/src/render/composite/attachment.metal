// Native stage interfaces around the build-time naga fragment bodies.
#include "engine_tile.metal"

struct TileInput {
    metal::float2 local [[user(loc0), center_perspective]];
    metal::float2 device_ [[user(loc1), center_perspective]];
    uint instance [[user(loc2), flat]];
    metal::uint4 meta [[user(loc3), flat]];
    metal::float4 color [[user(loc4), flat]];
    metal::float4 params [[user(loc5), flat]];
    metal::float4 shape_a [[user(loc6), flat]];
    metal::float4 shape_radii [[user(loc7), flat]];
    metal::float4 affine0 [[user(loc8), flat]];
    metal::float4 affine1 [[user(loc9), flat]];
    metal::float4 cell [[user(loc10), flat]];
};

VsOut tile_input(TileInput v, metal::float4 position, constant Globals& globals) {
    VsOut result = {};
    result.position = position;
    result.local = v.local;
    // Translating a viewport changes interpolant rounding at clip edges.
    // Device coordinates are pixel centres in the physical attachment grid.
    result.device_ = position.xy + globals.attachment_origin;
    result.instance = v.instance;
    result.meta = v.meta;
    result.color = v.color;
    result.params = v.params;
    result.shape_a = v.shape_a;
    result.shape_radii = v.shape_radii;
    result.affine0_ = v.affine0;
    result.affine1_ = v.affine1;
    result.cell = v.cell;
    return result;
}

#define TILE_RESOURCES \
    TileInput v [[stage_in]], metal::float4 position [[position]], \
    constant Globals& globals [[buffer(0)]], \
    device InstanceBuffer const& instances [[buffer(1)]], \
    device StopBuffer const& stops [[buffer(2)]], \
    metal::texture2d<float> atlas [[texture(0)]], \
    metal::texture2d<float> source [[texture(1)]], \
    metal::texture2d<float> image_tex [[texture(3)]], \
    metal::texture2d<float> mask_tex [[texture(4)]]

#define TILE_SHADE \
    metal::float2 backdrop_origin = {}, backdrop_size = {}; \
    metal::float4 source_pixel = metal::float4(src); \
    metal::float4 destination_pixel = metal::float4(dst); \
    return { fs_full(tile_input(v, position, globals), globals, instances, stops, \
        atlas, mask_tex, source, image_tex, backdrop_origin, backdrop_size, \
        source_pixel, destination_pixel) };

#define TILE_OUTPUT(D) \
    struct TileOutput##D { \
        metal::float4 color [[color(D), raster_order_group(0)]]; \
    }; \
    fragment TileOutput##D tile_##D##_simple( \
        TileInput v [[stage_in]], metal::float4 position [[position]], \
        constant Globals& globals [[buffer(0)]], metal::texture2d<float> atlas [[texture(0)]]) { \
        return { fs_simple(tile_input(v, position, globals), false, globals, atlas) }; \
    } \
    fragment TileOutput##D tile_##D##_shadow( \
        TileInput v [[stage_in]], metal::float4 position [[position]], \
        constant Globals& globals [[buffer(0)]]) { \
        return { fs_shadow(tile_input(v, position, globals), globals) }; \
    } \
    fragment TileOutput##D tile_##D##_clear() { return { metal::float4(0) }; } \
    fragment TileOutput##D tile_##D##_##D(TILE_RESOURCES, \
        metal::float4 dst [[color(D), raster_order_group(0)]]) { \
        metal::float4 src = dst; \
        TILE_SHADE \
    }

#define TILE_COMPOSITE(D,S) \
    fragment TileOutput##D tile_##D##_##S(TILE_RESOURCES, \
        metal::float4 dst [[color(D), raster_order_group(0)]], \
        metal::float4 src [[color(S), raster_order_group(0)]]) { \
        TILE_SHADE \
    }

TILE_OUTPUT(0)
TILE_OUTPUT(1)
TILE_OUTPUT(2)
TILE_OUTPUT(3)
TILE_COMPOSITE(0,1)
TILE_COMPOSITE(0,2)
TILE_COMPOSITE(0,3)
TILE_COMPOSITE(1,0)
TILE_COMPOSITE(1,2)
TILE_COMPOSITE(1,3)
TILE_COMPOSITE(2,0)
TILE_COMPOSITE(2,1)
TILE_COMPOSITE(2,3)
TILE_COMPOSITE(3,0)
TILE_COMPOSITE(3,1)
TILE_COMPOSITE(3,2)

vertex metal::float4 tile_clear_vertex(uint index [[vertex_id]]) {
    const metal::float2 points[] = {
        metal::float2(-1, -1), metal::float2(3, -1), metal::float2(-1, 3)
    };
    return metal::float4(points[index], 0, 1);
}
