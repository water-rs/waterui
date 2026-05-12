    // Photo effect: instant — instant-camera warmth with reduced contrast.
    {
        let warmed = color.rgb * vec3<f32>(1.10, 1.02, 0.85) + vec3<f32>(0.05, 0.04, 0.0);
        let lum = dot(warmed, vec3<f32>(0.299, 0.587, 0.114));
        // Pull contrast down toward the mid-tone luminance.
        let softened = mix(vec3<f32>(lum), warmed, 0.7);
        color = vec4<f32>(clamp(softened, vec3<f32>(0.0), vec3<f32>(1.0)), color.a);
    }
