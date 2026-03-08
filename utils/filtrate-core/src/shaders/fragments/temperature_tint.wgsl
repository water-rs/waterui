    // Temperature/tint filter: shift along blue-yellow and green-magenta axes
    {
        let temperature = param(param_idx);
        param_idx += 1u;
        let tint = param(param_idx);
        param_idx += 1u;
        let warmed = color.rgb * vec3<f32>(
            1.0 + temperature * 0.1,
            1.0,
            1.0 - temperature * 0.1,
        );
        let tinted = warmed + tint * vec3<f32>(0.05, -0.05, 0.05);
        color = vec4<f32>(max(tinted, vec3<f32>(0.0)), color.a);
    }
