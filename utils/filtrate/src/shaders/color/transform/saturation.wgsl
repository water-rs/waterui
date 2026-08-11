    // Saturation filter: mix(gray, color, amount)
    {
        let amount = param(param_idx);
        param_idx += 1u;
        let luma = luminance(color.rgb);
        color = vec4<f32>(mix(vec3<f32>(luma), color.rgb, amount), color.a);
    }
