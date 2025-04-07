pub mod live;
pub mod photo;

#[cfg(feature = "media-picker")]
pub mod picker;
pub mod video;
pub use {live::LivePhoto, photo::Photo, video::Video};

use waterui_core::{AnyView, Environment, Str, View};

use crate::live::LivePhotoSource;

type Url = Str;

#[derive(Debug)]
pub enum Media {
    Image(Url),
    LivePhoto(LivePhotoSource),
    Video(Url),
}

impl View for Media {
    fn body(self, _env: &Environment) -> impl View {
        match self {
            Media::Image(url) => AnyView::new(Photo::new(url)),
            Media::LivePhoto(live) => AnyView::new(LivePhoto::new(live)),
            Media::Video(url) => AnyView::new(Video::new(url)),
        }
    }
}
