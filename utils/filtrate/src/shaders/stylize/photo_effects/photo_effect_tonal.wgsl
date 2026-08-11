    // Photo effect: tonal — neutral low-saturation.
    {
        let lum = luminance(color.rgb);
        let neutral = mix(vec3<f32>(lum), color.rgb, 0.4);
        color = vec4<f32>(neutral, color.a);
    }
