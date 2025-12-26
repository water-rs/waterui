    // Brightness filter: color += amount
    {
        let amount = param(param_idx);
        param_idx += 1u;
        color = vec4<f32>(clamp(color.rgb + amount, vec3<f32>(0.0), vec3<f32>(1.0)), color.a);
    }
