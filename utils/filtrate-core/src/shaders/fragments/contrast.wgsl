    // Contrast filter: (color - 0.5) * amount + 0.5
    {
        let amount = param(param_idx);
        param_idx += 1u;
        let adjusted = (color.rgb - 0.5) * amount + 0.5;
        color = vec4<f32>(clamp(adjusted, vec3<f32>(0.0), vec3<f32>(1.0)), color.a);
    }
