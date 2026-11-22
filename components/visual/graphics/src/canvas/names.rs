//! The ids one recording draws its resources by.

use std::collections::HashMap;

use crate::canvas::resource::{Font, ImageSource};
use crate::draw::{FontId, ImageId};

/// The id each resource a recording names was registered under on the
/// target the recording is for.
///
/// A live operand whose next value names a resource missing here cannot be
/// updated in place: the canvas records again, registering it.
#[derive(Debug, Default)]
pub struct Names {
    images: HashMap<usize, ImageId>,
    fonts: HashMap<usize, FontId>,
}

impl Names {
    pub(crate) fn image(&self, image: &ImageSource) -> Option<ImageId> {
        self.images.get(&image.key()).copied()
    }

    pub(crate) fn font(&self, font: &Font) -> Option<FontId> {
        self.fonts.get(&font.key()).copied()
    }

    pub(crate) fn insert_image(&mut self, image: &ImageSource, id: ImageId) {
        self.images.insert(image.key(), id);
    }

    pub(crate) fn insert_font(&mut self, font: &Font, id: FontId) {
        self.fonts.insert(font.key(), id);
    }
}
