//! Internal raster image decoding for [`ImageSource`](crate::canvas::ImageSource).

mod image_decode;

pub use image_decode::ImageDecodeError;
pub use image_decode::decode_pixel_buffer;
