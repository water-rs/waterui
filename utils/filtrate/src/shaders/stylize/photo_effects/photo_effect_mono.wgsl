    // Photo effect: monochrome — luminance-based desaturation.
    {
        let lum = luminance(color.rgb);
        color = vec4<f32>(vec3<f32>(lum), color.a);
    }
