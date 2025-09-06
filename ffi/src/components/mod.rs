use crate::{IntoFFI, str::WuiStr};

pub mod layout;

impl<T: IntoFFI> IntoFFI for waterui::component::Native<T> {
    type FFI = T::FFI;
    fn into_ffi(self) -> Self::FFI {
        IntoFFI::into_ffi(self.0)
    }
}

ffi_view!(waterui::component::divder::Divider, waterui_divider_id);

pub mod button {
    use crate::action::WuiAction;
    use crate::{WuiAnyView, ffi_struct, ffi_view};
    use waterui::component::Native;
    use waterui::component::button::ButtonConfig;

    /// C representation of a WaterUI button for FFI purposes.
    #[derive(Debug)]
    #[repr(C)]
    pub struct WuiButton {
        /// Pointer to the button's label view
        pub label: *mut WuiAnyView,
        /// Pointer to the button's action handler
        pub action: *mut WuiAction,
    }

    ffi_struct!(ButtonConfig, WuiButton, label, action);
    ffi_view!(
        Native<ButtonConfig>,
        WuiButton,
        waterui_button_id,
        waterui_force_as_button
    );
}

ffi_view!(
    waterui::Str,
    WuiStr,
    waterui_force_as_label,
    waterui_label_id
);

pub mod link {
    use crate::{WuiAnyView, ffi_struct, ffi_view};
    use waterui::component::link::LinkConfig;
    use waterui::{AnyView, Computed, Str, component::Native};

    #[repr(C)]
    pub struct WuiLink {
        pub label: *mut WuiAnyView,
        pub url: *mut Computed<Str>,
    }

    ffi_struct!(LinkConfig, WuiLink, label, url);
    ffi_view!(
        Native<LinkConfig>,
        WuiLink,
        waterui_link_id,
        waterui_force_as_link
    );
}

pub mod text {
    use crate::IntoFFI;
    use crate::{color::WuiColor, ffi_struct, ffi_view};
    use waterui::component::Native;
    use waterui::{Computed, Str, view::ConfigurableView};
    use waterui_text::Text;
    use waterui_text::{TextConfig, font::Font};

    /// C representation of Font
    #[repr(C)]
    pub struct WuiFont {
        pub size: f64,
        pub italic: bool,
        pub strikethrough: WuiColor,
        pub underlined: WuiColor,
        pub bold: bool,
    }

    /// C representation of Text configuration
    #[repr(C)]
    pub struct WuiText {
        /// Pointer to the text content computed value
        pub content: *mut Computed<Str>,
        /// Pointer to the font computed value
        pub font: *mut Computed<Font>,
    }

    impl IntoFFI for Text {
        type FFI = WuiText;
        fn into_ffi(self) -> Self::FFI {
            self.config().into_ffi()
        }
    }

    // Implement struct conversions
    ffi_struct!(Font, WuiFont, size, italic, strikethrough, underlined, bold);
    ffi_struct!(TextConfig, WuiText, content, font);

    // FFI view bindings for text components
    ffi_view!(
        Native<TextConfig>,
        WuiText,
        waterui_text_id,
        waterui_force_as_text
    );
}

/// Form component FFI bindings
pub mod form {
    use crate::components::text::WuiText;
    use crate::{WuiAnyView, ffi_struct, ffi_view};
    use waterui::component::Native;
    use waterui::{Binding, Computed, Str};
    use waterui_form::{
        slider::SliderConfig,
        stepper::StepperConfig,
        text_field::{KeyboardType, TextFieldConfig},
        toggle::ToggleConfig,
    };

    ffi_enum_with_default!(
        KeyboardType,
        WuiKeyboardType,
        Text,
        Secure,
        Email,
        URL,
        Number,
        PhoneNumber
    );

    /// C representation of a TextField configuration
    #[repr(C)]
    pub struct WuiTextField {
        /// Pointer to the text field's label view
        pub label: *mut WuiAnyView,
        /// Pointer to the text value binding
        pub value: *mut Binding<Str>,
        /// Pointer to the prompt text
        pub prompt: WuiText,
        /// The keyboard type to use
        pub keyboard: WuiKeyboardType,
    }

