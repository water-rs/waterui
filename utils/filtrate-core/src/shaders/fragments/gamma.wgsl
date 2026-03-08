    // Gamma filter: color = pow(color, 1/gamma)
    {
        let gamma = max(param(param_idx), 0.001);
        param_idx += 1u;
        let safe_rgb = max(color.rgb, vec3<f32>(0.0));
        color = vec4<f32>(pow(safe_rgb, vec3<f32>(1.0 / gamma)), color.a);
    }
