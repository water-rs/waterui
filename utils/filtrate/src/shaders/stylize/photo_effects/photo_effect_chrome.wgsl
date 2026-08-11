    // Photo effect: chrome — saturation boost with a slight warm tint.
    {
        let lum = luminance(color.rgb);
        let boosted = mix(vec3<f32>(lum), color.rgb, 1.45);
        let warmed = boosted * vec3<f32>(1.05, 1.0, 0.95);
        color = vec4<f32>(clamp(warmed, vec3<f32>(0.0), vec3<f32>(COLOR_CLAMP_MAX)), color.a);
    }
