    // Photo effect: transfer — warm fade with a soft sepia bias.
    {
        let lum = luminance(color.rgb);
        let sepia = vec3<f32>(lum * 1.07, lum * 0.95, lum * 0.78);
        let blended = mix(color.rgb, sepia, 0.55);
        color = vec4<f32>(clamp(blended, vec3<f32>(0.0), vec3<f32>(COLOR_CLAMP_MAX)), color.a);
    }
