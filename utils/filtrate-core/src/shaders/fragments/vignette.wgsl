    // Vignette filter: darken edges based on distance from center
    {
        let radius = param(param_idx);
        param_idx += 1u;
        let softness = param(param_idx);
        param_idx += 1u;
        let center = vec2<f32>(0.5, 0.5);
        let dist = distance(uv, center);
        let vignette = smoothstep(radius, radius - softness, dist);
        color = vec4<f32>(color.rgb * vignette, color.a);
    }
