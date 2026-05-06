    // Photo effect: process — cool cast with crushed highlights.
    {
        let cooled = color.rgb * vec3<f32>(0.90, 0.95, 1.10);
        let crushed = vec3<f32>(
            min(cooled.r, 0.92),
            min(cooled.g, 0.94),
            min(cooled.b, 0.96),
        );
        color = vec4<f32>(clamp(crushed, vec3<f32>(0.0), vec3<f32>(1.0)), color.a);
    }
