    // Photo effect: fade — lifted blacks, mild desaturation.
    {
        let lifted = mix(vec3<f32>(0.10), color.rgb, 0.85);
        let lum = luminance(lifted);
        let desat = mix(vec3<f32>(lum), lifted, 0.75);
        color = vec4<f32>(clamp(desat, vec3<f32>(0.0), vec3<f32>(COLOR_CLAMP_MAX)), color.a);
    }
