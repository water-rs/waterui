/// `ShapeTokens.CornerSmall`.
pub const SHAPE_CORNER_SMALL: f64 = 8.0;
/// `ShapeTokens.CornerMedium`.
pub const SHAPE_CORNER_MEDIUM: f64 = 12.0;
/// `ShapeTokens.CornerLarge`.
pub const SHAPE_CORNER_LARGE: f64 = 16.0;
/// `ShapeTokens.CornerExtraLarge`.
pub const SHAPE_CORNER_EXTRA_LARGE: f64 = 28.0;

/// The Material 3 Expressive button size scale.
///
/// Each entry is one of Compose's `Button*Tokens` objects. Height, padding,
/// icon size and corner shape all change together, which is why a size is a
/// token set rather than a height the caller picks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonSizeTokens {
    /// `ContainerHeight`.
    pub container_height: f64,
    /// `LeadingSpace` / `TrailingSpace`.
    pub horizontal_space: f64,
    /// `IconSize`.
    pub icon_size: f64,
    /// `IconLabelSpace`.
    pub icon_label_space: f64,
    /// `ContainerShapeSquare`, the radius a square-shaped button uses.
    pub square_corner_radius: f64,
    /// `PressedContainerShape`, which the container morphs to under a press.
    pub pressed_corner_radius: f64,
    /// `OutlinedOutlineWidth`.
    pub outline_width: f64,
}

/// `ButtonXSmallTokens`.
pub const BUTTON_EXTRA_SMALL: ButtonSizeTokens = ButtonSizeTokens {
    container_height: 32.0,
    horizontal_space: 16.0,
    icon_size: 20.0,
    icon_label_space: 8.0,
    square_corner_radius: SHAPE_CORNER_MEDIUM,
    pressed_corner_radius: SHAPE_CORNER_SMALL,
    outline_width: 1.0,
};

/// `ButtonSmallTokens`, the default size. Its horizontal space comes from
/// `BaselineButtonTokens`, which is what `ButtonDefaults.ContentPadding` uses.
pub const BUTTON_SMALL: ButtonSizeTokens = ButtonSizeTokens {
    container_height: 40.0,
    horizontal_space: 24.0,
    icon_size: 20.0,
    icon_label_space: 8.0,
    square_corner_radius: SHAPE_CORNER_MEDIUM,
    pressed_corner_radius: SHAPE_CORNER_SMALL,
    outline_width: 1.0,
};

/// `ButtonMediumTokens`.
pub const BUTTON_MEDIUM: ButtonSizeTokens = ButtonSizeTokens {
    container_height: 56.0,
    horizontal_space: 24.0,
    icon_size: 24.0,
    icon_label_space: 8.0,
    square_corner_radius: SHAPE_CORNER_LARGE,
    pressed_corner_radius: SHAPE_CORNER_MEDIUM,
    outline_width: 1.0,
};

/// `ButtonLargeTokens`.
pub const BUTTON_LARGE: ButtonSizeTokens = ButtonSizeTokens {
    container_height: 96.0,
    horizontal_space: 48.0,
    icon_size: 32.0,
    icon_label_space: 12.0,
    square_corner_radius: SHAPE_CORNER_EXTRA_LARGE,
    pressed_corner_radius: SHAPE_CORNER_LARGE,
    outline_width: 2.0,
};

/// `ButtonXLargeTokens`.
pub const BUTTON_EXTRA_LARGE: ButtonSizeTokens = ButtonSizeTokens {
    container_height: 136.0,
    horizontal_space: 64.0,
    icon_size: 40.0,
    icon_label_space: 16.0,
    square_corner_radius: SHAPE_CORNER_EXTRA_LARGE,
    pressed_corner_radius: SHAPE_CORNER_LARGE,
    outline_width: 2.0,
};

/// The token set for `size`.
#[must_use]
pub const fn button_size_tokens(size: waterui_controls::button::ButtonSize) -> ButtonSizeTokens {
    use waterui_controls::button::ButtonSize;
    match size {
        ButtonSize::ExtraSmall => BUTTON_EXTRA_SMALL,
        ButtonSize::Small => BUTTON_SMALL,
        ButtonSize::Medium => BUTTON_MEDIUM,
        ButtonSize::Large => BUTTON_LARGE,
        ButtonSize::ExtraLarge => BUTTON_EXTRA_LARGE,
        _ => panic!("hydrolysis ButtonSize variant is not implemented"),
    }
}

