    // Exposure filter: color *= 2^ev
    {
        let ev = param(param_idx);
        param_idx += 1u;
        color = vec4<f32>(color.rgb * exp2(ev), color.a);
    }
