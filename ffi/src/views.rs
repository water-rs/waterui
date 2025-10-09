use waterui::{AnyView, views::AnyViews};
use waterui_text::Text;

ffi_type!(WuiAnyViews, AnyViews<AnyView>, waterui_drop_any_views);
ffi_type!(WuiAnyTexts, AnyViews<Text>, waterui_drop_any_texts);