/// `ButtonDefaults.MinWidth`.
pub const BUTTON_MIN_WIDTH: f64 = 58.0;
/// `ButtonDefaults.ContentPadding` vertical.
pub const BUTTON_TEXT_VERTICAL_PADDING: f64 = 8.0;
pub const BUTTON_LINK_UNDERLINE_BOTTOM_INSET: f64 = 2.0;
pub const BUTTON_LINK_UNDERLINE_THICKNESS: f64 = 1.0;
/// `ButtonDefaults.TextButtonContentPadding` horizontal.
pub const BUTTON_LINK_HORIZONTAL_PADDING: f64 = 12.0;
/// `ButtonDefaults.TextButtonContentPadding` vertical.
pub const BUTTON_LINK_VERTICAL_PADDING: f64 = 8.0;

pub const TOGGLE_SWITCH_WIDTH: f64 = 52.0;
pub const TOGGLE_SWITCH_HEIGHT: f64 = 32.0;
pub const TOGGLE_SWITCH_OUTLINE_WIDTH: f64 = 2.0;
pub const TOGGLE_SWITCH_UNSELECTED_HANDLE_SIZE: f64 = 16.0;
pub const TOGGLE_SWITCH_SELECTED_HANDLE_SIZE: f64 = 24.0;
pub const TOGGLE_SWITCH_PRESSED_HANDLE_SIZE: f64 = 28.0;
/// Compose's `ThumbPadding`: `(TrackHeight - SelectedHandleWidth) / 2`.
///
/// Both rest positions come from this one value. Compose lays the thumb out in
/// a slot the size of the *selected* handle at either end, and centres the
/// smaller unselected handle inside that slot, so the thumb never shifts when
/// it changes size.
pub const TOGGLE_SWITCH_THUMB_PADDING: f64 =
    (TOGGLE_SWITCH_HEIGHT - TOGGLE_SWITCH_SELECTED_HANDLE_SIZE) / 2.0;
/// Size of the thumb icon (mdui switch `checked-icon` font size).
pub const TOGGLE_SWITCH_ICON_SIZE: f64 = 16.0;
/// Initial scale of the thumb icon while it transitions in/out (mdui
/// `transform: scale(0.92)`).
pub const TOGGLE_SWITCH_ICON_SCALE_START: f64 = 0.92;
pub const TOGGLE_LABEL_SPACING: f64 = 8.0;
pub const TOGGLE_CHECKBOX_SIZE: f64 = 18.0;
pub const TOGGLE_CHECKBOX_CONTAINER_SHAPE: f64 = 2.0;
pub const TOGGLE_CHECKBOX_OUTLINE_WIDTH: f64 = 2.0;
pub const TOGGLE_CHECKBOX_SELECTED_SCALE_START: f64 = 0.6;

pub const STEPPER_BUTTON_MIN_SIZE: f64 = 40.0;
pub const STEPPER_BUTTON_MAX_SIZE: f64 = 40.0;
pub const STEPPER_BUTTON_INTRINSIC_SIZE: f64 = 40.0;
pub const STEPPER_BUTTON_SPACING: f64 = 2.0;
pub const STEPPER_LABEL_SPACING: f64 = 8.0;
pub const STEPPER_ICON_SIZE: f64 = 24.0;
pub const STEPPER_INNER_CORNER_RADIUS: f64 = 8.0;
pub const STEPPER_PRESSED_INNER_CORNER_RADIUS: f64 = 4.0;

pub const INPUT_LABEL_HEIGHT: f64 = 18.0;
pub const INPUT_FIELD_MIN_WIDTH: f64 = 72.0;
pub const INPUT_FIELD_MIN_HEIGHT: f64 = 56.0;
pub const INPUT_FIELD_HORIZONTAL_INSET: f64 = 16.0;
pub const INPUT_FIELD_VERTICAL_INSET: f64 = 8.0;
pub const INPUT_FILLED_CONTAINER_TOP_RADIUS: f64 = 4.0;
pub const INPUT_FILLED_ACTIVE_INDICATOR_HEIGHT: f64 = 1.0;
pub const INPUT_FILLED_FOCUS_ACTIVE_INDICATOR_HEIGHT: f64 = 2.0;

pub const TEXT_CONTEXT_MENU_ROW_HEIGHT: f64 = 56.0;
pub const TEXT_CONTEXT_MENU_HORIZONTAL_PADDING: f64 = 16.0;
pub const TEXT_CONTEXT_MENU_VERTICAL_PADDING: f64 = 12.0;
pub const TEXT_CONTEXT_MENU_MIN_WIDTH: f64 = 112.0;
pub const TEXT_CONTEXT_MENU_MAX_WIDTH: f64 = 320.0;
pub const TEXT_CONTEXT_MENU_WIDTH_PER_CHAR: f64 = 8.5;
pub const TEXT_CONTEXT_MENU_CONTAINER_SHAPE: f64 = 4.0;
pub const TEXT_CONTEXT_MENU_SEPARATOR_HORIZONTAL_INSET: f64 = 16.0;
pub const TEXT_CONTEXT_MENU_SEPARATOR_THICKNESS: f64 = 1.0;

