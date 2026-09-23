struct SphericalProjectionParams {
    yaw_radians: f32,
    pitch_radians: f32,
    vertical_field_of_view_radians: f32,
    stereo_layout: u32,
    surface_aspect_ratio: f32,
    _padding0: u32,
    _padding1: u32,
    _padding2: u32,
}

@group(0) @binding(5) var<uniform> spherical_projection: SphericalProjectionParams;

const SPHERICAL_MONO: u32 = 0u;
const SPHERICAL_TOP_BOTTOM: u32 = 1u;
const SPHERICAL_LEFT_RIGHT: u32 = 2u;
const PI: f32 = 3.141592653589793;
const TAU: f32 = 6.283185307179586;

fn spherical_sample_coordinates(screen_uv: vec2<f32>) -> vec2<f32> {
    var eye_index = 0u;
    var eye_uv = screen_uv;
    var eye_aspect_ratio = spherical_projection.surface_aspect_ratio;
    if spherical_projection.stereo_layout != SPHERICAL_MONO {
        eye_index = select(0u, 1u, screen_uv.x >= 0.5);
        eye_uv.x = fract(screen_uv.x * 2.0);
        eye_aspect_ratio *= 0.5;
    }

    let tangent = tan(spherical_projection.vertical_field_of_view_radians * 0.5);
    let screen = vec2<f32>(eye_uv.x * 2.0 - 1.0, 1.0 - eye_uv.y * 2.0);
    let camera_ray = normalize(vec3<f32>(
        screen.x * tangent * eye_aspect_ratio,
        screen.y * tangent,
        -1.0,
    ));

    let pitch_sine = sin(spherical_projection.pitch_radians);
    let pitch_cosine = cos(spherical_projection.pitch_radians);
    let pitched = vec3<f32>(
        camera_ray.x,
        camera_ray.y * pitch_cosine - camera_ray.z * pitch_sine,
        camera_ray.y * pitch_sine + camera_ray.z * pitch_cosine,
    );
    let yaw_sine = sin(spherical_projection.yaw_radians);
    let yaw_cosine = cos(spherical_projection.yaw_radians);
    let world_ray = vec3<f32>(
        pitched.x * yaw_cosine + pitched.z * yaw_sine,
        pitched.y,
        -pitched.x * yaw_sine + pitched.z * yaw_cosine,
    );

    let longitude = atan2(world_ray.x, -world_ray.z);
    let latitude = asin(clamp(world_ray.y, -1.0, 1.0));
    var source_uv = vec2<f32>(0.5 + longitude / TAU, 0.5 - latitude / PI);
    if spherical_projection.stereo_layout == SPHERICAL_TOP_BOTTOM {
        source_uv.y = source_uv.y * 0.5 + f32(eye_index) * 0.5;
    } else if spherical_projection.stereo_layout == SPHERICAL_LEFT_RIGHT {
        source_uv.x = source_uv.x * 0.5 + f32(eye_index) * 0.5;
    }
    return source_uv;
}

@fragment
fn fs_spherical(input: VertexOutput) -> @location(0) vec4<f32> {
    return render_yuv_sample(spherical_sample_coordinates(input.uv));
}
