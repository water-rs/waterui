    // Contrast filter: (color - 0.5) * amount + 0.5
    {
        let amount = param(param_idx);
        param_idx += 1u;
        let adjusted = (color.rgb - 0.5) * amount + 0.5;
        color = vec4<f32>(adjusted, color.a);
    }
