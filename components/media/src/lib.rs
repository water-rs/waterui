extern crate alloc;

pub mod live;
pub mod photo;

#[cfg(feature = "media-picker")]
pub mod picker;
pub mod video;
pub use {live::LivePhoto, photo::Photo, video::Video};

use waterui_core::{AnyView, Environment, Str, View, reactive::impl_constant};

use crate::live::LivePhotoSource;

type Url = Str;

#[derive(Debug, Clone, uniffi::Enum)]
pub enum Media {
    Image(Url),
    LivePhoto(LivePhotoSource),
    Video(Url),
}

impl_constant!(LivePhotoSource, Media);

impl View for Media {
    fn body(self, _env: &Environment) -> impl View {
        match self {
            Media::Image(url) => AnyView::new(Photo::new(url)),
            Media::LivePhoto(live) => AnyView::new(LivePhoto::new(live)),
            Media::Video(url) => AnyView::new(Video::new(url)),
        }
    }
}
uniffi::setup_scaffolding!();
