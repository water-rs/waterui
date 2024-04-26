use waterui_reactive::{compute::IntoComputed, Computed, CowStr};

#[derive(Debug)]
#[non_exhaustive]
pub struct Text {
    pub _content: Computed<CowStr>,
    pub _selection: Computed<bool>,
    pub _font: Font,
}

#[derive(Debug)]
#[repr(C)]
pub struct Font {
    size: f64,
    bold: bool,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            size: f64::NAN,
            bold: false,
        }
    }
}

raw_view!(Text);

impl Text {
    pub fn new(text: impl IntoComputed<CowStr>) -> Self {
        Self {
            _content: text.into_computed(),
            _selection: true.into_computed(),
            _font: Font::default(),
        }
    }

    pub fn font(mut self, font: Font) -> Self {
        self._font = font;
        self
    }

    pub fn size(mut self, size: f64) -> Self {
        self._font.size = size;
        self
    }

    pub fn selection(mut self, selection: impl IntoComputed<bool>) -> Self {
        self._selection = selection.into_computed();
        self
    }
}

pub fn text(text: impl IntoComputed<CowStr>) -> Text {
    Text::new(text)
}

mod ffi {
    use waterui_ffi::{
        computed::{waterui_computed_bool, waterui_computed_str},
        ffi_view, IntoFFI,
    };

    #[repr(C)]
    pub struct Text {
        content: *mut waterui_computed_str,
        selection: *mut waterui_computed_bool,
    }

    impl IntoFFI for super::Text {
        type FFI = Text;
        fn into_ffi(self) -> Self::FFI {
            Text {
                content: self._content.into_ffi(),
                selection: self._selection.into_ffi(),
            }
        }
    }

    ffi_view!(
        super::Text,
        Text,
        waterui_view_force_as_text,
        waterui_view_text_id
    );
}
