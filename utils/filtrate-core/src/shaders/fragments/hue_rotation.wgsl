    // Hue rotation filter: rotate hue by angle (degrees)
    {
        let angle = param(param_idx);
        param_idx += 1u;
        var hsl = rgb_to_hsl(color.rgb);
        hsl.x = fract(hsl.x + angle / 360.0);
        color = vec4<f32>(hsl_to_rgb(hsl), color.a);
    }
