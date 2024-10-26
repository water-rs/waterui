use waterui::layout::Edge;
pub type waterui_edge = Edge;
ffi_safe!(waterui_edge);

ffi_metadata!(
    Edge,
    waterui_edge,
    waterui_metadata_force_as_padding,
    waterui_metadata_padding_id
);