pub const PICKER_MIN_WIDTH: f64 = 72.0;
pub const PICKER_MIN_HEIGHT: f64 = 56.0;
pub const PICKER_HORIZONTAL_INSET: f64 = 16.0;
pub const PICKER_VERTICAL_INSET: f64 = 8.0;
pub const PICKER_LABEL_SPACING: f64 = 8.0;
pub const PICKER_INDICATOR_SPACE: f64 = 18.0;
pub const PICKER_RADIO_INDICATOR_SIZE: f64 = 20.0;
/// Compose's `RadioStrokeWidth`.
pub const PICKER_RADIO_OUTER_RING_WIDTH: f64 = 2.0;
/// Half of Compose's `RadioButtonDotSize` (12dp).
pub const PICKER_RADIO_INNER_DOT_RADIUS: f64 = 6.0;
pub const PICKER_RADIO_LABEL_SPACING: f64 = 8.0;
pub const PICKER_RADIO_ROW_SPACING: f64 = 6.0;
pub const PICKER_MENU_POPUP_TOP_SPACING: f64 = 4.0;
pub const PICKER_MENU_POPUP_ROW_HEIGHT: f64 = 48.0;
pub const PICKER_MENU_POPUP_CORNER_RADIUS: f64 = 4.0;
pub const PICKER_SEGMENTED_MIN_HEIGHT: f64 = 40.0;
pub const PICKER_SEGMENTED_HORIZONTAL_INSET: f64 = 12.0;
pub const PICKER_SEGMENTED_CONTAINER_RADIUS: f64 = 20.0;
pub const PICKER_SEGMENTED_OUTLINE_WIDTH: f64 = 1.0;

pub const SLIDER_HORIZONTAL_INSET: f64 = 20.0;
pub const SLIDER_HORIZONTAL_SPACING: f64 = 8.0;
pub const SLIDER_VERTICAL_SPACING: f64 = 6.0;
pub const SLIDER_MIN_TRACK_WIDTH: f64 = 72.0;
/// `SliderTokens.InactiveTrackHeight` / `ActiveTrackHeight`. The Expressive
/// slider's track is a thick bar, not the 4dp hairline of earlier Material.
pub const SLIDER_TRACK_HEIGHT: f64 = 16.0;
/// `SliderTokens.HandleWidth` — a narrow vertical bar, not a circular thumb.
pub const SLIDER_HANDLE_WIDTH: f64 = 4.0;
/// `SliderTokens.PressedHandleWidth`: the handle narrows under the finger.
pub const SLIDER_PRESSED_HANDLE_WIDTH: f64 = 2.0;
/// `SliderTokens.HandleHeight` — the handle stands taller than the track.
pub const SLIDER_HANDLE_HEIGHT: f64 = 44.0;
/// `SliderTokens.ActiveHandlePadding`: the gap the track leaves on each side of
/// the handle, so the bar never runs into it.
pub const SLIDER_HANDLE_PADDING: f64 = 6.0;
/// `SliderTokens.StopIndicatorSize`: the dot marking the track's far end.
pub const SLIDER_STOP_INDICATOR_SIZE: f64 = 4.0;

pub const PROGRESS_LINEAR_LABEL_HEIGHT: f64 = 18.0;
pub const PROGRESS_LINEAR_BAR_TOP_OFFSET: f64 = 10.0;
pub const PROGRESS_LINEAR_BAR_HEIGHT: f64 = 4.0;
pub const PROGRESS_LINEAR_BAR_HORIZONTAL_INSET: f64 = 8.0;
pub const PROGRESS_LINEAR_VALUE_LABEL_TOP_SPACING: f64 = 6.0;
pub const PROGRESS_LINEAR_MIN_TRACK_WIDTH: f64 = 80.0;
/// `CircularProgressIndicatorTokens.Size`.
pub const PROGRESS_CIRCULAR_DIAMETER: f64 = 40.0;
/// `CircularProgressIndicatorTokens.ActiveThickness` / `TrackThickness`.
pub const PROGRESS_CIRCULAR_STROKE_WIDTH: f64 = 4.0;
/// `CircularProgressIndicatorTokens.TrackActiveSpace`: the gap Expressive
/// leaves between the active arc and the track it runs over.
pub const PROGRESS_CIRCULAR_TRACK_ACTIVE_SPACE: f64 = 4.0;
/// `LinearProgressIndicatorTokens.TrackActiveSpace`: the same gap on the
/// linear bar, which is what makes the indicator read as riding in the track.
pub const PROGRESS_LINEAR_TRACK_ACTIVE_SPACE: f64 = 4.0;
/// `LinearProgressIndicatorTokens.StopSize`: the dot marking the bar's end.
pub const PROGRESS_LINEAR_STOP_SIZE: f64 = 4.0;

