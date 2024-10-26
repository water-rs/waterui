use alloc::vec::Vec;
use waterui::component::image::ImageConfig;

use crate::array::waterui_data;

impl_computed!(
    waterui_computed_data,
    Vec<u8>,
    waterui_data,
    waterui_read_computed_data,
    waterui_watch_computed_data,
    waterui_drop_computed_data
);

#[repr(C)]
pub struct waterui_image {
    data: *mut waterui_computed_data,
}

into_ffi!(ImageConfig, waterui_image, data);

native_view!(
    ImageConfig,
    waterui_image,
    waterui_view_force_as_image,
    waterui_view_image_id
);
