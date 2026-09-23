    // Saturation filter: mix(gray, color, amount)
    {
        let amount = param(param_idx);
        param_idx += 1u;
        let luminance = dot(color.rgb, vec3<f32>(0.299, 0.587, 0.114));
        let gray = vec3<f32>(luminance, luminance, luminance);
        color = vec4<f32>(mix(gray, color.rgb, amount), color.a);
    }