pub const DIVIDER_THICKNESS: f64 = 1.0;

pub const NAVIGATION_BAR_AUTOMATIC_HEIGHT: f64 = 64.0;
pub const NAVIGATION_BAR_INLINE_HEIGHT: f64 = 64.0;
pub const NAVIGATION_BAR_MEDIUM_HEIGHT: f64 = 112.0;
pub const NAVIGATION_BAR_LARGE_HEIGHT: f64 = 152.0;
pub const NAVIGATION_TITLE_INLINE_HEIGHT: f64 = 28.0;
pub const NAVIGATION_TITLE_MEDIUM_HEIGHT: f64 = 36.0;
pub const NAVIGATION_TITLE_LARGE_HEIGHT: f64 = 36.0;
pub const NAVIGATION_TITLE_LEADING_INSET: f64 = 16.0;
pub const NAVIGATION_TITLE_TRAILING_INSET: f64 = 16.0;
pub const NAVIGATION_LARGE_TITLE_BOTTOM_INSET: f64 = 28.0;
pub const NAVIGATION_BAR_HORIZONTAL_INSET: f64 = 4.0;
pub const NAVIGATION_BAR_ITEM_SPACING: f64 = 0.0;
pub const NAVIGATION_SEARCH_HEIGHT: f64 = 56.0;
pub const NAVIGATION_SEARCH_VERTICAL_INSET: f64 = 4.0;
pub const NAVIGATION_BACK_BUTTON_SIZE: f64 = 40.0;
pub const NAVIGATION_BACK_BUTTON_LEADING_INSET: f64 = 4.0;
pub const NAVIGATION_BACK_BUTTON_TOP_INSET: f64 = 12.0;
pub const NAVIGATION_SHARED_AXIS_SLIDE_DISTANCE: f64 = 30.0;
pub const SCROLL_INDICATOR_CORNER_RADIUS: f64 = 1.25;
pub const TABS_BAR_HEIGHT: f64 = 48.0;
pub const TABS_BUTTON_MIN_WIDTH: f64 = 48.0;
pub const TABS_BUTTON_HORIZONTAL_INSET: f64 = 16.0;
pub const TABS_ACTIVE_INDICATOR_HEIGHT: f64 = 3.0;
pub const TABS_ACTIVE_INDICATOR_RADIUS: f64 = 3.0;
pub const LIST_ONE_LINE_ROW_HEIGHT: f64 = 56.0;
pub const LIST_HORIZONTAL_INSET: f64 = 16.0;
pub const LIST_VERTICAL_INSET: f64 = 10.0;
pub const LIST_DIVIDER_LEADING_INSET: f64 = 16.0;
pub const LIST_DIVIDER_TRAILING_INSET: f64 = 16.0;
/// Hit box for the reorder grip: Material's 48dp minimum touch target,
/// holding a 24dp `drag_handle` icon.
pub const LIST_MOVE_CONTROL_WIDTH: f64 = 48.0;
/// Hit box for the delete affordance, sized like the grip beside it.
pub const LIST_DELETE_CONTROL_WIDTH: f64 = 48.0;
pub const LIST_TRAILING_CONTROL_SPACING: f64 = 8.0;
pub const LIST_TRAILING_CONTROL_VERTICAL_INSET: f64 = 4.0;
/// Material list subheader line (`md.sys.list.subheader`), a 48dp band.
pub const LIST_SECTION_HEADER_HEIGHT: f64 = 48.0;
/// Supporting text closing a section, on the same 40dp rhythm as a one-line
/// supporting paragraph.
pub const LIST_SECTION_FOOTER_HEIGHT: f64 = 40.0;
pub const TABLE_MIN_COLUMN_WIDTH: f64 = 72.0;
pub const TABLE_CELL_HORIZONTAL_PADDING: f64 = 32.0;
pub const TABLE_CELL_VERTICAL_INSET: f64 = 16.0;
pub const TABLE_HEADER_HEIGHT: f64 = 56.0;
pub const TABLE_ROW_HEIGHT: f64 = 52.0;
pub const TABLE_OUTLINE_WIDTH: f64 = 1.0;
