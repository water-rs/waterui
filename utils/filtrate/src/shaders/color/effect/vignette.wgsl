    // Vignette filter: darken edges based on distance from center.
    // Distance is measured in isotropic space (1.0 = shorter output edge),
    // so the vignette stays circular on non-square targets.
    {
        let radius = param(param_idx);
        param_idx += 1u;
        let softness = param(param_idx);
        param_idx += 1u;
        let dist = distance(to_isotropic(uv), to_isotropic(vec2<f32>(0.5, 0.5)));
        let clamped_softness = max(softness, 0.0001);
        let edge0 = max(radius - clamped_softness, 0.0);
        let edge1 = max(radius, edge0 + 0.0001);
        let vignette = 1.0 - smoothstep(edge0, edge1, dist);
        color = vec4<f32>(color.rgb * vignette, color.a);
    }
