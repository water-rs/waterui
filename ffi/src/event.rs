into_ffi! {WuiEvent,
    pub enum WuiEvent {
    Appear,
    Disappear,
}}

#[repr(C)]
pub struct WuiOnEvent {
    event: WuiEvent,
}
