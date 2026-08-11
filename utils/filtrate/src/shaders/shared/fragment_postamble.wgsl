
    // End of filter chain: re-premultiply and return.
    return vec4<f32>(color.rgb * color.a, color.a);
}
