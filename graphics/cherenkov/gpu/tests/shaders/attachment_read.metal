#include <metal_stdlib>

vertex float4 vertex_main(uint index [[vertex_id]]) {
    const float2 positions[] = { float2(-1, -1), float2(3, -1), float2(-1, 3) };
    return float4(positions[index], 0, 1);
}

struct Temporary {
    half4 value [[color(1), raster_order_group(0)]];
};

fragment Temporary produce() {
    return { half4(0.25h, 0.5h, 2.0h, 0.5h) };
}

struct Destination {
    half4 value [[color(0), raster_order_group(0)]];
};

fragment Destination composite(
    half4 destination [[color(0), raster_order_group(0)]],
    half4 source [[color(1), raster_order_group(0)]]
) {
    return { half4(float4(source) + float4(destination) * (1.0f - float(source.a))) };
}

struct FloatTemporary {
    float4 value [[color(1), raster_order_group(0)]];
};

fragment FloatTemporary produce_float() {
    return { float4(0.1f, 0.3f, 2.001f, 0.51f) };
}

struct FloatDestination {
    float4 value [[color(0), raster_order_group(0)]];
};

fragment FloatDestination composite_float(
    float4 source [[color(1), raster_order_group(0)]]
) {
    return { metal::abs(source - float4(half4(source))) * 1000.0f };
}
