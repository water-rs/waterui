    // Sepia filter: mix(color, sepia_tone, intensity)
    {
        let intensity = param(param_idx);
        param_idx += 1u;
        let sepia = vec3<f32>(
            dot(color.rgb, vec3<f32>(0.393, 0.769, 0.189)),
            dot(color.rgb, vec3<f32>(0.349, 0.686, 0.168)),
            dot(color.rgb, vec3<f32>(0.272, 0.534, 0.131))
        );
        color = vec4<f32>(mix(color.rgb, sepia, intensity), color.a);
    }
