    // Vibrance filter: boost muted colors more than already vivid colors.
    // Desaturation pivots on the shared luminance definition so Vibrance and
    // Saturation gray toward the same gray.
    {
        let amount = param(param_idx);
        param_idx += 1u;
        let luma = luminance(color.rgb);
        let max_c = max(max(color.r, color.g), color.b);
        let saturation = max_c - luma;
        let response = 1.0 - saturation / max(max_c, 0.0001);
        let factor = max(0.0, 1.0 + amount * response);
        color = vec4<f32>(mix(vec3<f32>(luma), color.rgb, factor), color.a);
    }
