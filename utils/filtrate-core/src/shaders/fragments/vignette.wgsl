    // Vignette filter: darken edges based on distance from center
    {
        let radius = param(param_idx);
        param_idx += 1u;
        let softness = param(param_idx);
        param_idx += 1u;
        let center = vec2<f32>(0.5, 0.5);
        let dist = distance(uv, center);
        let clamped_softness = max(softness, 0.0001);
        let edge0 = max(radius - clamped_softness, 0.0);
        let edge1 = max(radius, edge0 + 0.0001);
        let vignette = 1.0 - smoothstep(edge0, edge1, dist);
        color = vec4<f32>(color.rgb * vignette, color.a);
    }
