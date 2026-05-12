    // Photo effect: noir — high-contrast luminance desaturation.
    {
        let lum = dot(color.rgb, vec3<f32>(0.299, 0.587, 0.114));
        // Stretch around the midtone to push deep shadows and bright highlights.
        let scaled = clamp((lum - 0.5) * 1.6 + 0.5, 0.0, 1.0);
        color = vec4<f32>(vec3<f32>(scaled), color.a);
    }
