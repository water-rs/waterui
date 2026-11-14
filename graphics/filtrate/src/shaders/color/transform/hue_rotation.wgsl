// Hue rotation: the CSS/SVG `hue-rotate` matrix (Filter Effects Module
// Level 1, `feColorMatrix type="hueRotate"`), applied to the premultiplied
// colour with alpha unchanged — a linear map on premultiplied RGBA.

struct Params {
    angle: f32,
}

const DEGREES_TO_RADIANS: f32 = 0.017453292519943295;

fn apply(color: vec4<f32>, params: Params) -> vec4<f32> {
    let angle = params.angle * DEGREES_TO_RADIANS;
    let c = cos(angle);
    let s = sin(angle);
    // Rows are the (r', g', b') outputs' coefficients on the (r, g, b)
    // input.
    let row0 = vec3<f32>(
        0.213 + c * 0.787 - s * 0.213,
        0.715 - c * 0.715 - s * 0.715,
        0.072 - c * 0.072 + s * 0.928,
    );
    let row1 = vec3<f32>(
        0.213 - c * 0.213 + s * 0.143,
        0.715 + c * 0.285 + s * 0.140,
        0.072 - c * 0.072 - s * 0.283,
    );
    let row2 = vec3<f32>(
        0.213 - c * 0.213 - s * 0.787,
        0.715 - c * 0.715 + s * 0.715,
        0.072 + c * 0.928 + s * 0.072,
    );
    return vec4<f32>(dot(row0, color.rgb), dot(row1, color.rgb), dot(row2, color.rgb), color.a);
}