    /// C representation of a Toggle configuration
    #[repr(C)]
    pub struct WuiToggle {
        /// Pointer to the toggle's label view
        pub label: *mut WuiAnyView,
        /// Pointer to the toggle state binding
        pub toggle: *mut Binding<bool>,
    }

    /// C representation of a range
    #[repr(C)]
    pub struct WuiRange<T> {
        /// Start of the range
        pub start: T,
        /// End of the range
        pub end: T,
    }

    /// C representation of a Slider configuration
    #[repr(C)]
    pub struct WuiSlider {
        /// Pointer to the slider's label view
        pub label: *mut WuiAnyView,
        /// Pointer to the minimum value label view
        pub min_value_label: *mut WuiAnyView,
        /// Pointer to the maximum value label view
        pub max_value_label: *mut WuiAnyView,
        /// The range of values
        pub range: WuiRange<f64>,
        /// Pointer to the value binding
        pub value: *mut Binding<f64>,
    }

    /// C representation of a Stepper configuration
    #[repr(C)]
    pub struct WuiStepper {
        /// Pointer to the value binding
        pub value: *mut Binding<i32>,
        /// Pointer to the step size computed value
        pub step: *mut Computed<i32>,
        /// Pointer to the stepper's label view
        pub label: *mut WuiAnyView,
        /// The valid range of values
        pub range: WuiRange<i32>,
    }

    // Implement RangeInclusive conversions
    use crate::IntoFFI;
    use core::ops::RangeInclusive;

    impl IntoFFI for RangeInclusive<f64> {
        type FFI = WuiRange<f64>;
        fn into_ffi(self) -> Self::FFI {
            WuiRange {
                start: *self.start(),
                end: *self.end(),
            }
        }
    }

    impl IntoFFI for RangeInclusive<i32> {
        type FFI = WuiRange<i32>;
        fn into_ffi(self) -> Self::FFI {
            WuiRange {
                start: *self.start(),
                end: *self.end(),
            }
        }
    }

    // Implement struct conversions
    ffi_struct!(
        TextFieldConfig,
        WuiTextField,
        label,
        value,
        prompt,
        keyboard
    );

    ffi_struct!(ToggleConfig, WuiToggle, label, toggle);
    ffi_struct!(
        SliderConfig,
        WuiSlider,
        label,
        min_value_label,
        max_value_label,
        range,
        value
    );
    ffi_struct!(StepperConfig, WuiStepper, value, step, label, range);

    // FFI view bindings for form components
    ffi_view!(
        Native<TextFieldConfig>,
        WuiTextField,
        waterui_text_field_id,
        waterui_force_as_text_field
    );

    ffi_view!(
        Native<ToggleConfig>,
        WuiToggle,
        waterui_toggle_id,
        waterui_force_as_toggle
    );

    ffi_view!(
        Native<SliderConfig>,
        WuiSlider,
        waterui_slider_id,
        waterui_force_as_slider
    );

    ffi_view!(
        Native<StepperConfig>,
        WuiStepper,
        waterui_stepper_id,
        waterui_force_as_stepper
    );
}

/// Navigation component FFI bindings
pub mod navigation {
    use crate::WuiAnyView;
    use crate::components::text::WuiText;
    use crate::{closure::WuiFn, ffi_struct, ffi_view};
    use alloc::vec::Vec;
    use waterui::{Binding, Color, Computed};
    use waterui_core::id::Id;
    use waterui_navigation::{Bar, NavigationLink, NavigationView, tab::TabsConfig};

    pub struct WuiNavigationView {
        pub root: *mut WuiAnyView,
        pub bar: *mut Bar,
        pub links: *mut Vec<NavigationLink>,
        pub tabs: *mut Option<TabsConfig>,
        pub selection: *mut Option<Binding<Id>>,
    }

