use waterui_reactive::{
    compute::{ComputeBool, ComputeStr, ComputedBool, ComputedStr},
    Compute,
};

#[derive(Debug)]
#[non_exhaustive]
pub struct Text {
    pub _content: ComputedStr,
    pub _selection: ComputedBool,
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
    pub fn new(text: impl ComputeStr) -> Self {
        Self {
            _content: text.computed(),
            _selection: true.computed(),
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

    pub fn selection(mut self, selection: impl ComputeBool) -> Self {
        self._selection = selection.computed();
        self
    }
}

pub fn text(text: impl ComputeStr) -> Text {
    Text::new(text)
}

mod ffi {
    use waterui_ffi::{
        computed::{ComputedBool, ComputedStr},
        ffi_view, IntoFFI,
    };

    #[repr(C)]
    pub struct Text {
        content: ComputedStr,
        selection: ComputedBool,
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
