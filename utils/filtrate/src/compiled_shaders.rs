use shaderloom::CompiledShader;

pub const BLIT: CompiledShader = include!(concat!(env!("OUT_DIR"), "/filter_blit.rs"));
pub const MULTI_INPUT: CompiledShader =
    include!(concat!(env!("OUT_DIR"), "/multi_input_filter.rs"));
