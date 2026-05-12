    // Photo effect: tonal — neutral low-saturation.
    {
        let lum = dot(color.rgb, vec3<f32>(0.299, 0.587, 0.114));
        let neutral = mix(vec3<f32>(lum), color.rgb, 0.4);
        color = vec4<f32>(neutral, color.a);
    }
