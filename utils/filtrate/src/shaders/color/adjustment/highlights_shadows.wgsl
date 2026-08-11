    // Highlights/shadows filter: lift shadows and compress highlights
    {
        let highlights = param(param_idx);
        param_idx += 1u;
        let shadows = param(param_idx);
        param_idx += 1u;
        let luma = luminance(color.rgb);
        let shadow_mask = pow(clamp(1.0 - luma, 0.0, 1.0), 2.0);
        let highlight_mask = pow(clamp(luma, 0.0, 1.0), 2.0);
        let shadowed = color.rgb * (1.0 + shadows * shadow_mask);
        let adjusted = shadowed * (1.0 - highlights * highlight_mask);
        color = vec4<f32>(max(adjusted, vec3<f32>(0.0)), color.a);
    }
