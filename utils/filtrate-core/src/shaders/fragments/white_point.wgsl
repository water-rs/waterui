    // White point filter: normalize by a source white point triplet
    {
        let wp_r = max(param(param_idx), 0.0001);
        param_idx += 1u;
        let wp_g = max(param(param_idx), 0.0001);
        param_idx += 1u;
        let wp_b = max(param(param_idx), 0.0001);
        param_idx += 1u;
        let white_point = vec3<f32>(wp_r, wp_g, wp_b);
        color = vec4<f32>(color.rgb / white_point, color.a);
    }