    pub struct WuiNavigationLink {
        pub label: *mut WuiAnyView,
        pub destination: *mut WuiFn<*mut WuiAnyView>,
    }

    /// C representation of a navigation Bar configuration
    #[repr(C)]
    pub struct WuiBar {
        /// Pointer to the title text
        pub title: WuiText,
        /// Pointer to the background color computed value
        pub color: *mut Computed<Color>,
        /// Pointer to the hidden state computed value
        pub hidden: *mut Computed<bool>,
    }

    // Implement struct conversions
    ffi_struct!(Bar, WuiBar, title, color, hidden);

    // FFI view bindings for navigation components
    ffi_view!(NavigationView, waterui_navigation_view_id);
    ffi_view!(NavigationLink, waterui_navigation_link_id);

    // Note: TabsConfig and Tab are complex types with Vec<T> and closures
    // that require more advanced FFI handling - leaving for future implementation
}

/// Media component FFI bindings
pub mod media {
    use crate::str::WuiStr;
    use crate::{IntoFFI, WuiAnyView, ffi_struct, ffi_view};
    use waterui::component::Native;
    use waterui::{Binding, Computed};
    use waterui_media::{
        Video,
        live::{LivePhotoConfig, LivePhotoSource},
        photo::PhotoConfig,
        video::VideoPlayerConfig,
    };

    // Type alias for URL
    type Url = WuiStr;
    type Volume = f64;

    /// C representation of Photo configuration
    #[repr(C)]
    pub struct WuiPhoto {
        /// The image source URL
        pub source: Url,
        /// Pointer to the placeholder view
        pub placeholder: *mut WuiAnyView,
    }
    #[repr(C)]
    pub struct WuiVideo {
        pub url: Url,
    }

    /// C representation of VideoPlayer configuration
    #[repr(C)]
    pub struct WuiVideoPlayer {
        /// Pointer to the video computed value
        pub video: *mut Computed<Video>,
        /// Pointer to the volume binding
        pub volume: *mut Binding<Volume>,
    }

    /// C representation of LivePhoto configuration
    #[repr(C)]
    pub struct WuiLivePhoto {
        /// Pointer to the live photo source computed value
        pub source: *mut Computed<LivePhotoSource>,
    }

    /// C representation of LivePhotoSource
    #[repr(C)]
    pub struct WuiLivePhotoSource {
        /// The image URL
        pub image: Url,
        /// The video URL
        pub video: Url,
    }

    ffi_struct!(LivePhotoSource, WuiLivePhotoSource, image, video);

    // Implement struct conversions
    ffi_struct!(PhotoConfig, WuiPhoto, source, placeholder);
    ffi_struct!(VideoPlayerConfig, WuiVideoPlayer, video, volume);
    ffi_struct!(LivePhotoConfig, WuiLivePhoto, source);

    impl IntoFFI for Video {
        type FFI = WuiVideo;
        fn into_ffi(self) -> Self::FFI {
            WuiVideo {
                url: self.url().inner().into_ffi(),
            }
        }
    }

    impl IntoFFI for waterui_media::Url {
        type FFI = WuiStr;
        fn into_ffi(self) -> Self::FFI {
            self.inner().into_ffi()
        }
    }

    // FFI view bindings for media components
    ffi_view!(
        Native<PhotoConfig>,
        WuiPhoto,
        waterui_photo_id,
        waterui_force_as_photo
    );

    ffi_view!(
        Native<VideoPlayerConfig>,
        WuiVideoPlayer,
        waterui_video_player_id,
        waterui_force_as_video_player
    );

    ffi_view!(
        Native<LivePhotoConfig>,
        WuiLivePhoto,
        waterui_live_photo_id,
        waterui_force_as_live_photo
    );

    ffi_view!(LivePhotoSource, waterui_live_photo_source_id);

    // Note: Media enum has complex tuple variants that need special FFI handling
    // - leaving for future implementation with manual IntoFFI implementation
}
