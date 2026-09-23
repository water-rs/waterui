    // Photo effect: monochrome — luminance-based desaturation.
    {
        let lum = dot(color.rgb, vec3<f32>(0.299, 0.587, 0.114));
        color = vec4<f32>(vec3<f32>(lum), color.a);
    }
