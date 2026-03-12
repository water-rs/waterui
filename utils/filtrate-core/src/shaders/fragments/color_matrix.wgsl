    // Color matrix filter: 3x4 matrix over RGB + bias
    {
        let row0 = vec4<f32>(param(param_idx), param(param_idx + 1u), param(param_idx + 2u), param(param_idx + 3u));
        param_idx += 4u;
        let row1 = vec4<f32>(param(param_idx), param(param_idx + 1u), param(param_idx + 2u), param(param_idx + 3u));
        param_idx += 4u;
        let row2 = vec4<f32>(param(param_idx), param(param_idx + 1u), param(param_idx + 2u), param(param_idx + 3u));
        param_idx += 4u;
        let source = vec4<f32>(color.rgb, 1.0);
        color = vec4<f32>(dot(row0, source), dot(row1, source), dot(row2, source), color.a);
    }
