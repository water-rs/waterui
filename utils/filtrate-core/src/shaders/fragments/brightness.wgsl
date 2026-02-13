    // Brightness filter: color += amount
    {
        let amount = param(param_idx);
        param_idx += 1u;
        color = vec4<f32>(color.rgb + amount, color.a);
    }
