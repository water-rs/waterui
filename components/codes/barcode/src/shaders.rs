use shaderloom::CompiledShader;

pub const QR_MASK_EFFECT: CompiledShader = include!(concat!(env!("OUT_DIR"), "/qr_mask_effect.rs"));
pub const QR_RENDER: CompiledShader = include!(concat!(env!("OUT_DIR"), "/qr_render.rs"));
