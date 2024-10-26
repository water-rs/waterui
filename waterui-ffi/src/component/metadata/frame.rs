use waterui::{
    layout::{Alignment, Frame},
    Computed,
};
pub type waterui_frame = Frame;
ffi_safe!(waterui_frame);

pub type waterui_alignment = Alignment;

ffi_metadata!(
    Computed<Frame>,
    *mut waterui_computed_frame,
    waterui_metadata_force_as_frame,
    waterui_metadata_frame_id
);

impl_computed!(
    waterui_computed_frame,
    Frame,
    waterui_frame,
    waterui_read_computed_frame,
    waterui_watch_computed_frame,
    waterui_drop_computed_frame
);
