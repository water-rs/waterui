    // Photo effect: noir — high-contrast luminance desaturation.
    {
        let lum = luminance(color.rgb);
        // Stretch around the midtone to push deep shadows and bright highlights.
        let scaled = clamp((lum - 0.5) * 1.6 + 0.5, 0.0, COLOR_CLAMP_MAX);
        color = vec4<f32>(vec3<f32>(scaled), color.a);
    }
