    // Vibrance filter: boost muted colors more than already vivid colors
    {
        let amount = param(param_idx);
        param_idx += 1u;
        let average = (color.r + color.g + color.b) / 3.0;
        let max_c = max(max(color.r, color.g), color.b);
        let saturation = max_c - average;
        let response = 1.0 - saturation / max(max_c, 0.0001);
        let factor = max(0.0, 1.0 + amount * response);
        let gray = vec3<f32>(average, average, average);
        color = vec4<f32>(mix(gray, color.rgb, factor), color.a);
    }
