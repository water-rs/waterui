    // Grayscale filter: mix(color, gray, intensity)
    {
        let intensity = param(param_idx);
        param_idx += 1u;
        let luma = luminance(color.rgb);
        color = vec4<f32>(mix(color.rgb, vec3<f32>(luma), intensity), color.a);
    }
