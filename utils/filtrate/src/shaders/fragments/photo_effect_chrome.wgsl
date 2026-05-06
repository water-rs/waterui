    // Photo effect: chrome — saturation boost with a slight warm tint.
    {
        let lum = dot(color.rgb, vec3<f32>(0.299, 0.587, 0.114));
        let boosted = mix(vec3<f32>(lum), color.rgb, 1.45);
        let warmed = boosted * vec3<f32>(1.05, 1.0, 0.95);
        color = vec4<f32>(clamp(warmed, vec3<f32>(0.0), vec3<f32>(1.0)), color.a);
    }
