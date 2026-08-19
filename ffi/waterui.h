// Generate by generate_header.rs, do not modify by hand.
//
// This header declares the full waterui-ffi API surface. Which symbols are
// actually linked depends on the crate features the app was built with:
// GpuSurface/ViewEffect/AppliedFilter/GPU-runtime symbols exist only when the
// `gpu` feature (default-on) is enabled, and the C API as a whole requires
// `c-api` (Android's `android-jni` builds export JNI symbols instead).

#ifdef __cplusplus
extern "C" {
#endif


#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
typedef void *NonNull;

typedef struct WuiArraySlice {
  void *head;
  uintptr_t len;
} WuiArraySlice;

typedef struct WuiArrayVTable {
  void (*drop)(void*);
  struct WuiArraySlice (*slice)(const void*);
} WuiArrayVTable;

typedef struct WuiArray {
  void *data;
  struct WuiArrayVTable vtable;
} WuiArray;



/**
 * FFI representation of `StretchAxis` enum.
 *
 * Specifies which axis (or axes) a view stretches to fill available space.
 */
typedef enum WuiStretchAxis {
  /**
   * No stretching - view uses its intrinsic size
   */
  WuiStretchAxis_None = 0,
  /**
   * Stretch horizontally only (expand width, use intrinsic height)
   */
  WuiStretchAxis_Horizontal = 1,
  /**
   * Stretch vertically only (expand height, use intrinsic width)
   */
  WuiStretchAxis_Vertical = 2,
  /**
   * Stretch in both directions (expand width and height)
   */
  WuiStretchAxis_Both = 3,
  /**
   * Stretch along the parent container's main axis (e.g., Spacer)
   */
  WuiStretchAxis_MainAxis = 4,
  /**
   * Stretch along the parent container's cross axis (e.g., Divider)
   */
  WuiStretchAxis_CrossAxis = 5,
} WuiStretchAxis;

/**
 * FFI lifecycle enum for one-time lifecycle events.
 */
typedef enum WuiLifecycle {
  /**
   * The component appeared (attached to the view hierarchy).
   */
  WuiLifecycle_Appear,
  /**
   * The component disappeared (detached from the view hierarchy).
   */
  WuiLifecycle_Disappear,
} WuiLifecycle;

/**
 * FFI event enum for repeatable interaction events.
 */
typedef enum WuiEvent {
  /**
   * The cursor entered a component's bounds.
   */
  WuiEvent_HoverEnter,
  /**
   * Pointer motion within a component's bounds.
   */
  WuiEvent_HoverMove,
  /**
   * The cursor exited a component's bounds.
   */
  WuiEvent_HoverExit,
} WuiEvent;

/**
 * FFI-safe menu item tag.
 */
typedef enum WuiMenuItemTag {
  /**
   * A leaf command.
   */
  WuiMenuItemTag_Command = 0,
  /**
   * A divider separating adjacent items.
   */
  WuiMenuItemTag_Divider = 1,
  /**
   * A nested menu.
   */
  WuiMenuItemTag_Menu = 2,
} WuiMenuItemTag;

/**
 * FFI-safe representation of a material blur style.
 *
 * Maps to `SwiftUI`'s Material types on Apple platforms.
 */
typedef enum WuiMaterial {
  /**
   * Ultra-thin blur, most transparent.
   */
  WuiMaterial_UltraThin = 0,
  /**
   * Thin blur.
   */
  WuiMaterial_Thin = 1,
  /**
   * Regular blur (default).
   */
  WuiMaterial_Regular = 2,
  /**
   * Thick blur.
   */
  WuiMaterial_Thick = 3,
  /**
   * Ultra-thick blur, most opaque.
   */
  WuiMaterial_UltraThick = 4,
} WuiMaterial;

/**
 * C ABI mirror of [`GradientType`], the discriminator for a resolved gradient's shape.
 */
typedef enum WuiGradientType {
  /**
   * Linear gradient along a line.
   */
  WuiGradientType_Linear = 0,
  /**
   * Radial gradient from a center point.
   */
  WuiGradientType_Radial = 1,
  /**
   * Angular (conic) gradient around a center point.
   */
  WuiGradientType_Angular = 2,
  /**
   * 2D mesh gradient.
   */
  WuiGradientType_Mesh = 3,
} WuiGradientType;

/**
 * FFI-safe cursor style enum.
 */
typedef enum WuiCursorStyle {
  /**
   * Default arrow cursor (system default behavior).
   */
  WuiCursorStyle_Arrow = 0,
  /**
   * Pointing hand cursor (for clickable/link elements).
   */
  WuiCursorStyle_PointingHand = 1,
  /**
   * Text selection cursor (I-beam).
   */
  WuiCursorStyle_IBeam = 2,
  /**
   * Crosshair cursor (for precise selection).
   */
  WuiCursorStyle_Crosshair = 3,
  /**
   * Open hand cursor (for draggable content).
   */
  WuiCursorStyle_OpenHand = 4,
  /**
   * Closed hand cursor (while dragging).
   */
  WuiCursorStyle_ClosedHand = 5,
  /**
   * Not-allowed cursor (for disabled actions).
   */
  WuiCursorStyle_NotAllowed = 6,
  /**
   * Resize left cursor.
   */
  WuiCursorStyle_ResizeLeft = 7,
  /**
   * Resize right cursor.
   */
  WuiCursorStyle_ResizeRight = 8,
  /**
   * Resize up cursor.
   */
  WuiCursorStyle_ResizeUp = 9,
  /**
   * Resize down cursor.
   */
  WuiCursorStyle_ResizeDown = 10,
  /**
   * Resize left-right cursor (horizontal resize).
   */
  WuiCursorStyle_ResizeLeftRight = 11,
  /**
   * Resize up-down cursor (vertical resize).
   */
  WuiCursorStyle_ResizeUpDown = 12,
  /**
   * Move cursor (for movable content).
   */
  WuiCursorStyle_Move = 13,
  /**
   * Wait/loading cursor.
   */
  WuiCursorStyle_Wait = 14,
  /**
   * Copy cursor (for copy operations).
   */
  WuiCursorStyle_Copy = 15,
} WuiCursorStyle;

/**
 * FFI-safe representation of a drag data type tag.
 */
typedef enum WuiDragDataTag {
  /**
   * Plain text content.
   */
  WuiDragDataTag_Text = 0,
  /**
   * A URL string.
   */
  WuiDragDataTag_Url = 1,
} WuiDragDataTag;

/**
 * Color scheme enum for FFI.
 *
 * Maps directly to `waterui::theme::ColorScheme`.
 */
typedef enum WuiColorScheme {
  /**
   * Light appearance.
   */
  WuiColorScheme_Light = 0,
  /**
   * Dark appearance.
   */
  WuiColorScheme_Dark = 1,
} WuiColorScheme;

/**
 * Color slot enum for FFI.
 *
 * Each variant corresponds to a color token in `waterui::theme::color`.
 */
typedef enum WuiColorSlot {
  /**
   * Primary background color.
   */
  WuiColorSlot_Background = 0,
  /**
   * Elevated surface color (cards, sheets).
   */
  WuiColorSlot_Surface = 1,
  /**
   * Alternate surface color.
   */
  WuiColorSlot_SurfaceVariant = 2,
  /**
   * Border and divider color.
   */
  WuiColorSlot_Border = 3,
  /**
   * Primary text and icon color.
   */
  WuiColorSlot_Foreground = 4,
  /**
   * Secondary/dimmed text color.
   */
  WuiColorSlot_MutedForeground = 5,
  /**
   * Accent color for interactive elements.
   */
  WuiColorSlot_Accent = 6,
  /**
   * Foreground color on accent backgrounds.
   */
  WuiColorSlot_AccentForeground = 7,
  /**
   * Container color associated with the accent.
   */
  WuiColorSlot_AccentContainer = 8,
  /**
   * Contrasting accent color for complementary emphasis.
   */
  WuiColorSlot_Tertiary = 9,
  /**
   * Container color associated with the tertiary accent.
   */
  WuiColorSlot_TertiaryContainer = 10,
} WuiColorSlot;

/**
 * Font slot enum for FFI.
 *
 * Each variant corresponds to a font token in `waterui::text::font`.
 */
typedef enum WuiFontSlot {
  /**
   * Body text font.
   */
  WuiFontSlot_Body = 0,
  /**
   * Title font.
   */
  WuiFontSlot_Title = 1,
  /**
   * Headline font.
   */
  WuiFontSlot_Headline = 2,
  /**
   * Subheadline font.
   */
  WuiFontSlot_Subheadline = 3,
  /**
   * Caption font.
   */
  WuiFontSlot_Caption = 4,
  /**
   * Footnote font.
   */
  WuiFontSlot_Footnote = 5,
} WuiFontSlot;

/**
 *C ABI mirror of `FontWeight`.
 */
typedef enum WuiFontWeight {
  /**
   *Mirrors `FontWeight::Thin`.
   */
  WuiFontWeight_Thin,
  /**
   *Mirrors `FontWeight::UltraLight`.
   */
  WuiFontWeight_UltraLight,
  /**
   *Mirrors `FontWeight::Light`.
   */
  WuiFontWeight_Light,
  /**
   *Mirrors `FontWeight::Normal`.
   */
  WuiFontWeight_Normal,
  /**
   *Mirrors `FontWeight::Medium`.
   */
  WuiFontWeight_Medium,
  /**
   *Mirrors `FontWeight::SemiBold`.
   */
  WuiFontWeight_SemiBold,
  /**
   *Mirrors `FontWeight::Bold`.
   */
  WuiFontWeight_Bold,
  /**
   *Mirrors `FontWeight::UltraBold`.
   */
  WuiFontWeight_UltraBold,
  /**
   *Mirrors `FontWeight::Black`.
   */
  WuiFontWeight_Black,
} WuiFontWeight;

/**
 * FFI-compatible representation of [`WindowState`].
 */
typedef enum WuiWindowState {
  /**
   * The window is in its normal state.
   */
  WuiWindowState_Normal = 0,
  /**
   * The window is closed.
   */
  WuiWindowState_Closed = 1,
  /**
   * The window is minimized.
   */
  WuiWindowState_Minimized = 2,
  /**
   * The window is maximized to fullscreen.
   */
  WuiWindowState_Fullscreen = 3,
} WuiWindowState;

/**
 * FFI-compatible representation of [`WindowStyle`].
 */
typedef enum WuiWindowStyle {
  /**
   * Standard window with title bar and controls.
   */
  WuiWindowStyle_Titled = 0,
  /**
   * Borderless window without title bar.
   */
  WuiWindowStyle_Borderless = 1,
  /**
   * Window where content extends into the title bar area.
   */
  WuiWindowStyle_FullSizeContentView = 2,
} WuiWindowStyle;

/**
 * Visual presentation mode for the label slot of every control.
 */
typedef enum WuiLabelDisplayMode {
  /**
   * Use the label's own preferred presentation.
   */
  WuiLabelDisplayMode_Automatic,
  /**
   * Show both title and icon when an icon is available.
   */
  WuiLabelDisplayMode_TitleAndIcon,
  /**
   * Show only the title text.
   */
  WuiLabelDisplayMode_TitleOnly,
  /**
   * Show only the icon when an icon is available.
   */
  WuiLabelDisplayMode_IconOnly,
  /**
   * Visually hide the label entirely. The accessibility tree still
   * announces [`WuiLabel::accessibility_label`].
   */
  WuiLabelDisplayMode_Hidden,
} WuiLabelDisplayMode;

/**
 *C ABI mirror of `ButtonStyle`.
 */
typedef enum WuiButtonStyle {
  /**
   *Mirrors `ButtonStyle::Automatic`.
   */
  WuiButtonStyle_Automatic,
  /**
   *Mirrors `ButtonStyle::Plain`.
   */
  WuiButtonStyle_Plain,
  /**
   *Mirrors `ButtonStyle::Link`.
   */
  WuiButtonStyle_Link,
  /**
   *Mirrors `ButtonStyle::Borderless`.
   */
  WuiButtonStyle_Borderless,
  /**
   *Mirrors `ButtonStyle::Bordered`.
   */
  WuiButtonStyle_Bordered,
  /**
   *Mirrors `ButtonStyle::BorderedProminent`.
   */
  WuiButtonStyle_BorderedProminent,
} WuiButtonStyle;

/**
 *C ABI mirror of `KeyboardType`.
 */
typedef enum WuiKeyboardType {
  /**
   *Mirrors `KeyboardType::Text`.
   */
  WuiKeyboardType_Text,
  /**
   *Mirrors `KeyboardType::Email`.
   */
  WuiKeyboardType_Email,
  /**
   *Mirrors `KeyboardType::URL`.
   */
  WuiKeyboardType_URL,
  /**
   *Mirrors `KeyboardType::Number`.
   */
  WuiKeyboardType_Number,
  /**
   *Mirrors `KeyboardType::PhoneNumber`.
   */
  WuiKeyboardType_PhoneNumber,
} WuiKeyboardType;

/**
 *C ABI mirror of `ToggleStyle`.
 */
typedef enum WuiToggleStyle {
  /**
   *Mirrors `ToggleStyle::Automatic`.
   */
  WuiToggleStyle_Automatic,
  /**
   *Mirrors `ToggleStyle::Switch`.
   */
  WuiToggleStyle_Switch,
  /**
   *Mirrors `ToggleStyle::Checkbox`.
   */
  WuiToggleStyle_Checkbox,
} WuiToggleStyle;

/**
 *C ABI mirror of `PickerStyle`.
 */
typedef enum WuiPickerStyle {
  /**
   *Mirrors `PickerStyle::Automatic`.
   */
  WuiPickerStyle_Automatic,
  /**
   *Mirrors `PickerStyle::Menu`.
   */
  WuiPickerStyle_Menu,
  /**
   *Mirrors `PickerStyle::Radio`.
   */
  WuiPickerStyle_Radio,
  /**
   *Mirrors `PickerStyle::Segmented`.
   */
  WuiPickerStyle_Segmented,
} WuiPickerStyle;

/**
 *C ABI mirror of `DatePickerType`.
 */
typedef enum WuiDatePickerType {
  /**
   *Mirrors `DatePickerType::Date`.
   */
  WuiDatePickerType_Date,
  /**
   *Mirrors `DatePickerType::HourAndMinute`.
   */
  WuiDatePickerType_HourAndMinute,
  /**
   *Mirrors `DatePickerType::HourMinuteAndSecond`.
   */
  WuiDatePickerType_HourMinuteAndSecond,
  /**
   *Mirrors `DatePickerType::DateHourAndMinute`.
   */
  WuiDatePickerType_DateHourAndMinute,
  /**
   *Mirrors `DatePickerType::DateHourMinuteAndSecond`.
   */
  WuiDatePickerType_DateHourMinuteAndSecond,
} WuiDatePickerType;

/**
 *C ABI mirror of `ProgressStyle`.
 */
typedef enum WuiProgressStyle {
  /**
   *Mirrors `ProgressStyle::Linear`.
   */
  WuiProgressStyle_Linear,
  /**
   *Mirrors `ProgressStyle::Circular`.
   */
  WuiProgressStyle_Circular,
  /**
   *Mirrors `ProgressStyle::Loading`.
   */
  WuiProgressStyle_Loading,
} WuiProgressStyle;

/**
 * FFI representation of map display style.
 */
typedef enum WuiMapStyle {
  /**
   * Standard road map.
   */
  WuiMapStyle_Standard = 0,
  /**
   * Satellite imagery.
   */
  WuiMapStyle_Satellite = 1,
  /**
   * Hybrid of satellite and roads.
   */
  WuiMapStyle_Hybrid = 2,
} WuiMapStyle;

/**
 * Discriminant of [`WuiMapStatus`].
 */
typedef enum WuiMapStatusKind {
  /**
   * The map is loading what it needs to draw.
   */
  WuiMapStatusKind_Loading = 0,
  /**
   * The map has everything it needs.
   */
  WuiMapStatusKind_Ready = 1,
  /**
   * The map failed to load.
   */
  WuiMapStatusKind_Failed = 2,
} WuiMapStatusKind;

/**
 * FFI-safe horizontal paragraph alignment.
 */
typedef enum WuiHorizontalAlignment {
  /**
   * Align to the leading edge (left in left-to-right locales).
   */
  WuiHorizontalAlignment_Leading = 0,
  /**
   * Center within the available width.
   */
  WuiHorizontalAlignment_Center = 1,
  /**
   * Align to the trailing edge (right in left-to-right locales).
   */
  WuiHorizontalAlignment_Trailing = 2,
} WuiHorizontalAlignment;

/**
 * C ABI mirror of [`VerticalAlignment`], a named vertical alignment guide
 * used for stack cross-axis alignment and explicit view dimension guides.
 */
typedef enum WuiVerticalAlignment {
  /**
   * Aligns to the top edge.
   */
  WuiVerticalAlignment_Top = 0,
  /**
   * Aligns to the vertical center.
   */
  WuiVerticalAlignment_Center = 1,
  /**
   * Aligns to the bottom edge.
   */
  WuiVerticalAlignment_Bottom = 2,
  /**
   * Aligns to the first line's text baseline.
   */
  WuiVerticalAlignment_FirstBaseline = 3,
  /**
   * Aligns to the last line's text baseline.
   */
  WuiVerticalAlignment_LastBaseline = 4,
} WuiVerticalAlignment;

/**
 * The stacking axis a layout advertises for lazy (virtualized) rendering,
 * as reported by [`waterui_layout_lazy_stack_axis`].
 */
typedef enum WuiLazyStackAxis {
  /**
   * The layout does not support lazy stacking (not a `VStackLayout`/`HStackLayout`).
   */
  WuiLazyStackAxis_Unsupported = 0,
  /**
   * The layout stacks children vertically (`VStackLayout`).
   */
  WuiLazyStackAxis_Vertical = 1,
  /**
   * The layout stacks children horizontally (`HStackLayout`).
   */
  WuiLazyStackAxis_Horizontal = 2,
} WuiLazyStackAxis;

/**
 *C ABI mirror of `Axis`.
 */
typedef enum WuiAxis {
  /**
   *Mirrors `Axis::Horizontal`.
   */
  WuiAxis_Horizontal,
  /**
   *Mirrors `Axis::Vertical`.
   */
  WuiAxis_Vertical,
  /**
   *Mirrors `Axis::All`.
   */
  WuiAxis_All,
} WuiAxis;

/**
 * FFI representation of a media delivery strategy.
 */
typedef enum WuiVideoDelivery {
  /**
   * A progressively downloaded single media file.
   */
  WuiVideoDelivery_Progressive = 0,
  /**
   * HTTP Live Streaming (HLS).
   */
  WuiVideoDelivery_Hls = 1,
  /**
   * MPEG-DASH adaptive streaming.
   */
  WuiVideoDelivery_Dash = 2,
} WuiVideoDelivery;

/**
 * Tagged subtitle selection used by the native player.
 */
typedef enum WuiSubtitleSelectionType {
  /**
   * Let the player choose a subtitle track automatically.
   */
  WuiSubtitleSelectionType_Auto = 0,
  /**
   * Disable subtitles.
   */
  WuiSubtitleSelectionType_Off = 1,
  /**
   * Select a specific subtitle track by index.
   */
  WuiSubtitleSelectionType_Track = 2,
} WuiSubtitleSelectionType;

/**
 * Tagged audio track selection used by the native player.
 */
typedef enum WuiAudioTrackSelectionType {
  /**
   * Let the player choose an audio track automatically.
   */
  WuiAudioTrackSelectionType_Auto = 0,
  /**
   * Select a specific audio track by index.
   */
  WuiAudioTrackSelectionType_Track = 1,
} WuiAudioTrackSelectionType;

/**
 * Tagged video-quality track selection used by the native player.
 */
typedef enum WuiVideoTrackSelectionType {
  /**
   * Let the player choose a video-quality track automatically.
   */
  WuiVideoTrackSelectionType_Auto = 0,
  /**
   * Select a specific video-quality track by index.
   */
  WuiVideoTrackSelectionType_Track = 1,
} WuiVideoTrackSelectionType;

/**
 * Native subtitle-track origin.
 */
typedef enum WuiVideoSubtitleTrackOrigin {
  /**
   * Loaded from a separate sidecar subtitle file.
   */
  WuiVideoSubtitleTrackOrigin_Sidecar = 0,
  /**
   * Embedded directly in the media container.
   */
  WuiVideoSubtitleTrackOrigin_Embedded = 1,
  /**
   * Declared by an adaptive-streaming manifest (e.g. HLS/DASH).
   */
  WuiVideoSubtitleTrackOrigin_Manifest = 2,
  /**
   * Sourced from a platform-native subtitle mechanism.
   */
  WuiVideoSubtitleTrackOrigin_Native = 3,
} WuiVideoSubtitleTrackOrigin;

/**
 * FFI representation of video events.
 */
typedef enum WuiVideoEventType {
  /**
   * Playback is ready to begin (enough data buffered to play).
   */
  WuiVideoEventType_ReadyToPlay = 0,
  /**
   * Playback reached the end of the media.
   */
  WuiVideoEventType_Ended = 1,
  /**
   * A fatal playback error occurred; see `WuiVideoEvent::error_message`.
   */
  WuiVideoEventType_Error = 2,
  /**
   * Playback stalled and is buffering.
   */
  WuiVideoEventType_Buffering = 3,
  /**
   * A prior buffering stall has ended.
   */
  WuiVideoEventType_BufferingEnded = 4,
  /**
   * The amount of buffered-ahead media changed.
   */
  WuiVideoEventType_BufferLevel = 5,
  /**
   * Updated playback quality-of-service metrics are available.
   */
  WuiVideoEventType_PlaybackMetrics = 6,
  /**
   * Picture-in-Picture presentation was entered or exited.
   */
  WuiVideoEventType_PictureInPictureChanged = 7,
  /**
   * The user requested skipping to the next playlist item.
   */
  WuiVideoEventType_NextRequested = 8,
  /**
   * The user requested skipping to the previous playlist item.
   */
  WuiVideoEventType_PreviousRequested = 9,
  /**
   * The playing/paused state changed.
   */
  WuiVideoEventType_PlaybackStateChanged = 10,
  /**
   * External playback (e.g., `AirPlay`, cast) was engaged or disengaged.
   */
  WuiVideoEventType_ExternalPlaybackChanged = 11,
  /**
   * The active playback output architecture changed.
   */
  WuiVideoEventType_PlaybackOutputPathChanged = 12,
} WuiVideoEventType;

/**
 * Native representation of the active playback output architecture.
 */
typedef enum WuiVideoPlaybackOutputPath {
  /**
   * The platform's default managed decode/render pipeline.
   */
  WuiVideoPlaybackOutputPath_PlatformManaged = 0,
  /**
   * Decoded PCM audio routed through the GPU-adjacent pipeline.
   */
  WuiVideoPlaybackOutputPath_DecodedPcmGpu = 1,
  /**
   * Compressed audio handed off to a hardware offload path.
   */
  WuiVideoPlaybackOutputPath_AudioOffload = 2,
  /**
   * Audio and video routed through hardware tunneling.
   */
  WuiVideoPlaybackOutputPath_AudioVideoTunneling = 3,
} WuiVideoPlaybackOutputPath;

/**
 * Native projection of a required playback power path.
 */
typedef enum WuiVideoPlaybackPowerPolicy {
  /**
   * Let the platform choose the playback power path.
   */
  WuiVideoPlaybackPowerPolicy_PlatformManaged = 0,
  /**
   * Require hardware audio offload decoding.
   */
  WuiVideoPlaybackPowerPolicy_RequireAudioOffload = 1,
  /**
   * Require combined audio/video hardware tunneling.
   */
  WuiVideoPlaybackPowerPolicy_RequireAudioVideoTunneling = 2,
} WuiVideoPlaybackPowerPolicy;

/**
 * FFI representation of how the picture fills the bounds it is given.
 */
typedef enum WuiContentMode {
  /**
   * Scale to fit entirely within the bounds, preserving aspect ratio.
   */
  WuiContentMode_Fit = 0,
  /**
   * Scale to fill the bounds, preserving aspect ratio and cropping overflow.
   */
  WuiContentMode_Fill = 1,
  /**
   * Stretch to exactly fill the bounds, ignoring aspect ratio.
   */
  WuiContentMode_Stretch = 2,
} WuiContentMode;

/**
 * Native projection support discriminator.
 */
typedef enum WuiVideoProjection {
  /**
   * A conventional flat (non-projected) video frame.
   */
  WuiVideoProjection_Rectilinear = 0,
  /**
   * A 360-degree equirectangular projection.
   */
  WuiVideoProjection_Equirectangular = 1,
} WuiVideoProjection;

/**
 * Semantic native toolbar placement.
 */
typedef enum WuiNavigationToolbarPlacement {
  /**
   * The default, platform-chosen placement.
   */
  WuiNavigationToolbarPlacement_Principal = 0,
  /**
   * The primary action slot (e.g., a trailing "Save" or "Done" button).
   */
  WuiNavigationToolbarPlacement_PrimaryAction = 1,
  /**
   * The secondary action slot (e.g., a leading "Edit" button).
   */
  WuiNavigationToolbarPlacement_SecondaryAction = 2,
  /**
   * A confirmation action within a modal presentation.
   */
  WuiNavigationToolbarPlacement_Confirmation = 3,
  /**
   * A cancellation action within a modal presentation.
   */
  WuiNavigationToolbarPlacement_Cancellation = 4,
  /**
   * The bottom toolbar bar.
   */
  WuiNavigationToolbarPlacement_BottomBar = 5,
  /**
   * A status item, typically centered.
   */
  WuiNavigationToolbarPlacement_Status = 6,
  /**
   * The leading slot of the top bar.
   */
  WuiNavigationToolbarPlacement_TopBarLeading = 7,
  /**
   * The trailing slot of the top bar.
   */
  WuiNavigationToolbarPlacement_TopBarTrailing = 8,
} WuiNavigationToolbarPlacement;

/**
 * The display mode for the navigation bar title (FFI-compatible).
 */
typedef enum WuiNavigationTitleDisplayMode {
  /**
   * System decides based on context.
   */
  WuiNavigationTitleDisplayMode_Automatic = 0,
  /**
   * Always use inline (small) title.
   */
  WuiNavigationTitleDisplayMode_Inline = 1,
  /**
   * Always use a medium title, between inline and large.
   */
  WuiNavigationTitleDisplayMode_Medium = 2,
  /**
   * Always use large title.
   */
  WuiNavigationTitleDisplayMode_Large = 3,
} WuiNavigationTitleDisplayMode;

/**
 * FFI representation of the kind of transition used for a navigation push/pop.
 */
typedef enum WuiNavigationTransitionKind {
  /**
   * The platform-default transition.
   */
  WuiNavigationTransitionKind_Automatic = 0,
  /**
   * A cross-fade transition.
   */
  WuiNavigationTransitionKind_Fade = 1,
  /**
   * A zoom transition anchored at `WuiNavigationTransition::source_id`.
   */
  WuiNavigationTransitionKind_Zoom = 2,
  /**
   * No transition; the destination appears instantly.
   */
  WuiNavigationTransitionKind_None = 3,
  /**
   * A caller-supplied custom transition.
   */
  WuiNavigationTransitionKind_Custom = 4,
} WuiNavigationTransitionKind;

/**
 * FFI representation of a `NavigationSplitLayout`'s native presentation style.
 */
typedef enum WuiNavigationSplitStyle {
  /**
   * The platform-default split style.
   */
  WuiNavigationSplitStyle_Automatic = 0,
  /**
   * Sidebar and detail columns share space evenly.
   */
  WuiNavigationSplitStyle_Balanced = 1,
  /**
   * The detail column is given prominence over the sidebar.
   */
  WuiNavigationSplitStyle_ProminentDetail = 2,
} WuiNavigationSplitStyle;

/**
 * Native adaptive tab style.
 */
typedef enum WuiTabStyle {
  /**
   * The platform-default tab presentation.
   */
  WuiTabStyle_Automatic = 0,
  /**
   * A bottom tab bar.
   */
  WuiTabStyle_TabBar = 1,
  /**
   * A sidebar of tabs.
   */
  WuiTabStyle_Sidebar = 2,
} WuiTabStyle;

/**
 * Pointer buttons supported by CEF windowless rendering.
 */
typedef enum WuiCefPointerButton {
  /**
   * Primary pointer button.
   */
  WuiCefPointerButton_Primary,
  /**
   * Middle pointer button.
   */
  WuiCefPointerButton_Middle,
  /**
   * Secondary pointer button.
   */
  WuiCefPointerButton_Secondary,
} WuiCefPointerButton;

/**
 * Editing operations forwarded to Chromium's focused frame.
 */
typedef enum WuiCefEditCommand {
  /**
   * Undo.
   */
  WuiCefEditCommand_Undo,
  /**
   * Redo.
   */
  WuiCefEditCommand_Redo,
  /**
   * Cut.
   */
  WuiCefEditCommand_Cut,
  /**
   * Copy.
   */
  WuiCefEditCommand_Copy,
  /**
   * Paste.
   */
  WuiCefEditCommand_Paste,
  /**
   * Select all.
   */
  WuiCefEditCommand_SelectAll,
} WuiCefEditCommand;

/**
 * Which of the bridge's two channels a handler's reply crosses on.
 *
 * The page sees the difference — a value it can use, or a `Uint8Array` — so the
 * answer has to survive the trip out. It used to be dropped here, and every
 * reply reached the page as base64 text.
 */
typedef enum WuiJsReplyKind {
  /**
   * The bytes are a serialized JSON value; the page receives the value.
   */
  WuiJsReplyKind_Json = 0,
  /**
   * The bytes are opaque; the page receives a `Uint8Array`.
   */
  WuiJsReplyKind_Bytes = 1,
} WuiJsReplyKind;

/**
 * FFI representation of script injection timing.
 */
typedef enum WuiScriptInjectionTime {
  /**
   * Inject at the start of document loading, before the DOM is constructed.
   */
  WuiScriptInjectionTime_DocumentStart = 0,
  /**
   * Inject after the document has finished loading.
   */
  WuiScriptInjectionTime_DocumentEnd = 1,
} WuiScriptInjectionTime;

/**
 * FFI representation of `WebView` event types.
 */
typedef enum WuiWebViewEventType {
  /**
   * The web view is about to navigate to a new URL.
   */
  WuiWebViewEventType_WillNavigate = 1,
  /**
   * The web view is loading content.
   */
  WuiWebViewEventType_Loading = 2,
  /**
   * The web view has finished loading.
   */
  WuiWebViewEventType_Loaded = 3,
  /**
   * A redirect occurred.
   */
  WuiWebViewEventType_Redirect = 4,
  /**
   * An SSL error occurred.
   */
  WuiWebViewEventType_SslError = 5,
  /**
   * A general error occurred.
   */
  WuiWebViewEventType_Error = 6,
  /**
   * Navigation state changed.
   */
  WuiWebViewEventType_StateChanged = 7,
} WuiWebViewEventType;

/**
 * 2D affine transform stored as a row-major 2x3 matrix.
 *
 * The transform maps a point `(x, y)` to:
 *
 * ```text
 * x' = a * x + c * y + e
 * y' = b * x + d * y + f
 * ```
 *
 * Layout matches the canonical 6-coefficient ordering used by `kurbo` and the
 * HTML Canvas 2D context, so a transform built here can be losslessly handed to
 * a 2D rendering backend.
 */
typedef struct Affine2 Affine2;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_AudioTrackSelection Binding_AudioTrackSelection;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_Color Binding_Color;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_DateTime Binding_DateTime;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_Id Binding_Id;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_MapStatus Binding_MapStatus;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_Option_LiveWindow Binding_Option_LiveWindow;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_Rect Binding_Rect;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_Secure Binding_Secure;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_Str Binding_Str;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_StyledStr Binding_StyledStr;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_SubtitleSelection Binding_SubtitleSelection;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_TrackCatalog Binding_TrackCatalog;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_Vec_Date Binding_Vec_Date;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_VideoTrackSelection Binding_VideoTrackSelection;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_WindowState Binding_WindowState;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_bool Binding_bool;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_f32 Binding_f32;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_f64 Binding_f64;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_i32 Binding_i32;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_ColorScheme Computed_ColorScheme;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_CursorStyle Computed_CursorStyle;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_Delivery Computed_Delivery;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_HorizontalAlignment Computed_HorizontalAlignment;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_Option_Location Computed_Option_Location;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_Region Computed_Region;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_ResolvedColor Computed_ResolvedColor;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_ResolvedFont Computed_ResolvedFont;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_Size Computed_Size;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_Str Computed_Str;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_StyledStr Computed_StyledStr;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_Vec_Annotation Computed_Vec_Annotation;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_Vec_Date Computed_Vec_Date;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_bool Computed_bool;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_f32 Computed_f32;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_f64 Computed_f64;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_i32 Computed_i32;

/**
 * Specifies which edges should ignore safe area insets.
 *
 * Used with `IgnoreSafeArea` to control which edges of a view
 * should extend into the unsafe screen regions.
 */
typedef struct EdgeSet EdgeSet;

/**
 * A size proposal from parent to child during layout negotiation.
 *
 * Each dimension can be:
 * - `None` - "Tell me your ideal size" (unspecified)
 * - `Some(0.0)` - "Tell me your minimum size"
 * - `Some(f32::INFINITY)` - "Tell me your maximum size"
 * - `Some(value)` - "I suggest you use this size"
 *
 * Children are free to return any size; the proposal is just a suggestion.
 */
typedef struct ProposalSize ProposalSize;

/**
 * Normalized coordinates (0.0–1.0) for positioning and gradient endpoints.
 *
 * Used to specify both anchor points on views and target positions in parents.
 * Values outside `0.0..=1.0` are valid and will position outside bounds.
 */
typedef struct UnitPoint UnitPoint;

/**
 * Two-dimensional displacement vector (e.g. velocity, gravity, wind, offset).
 *
 * Distinct from [`Point`], which represents an absolute position. A `Vec2`
 * encodes a delta and is the natural input for physics-style modifiers such
 * as `gravity(...)` and `wind(...)` on a particle system.
 */
typedef struct Vec2 Vec2;

/**
 *Opaque FFI handle owning a `RetainedCallback<BoxedAction<()>>`.
 */
typedef struct WuiAction WuiAction;

/**
 *Opaque FFI handle owning a `waterui::AnyView`.
 */
typedef struct WuiAnyView WuiAnyView;

/**
 *Opaque FFI handle owning a `AnyViews<AnyView>`.
 */
typedef struct WuiAnyViews WuiAnyViews;

/**
 * Opaque state held by the native backend for one semantic applied filter.
 */
typedef struct WuiAppliedFilterState WuiAppliedFilterState;

typedef struct WuiCefSurfaceState WuiCefSurfaceState;

/**
 *Opaque FFI handle owning a `Color`.
 */
typedef struct WuiColor WuiColor;

/**
 * Opaque wrapper for Draggable.
 */
typedef struct WuiDraggableWrapper WuiDraggableWrapper;

/**
 * Wrapper for `DropDestination` to avoid orphan rule issues.
 */
typedef struct WuiDropHandler WuiDropHandler;

/**
 *Opaque FFI handle owning a `Dynamic`.
 */
typedef struct WuiDynamic WuiDynamic;

/**
 *Opaque FFI handle owning a `waterui::Environment`.
 */
typedef struct WuiEnv WuiEnv;

/**
 *Opaque FFI handle owning a `Font`.
 */
typedef struct WuiFont WuiFont;

/**
 * Submission fence returned after encoding an external-texture capture.
 *
 * The renderer state remains on its owning UI thread. Native consumes this
 * token by registering a completion with
 * [`waterui_gpu_capture_fence_on_complete`].
 */
typedef struct WuiGpuCaptureFence WuiGpuCaptureFence;

/**
 *Opaque FFI handle owning a `GpuRuntime`.
 */
typedef struct WuiGpuRuntime WuiGpuRuntime;

/**
 * Opaque state held by the native backend after initialization.
 *
 * Owns the semantic renderer and environment GPU runtime independently of the
 * currently attached presentation surface.
 */
typedef struct WuiGpuSurfaceState WuiGpuSurfaceState;

/**
 *Opaque FFI handle owning a `RetainedCallback<IndexHandler>`.
 */
typedef struct WuiIndexAction WuiIndexAction;

/**
 *Opaque FFI handle owning a `Box<dyn Layout>`.
 */
typedef struct WuiLayout WuiLayout;

/**
 * Retained native invalidation target and its precise signal subscriptions.
 */
typedef struct WuiLayoutWatcher WuiLayoutWatcher;

/**
 * Wrapper for `LifeCycleHook` to avoid orphan rule issues.
 */
typedef struct WuiLifecycleHookHandler WuiLifecycleHookHandler;

/**
 *Opaque FFI handle owning a `RetainedCallback<MoveHandler>`.
 */
typedef struct WuiMoveAction WuiMoveAction;

/**
 * Wrapper for `OnEvent` to avoid orphan rule issues.
 */
typedef struct WuiOnEventHandler WuiOnEventHandler;

/**
 *Opaque FFI handle owning a `SharedAction<()>`.
 */
typedef struct WuiSharedAction WuiSharedAction;

/**
 *Opaque FFI handle owning a `AnyViewBuilder<NavigationView>`.
 */
typedef struct WuiTabContent WuiTabContent;

/**
 *Opaque FFI handle owning a `PlayerController`.
 */
typedef struct WuiVideoController WuiVideoController;

/**
 *Opaque FFI handle owning a `Rc<BoundVideoEventHandler>`.
 */
typedef struct WuiVideoEventHandler WuiVideoEventHandler;

/**
 * Opaque state held by the native backend after initialization.
 */
typedef struct WuiViewEffectState WuiViewEffectState;

/**
 *Opaque FFI handle owning a `BoxWatcherGuard`.
 */
typedef struct WuiWatcherGuard WuiWatcherGuard;

/**
 *Opaque FFI handle owning a `Metadata`.
 */
typedef struct WuiWatcherMetadata WuiWatcherMetadata;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_AnyView WuiWatcher_AnyView;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_AudioTrackSelection WuiWatcher_AudioTrackSelection;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_Color WuiWatcher_Color;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_ColorScheme WuiWatcher_ColorScheme;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_CursorStyle WuiWatcher_CursorStyle;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_DateTime WuiWatcher_DateTime;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_Delivery WuiWatcher_Delivery;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_HorizontalAlignment WuiWatcher_HorizontalAlignment;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_Id WuiWatcher_Id;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_MapStatus WuiWatcher_MapStatus;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_Option_Location WuiWatcher_Option_Location;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_Rect WuiWatcher_Rect;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_Region WuiWatcher_Region;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_ResolvedColor WuiWatcher_ResolvedColor;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_ResolvedFont WuiWatcher_ResolvedFont;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_Secure WuiWatcher_Secure;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_Size WuiWatcher_Size;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_Str WuiWatcher_Str;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_StyledStr WuiWatcher_StyledStr;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_SubtitleSelection WuiWatcher_SubtitleSelection;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_Vec_Annotation WuiWatcher_Vec_Annotation;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_Vec_Date WuiWatcher_Vec_Date;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_VideoTrackSelection WuiWatcher_VideoTrackSelection;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_WindowState WuiWatcher_WindowState;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_bool WuiWatcher_bool;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_f32 WuiWatcher_f32;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_f64 WuiWatcher_f64;

/**
 * FFI-owned wrapper around a native watcher callback.
 *
 * Bridges a C function pointer pair (`call`/`drop`) into a Rust [`Watcher`]
 * that can be registered with a [`WuiComputed`] or [`WuiBinding`].
 */
typedef struct WuiWatcher_i32 WuiWatcher_i32;

/**
 *Opaque FFI handle owning a `WebView`.
 */
typedef struct WuiWebView WuiWebView;

/**
 * Type ID as a 128-bit value for O(1) comparison.
 *
 * Uses 128-bit FNV-1a hash of `type_name()` for stability across dylib boundaries,
 * which is required for the preview system that loads user code as a dylib.
 */
typedef struct WuiTypeId {
  /**
   * The low 64 bits of the 128-bit FNV-1a type hash.
   */
  uint64_t low;
  /**
   * The high 64 bits of the 128-bit FNV-1a type hash.
   */
  uint64_t high;
} WuiTypeId;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_bool WuiComputed_bool;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_u8 {
  uint8_t *head;
  uintptr_t len;
} WuiArraySlice_u8;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_u8 {
  void (*drop)(void*);
  struct WuiArraySlice_u8 (*slice)(const void*);
} WuiArrayVTable_u8;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_u8 {
  NonNull data;
  struct WuiArrayVTable_u8 vtable;
} WuiArray_u8;

/**
 * UTF-8 string represented as a byte array.
 */
typedef struct WuiStr {
  struct WuiArray_u8 _0;
} WuiStr;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_____WuiEnv {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiEnv *value;
} WuiMetadata_____WuiEnv;

/**
 * Type alias for Metadata<Environment> FFI struct
 * Layout: { content: *mut `WuiAnyView`, value: *mut `WuiEnv` }
 */
typedef struct WuiMetadata_____WuiEnv WuiMetadataEnv;

/**
 * FFI-compatible representation of [`waterui_core::id::Id`].
 */
typedef struct WuiId {
  /**
   * The inner integer value of the ID.
   */
  int32_t inner;
} WuiId;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiId {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiId value;
} WuiMetadata_WuiId;

/**
 * Source geometry metadata for native navigation transitions.
 */
typedef struct WuiMetadata_WuiId WuiMetadataNavigationTransitionSource;

/**
 * Destination geometry metadata for native navigation transitions.
 */
typedef struct WuiMetadata_WuiId WuiMetadataNavigationTransitionDestination;

/**
 * C-compatible empty marker struct for Secure metadata.
 * This is needed because `()` (unit type) is not representable in C.
 */
typedef struct WuiSecureMarker {
  /**
   * Placeholder field to ensure struct has valid size in C.
   * The actual value is meaningless - Secure is just a marker type.
   */
  uint8_t _marker;
} WuiSecureMarker;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiSecureMarker {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiSecureMarker value;
} WuiMetadata_WuiSecureMarker;

/**
 * Type alias for Metadata<Secure> FFI struct
 * Layout: { content: *mut `WuiAnyView`, value: `WuiSecureMarker` }
 */
typedef struct WuiMetadata_WuiSecureMarker WuiMetadataSecure;

/**
 * C-compatible empty marker struct for dynamic range metadata.
 */
typedef struct WuiDynamicRangeMarker {
  /**
   * Placeholder field so the struct has a valid size in C; the value is
   * meaningless.
   */
  uint8_t _marker;
} WuiDynamicRangeMarker;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiDynamicRangeMarker {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiDynamicRangeMarker value;
} WuiMetadata_WuiDynamicRangeMarker;

/**
 * Type alias for Metadata<StandardDynamicRange> FFI struct.
 */
typedef struct WuiMetadata_WuiDynamicRangeMarker WuiMetadataStandardDynamicRange;

/**
 * Type alias for Metadata<HighDynamicRange> FFI struct.
 */
typedef struct WuiMetadata_WuiDynamicRangeMarker WuiMetadataHighDynamicRange;

/**
 * FFI-safe representation of a gesture type.
 */
typedef enum WuiGesture_Tag {
  /**
   * A tap gesture requiring a specific number of taps.
   */
  WuiGesture_Tap,
  /**
   * A long-press gesture requiring a minimum duration.
   */
  WuiGesture_LongPress,
  /**
   * A drag gesture with minimum distance threshold.
   */
  WuiGesture_Drag,
  /**
   * A magnification (pinch) gesture with initial scale.
   */
  WuiGesture_Magnification,
  /**
   * A rotation gesture with initial angle.
   */
  WuiGesture_Rotation,
  /**
   * A sequential composition of two gestures.
   */
  WuiGesture_Then,
  /**
   * A parallel composition of two gestures.
   */
  WuiGesture_Simultaneous,
  /**
   * An exclusive composition where first has priority over second.
   */
  WuiGesture_Exclusive,
} WuiGesture_Tag;

typedef struct WuiGesture_Tap_Body {
  /**
   * Number of taps required to recognize the gesture.
   */
  uint32_t count;
} WuiGesture_Tap_Body;

typedef struct WuiGesture_LongPress_Body {
  /**
   * Minimum press duration in milliseconds before the gesture fires.
   */
  uint32_t duration;
} WuiGesture_LongPress_Body;

typedef struct WuiGesture_Drag_Body {
  /**
   * Minimum drag distance (in points) before the gesture fires.
   */
  float min_distance;
} WuiGesture_Drag_Body;

typedef struct WuiGesture_Magnification_Body {
  /**
   * Scale factor the gesture starts recognizing from.
   */
  float initial_scale;
} WuiGesture_Magnification_Body;

typedef struct WuiGesture_Rotation_Body {
  /**
   * Angle (in radians) the gesture starts recognizing from.
   */
  float initial_angle;
} WuiGesture_Rotation_Body;

typedef struct WuiGesture_Then_Body {
  /**
   * The first gesture that must complete.
   */
  struct WuiGesture *first;
  /**
   * The gesture that runs after the first completes.
   */
  struct WuiGesture *then;
} WuiGesture_Then_Body;

typedef struct WuiGesture_Simultaneous_Body {
  /**
   * The first gesture in the composition.
   */
  struct WuiGesture *first;
  /**
   * The second gesture in the composition.
   */
  struct WuiGesture *second;
} WuiGesture_Simultaneous_Body;

typedef struct WuiGesture_Exclusive_Body {
  /**
   * The primary gesture.
   */
  struct WuiGesture *first;
  /**
   * The fallback gesture.
   */
  struct WuiGesture *second;
} WuiGesture_Exclusive_Body;

typedef struct WuiGesture {
  WuiGesture_Tag tag;
  union {
    WuiGesture_Tap_Body tap;
    WuiGesture_LongPress_Body long_press;
    WuiGesture_Drag_Body drag;
    WuiGesture_Magnification_Body magnification;
    WuiGesture_Rotation_Body rotation;
    WuiGesture_Then_Body then;
    WuiGesture_Simultaneous_Body simultaneous;
    WuiGesture_Exclusive_Body exclusive;
  };
} WuiGesture;

/**
 * FFI-safe representation of a gesture observer.
 */
typedef struct WuiGestureObserver {
  /**
   * The gesture type to observe.
   */
  struct WuiGesture gesture;
  /**
   * Pointer to the action handler.
   */
  struct WuiAction *action;
} WuiGestureObserver;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiGestureObserver {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiGestureObserver value;
} WuiMetadata_WuiGestureObserver;

/**
 * Type alias for Metadata<GestureObserver> FFI struct
 */
typedef struct WuiMetadata_WuiGestureObserver WuiMetadataGesture;

/**
 * FFI-safe representation of a lifecycle hook.
 */
typedef struct WuiLifecycleHook {
  /**
   * The lifecycle event to listen for.
   */
  enum WuiLifecycle lifecycle;
  /**
   * Opaque pointer to the lifecycle hook (owns the handler).
   */
  struct WuiLifecycleHookHandler *handler;
} WuiLifecycleHook;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiLifecycleHook {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiLifecycleHook value;
} WuiMetadata_WuiLifecycleHook;

/**
 * Type alias for `Metadata<LifeCycleHook>` FFI struct.
 */
typedef struct WuiMetadata_WuiLifecycleHook WuiMetadataLifecycleHook;

/**
 * FFI-safe representation of an event handler.
 */
typedef struct WuiOnEvent {
  /**
   * The event type to listen for.
   */
  enum WuiEvent event;
  /**
   * Opaque pointer to the `OnEvent` (owns the handler).
   */
  struct WuiOnEventHandler *handler;
} WuiOnEvent;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiOnEvent {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiOnEvent value;
} WuiMetadata_WuiOnEvent;

/**
 * Type alias for Metadata<OnEvent> FFI struct
 */
typedef struct WuiMetadata_WuiOnEvent WuiMetadataOnEvent;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_CursorStyle WuiComputed_CursorStyle;

/**
 * FFI-safe representation of cursor metadata.
 */
typedef struct WuiCursor {
  /**
   * The cursor style (reactive).
   */
  WuiComputed_CursorStyle *style;
} WuiCursor;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiCursor {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiCursor value;
} WuiMetadata_WuiCursor;

/**
 * Type alias for Metadata<Cursor> FFI struct
 */
typedef struct WuiMetadata_WuiCursor WuiMetadataCursor;

/**
 * FFI-safe representation of `IgnorableMetadata`<`AccessibilityIdentifier`>
 */
typedef struct WuiIgnorableMetadataAccessibilityIdentifier {
  /**
   * The view content wrapped by this metadata
   */
  struct WuiAnyView *content;
  /**
   * The stable automation identifier
   */
  struct WuiStr identifier;
} WuiIgnorableMetadataAccessibilityIdentifier;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_StyledStr WuiComputed_StyledStr;

/**
 * Reactive accessibility label metadata.
 */
typedef struct WuiIgnorableMetadataAccessibilityLabel {
  /**
   * Wrapped view content.
   */
  struct WuiAnyView *content;
  /**
   * Resolved semantic label.
   */
  WuiComputed_StyledStr *label;
} WuiIgnorableMetadataAccessibilityLabel;

/**
 * Accessibility metadata containing a static integer value.
 */
typedef struct WuiIgnorableMetadataAccessibilityValue {
  /**
   * Wrapped view content.
   */
  struct WuiAnyView *content;
  /**
   * Backend-independent semantic value.
   */
  int32_t value;
} WuiIgnorableMetadataAccessibilityValue;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_i32 WuiComputed_i32;

/**
 * Reactive accessibility state split into primitive signals for native APIs.
 */
typedef struct WuiAccessibilityState {
  /**
   * Whether the semantic node is disabled.
   */
  WuiComputed_bool *disabled;
  /**
   * Whether the semantic node is selected.
   */
  WuiComputed_bool *selected;
  /**
   * -1 is absent, 0 false, 1 true, and 2 mixed.
   */
  WuiComputed_i32 *checked;
  /**
   * -1 is absent, 0 collapsed, and 1 expanded.
   */
  WuiComputed_i32 *expanded;
  /**
   * Whether the semantic node is busy.
   */
  WuiComputed_bool *busy;
  /**
   * Whether the semantic node is hidden.
   */
  WuiComputed_bool *hidden;
} WuiAccessibilityState;

/**
 * Accessibility state metadata wrapper.
 */
typedef struct WuiIgnorableMetadataAccessibilityState {
  /**
   * Wrapped view content.
   */
  struct WuiAnyView *content;
  /**
   * Reactive semantic state.
   */
  struct WuiAccessibilityState state;
} WuiIgnorableMetadataAccessibilityState;

/**
 * FFI-safe representation of a shadow.
 */
typedef struct WuiShadow {
  /**
   * Shadow color (as opaque pointer - needs environment to resolve).
   */
  struct WuiColor *color;
  /**
   * Horizontal offset.
   */
  float offset_x;
  /**
   * Vertical offset.
   */
  float offset_y;
  /**
   * Blur radius.
   */
  float radius;
} WuiShadow;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiShadow {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiShadow value;
} WuiMetadata_WuiShadow;

/**
 * Type alias for Metadata<Shadow> FFI struct
 */
typedef struct WuiMetadata_WuiShadow WuiMetadataShadow;

/**
 * FFI-safe representation of edge set for safe area.
 */
typedef struct WuiEdgeSet {
  /**
   * Ignore safe area on top edge.
   */
  bool top;
  /**
   * Ignore safe area on leading edge.
   */
  bool leading;
  /**
   * Ignore safe area on bottom edge.
   */
  bool bottom;
  /**
   * Ignore safe area on trailing edge.
   */
  bool trailing;
} WuiEdgeSet;

/**
 * FFI-safe representation of a border.
 */
typedef struct WuiBorder {
  /**
   * Border color (as opaque pointer - needs environment to resolve).
   */
  struct WuiColor *color;
  /**
   * Border width in points.
   */
  float width;
  /**
   * Corner radius in points (0 = square corners).
   */
  float corner_radius;
  /**
   * Which edges to draw the border on.
   */
  struct WuiEdgeSet edges;
} WuiBorder;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiBorder {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiBorder value;
} WuiMetadata_WuiBorder;

/**
 * Type alias for Metadata<Border> FFI struct
 */
typedef struct WuiMetadata_WuiBorder WuiMetadataBorder;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_f32 WuiComputed_f32;

/**
 * FFI-safe representation of an anchor point.
 * Normalized coordinates: (0.0, 0.0) = top-left, (0.5, 0.5) = center, (1.0, 1.0) = bottom-right.
 */
typedef struct WuiAnchor {
  /**
   * X coordinate (0.0 = left, 0.5 = center, 1.0 = right)
   */
  float x;
  /**
   * Y coordinate (0.0 = top, 0.5 = center, 1.0 = bottom)
   */
  float y;
} WuiAnchor;

/**
 * FFI-safe representation of a scale transform.
 * All values are reactive (Computed) and can be animated.
 */
typedef struct WuiScale {
  /**
   * Scale factor along X axis (1.0 = no scale)
   */
  WuiComputed_f32 *x;
  /**
   * Scale factor along Y axis (1.0 = no scale)
   */
  WuiComputed_f32 *y;
  /**
   * Anchor point for the scale transform
   */
  struct WuiAnchor anchor;
} WuiScale;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiScale {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiScale value;
} WuiMetadata_WuiScale;

/**
 * Type alias for Metadata<Scale> FFI struct
 */
typedef struct WuiMetadata_WuiScale WuiMetadataScale;

/**
 * FFI-safe representation of a rotation transform.
 * All values are reactive (Computed) and can be animated.
 */
typedef struct WuiRotation {
  /**
   * Rotation angle in degrees (positive = clockwise)
   */
  WuiComputed_f32 *angle;
  /**
   * Anchor point for the rotation transform
   */
  struct WuiAnchor anchor;
} WuiRotation;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiRotation {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiRotation value;
} WuiMetadata_WuiRotation;

/**
 * Type alias for Metadata<Rotation> FFI struct
 */
typedef struct WuiMetadata_WuiRotation WuiMetadataRotation;

/**
 * FFI-safe representation of an offset transform.
 * All values are reactive (Computed) and can be animated.
 */
typedef struct WuiOffset {
  /**
   * Offset along X axis in points
   */
  WuiComputed_f32 *x;
  /**
   * Offset along Y axis in points
   */
  WuiComputed_f32 *y;
} WuiOffset;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiOffset {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiOffset value;
} WuiMetadata_WuiOffset;

/**
 * Type alias for Metadata<Offset> FFI struct
 */
typedef struct WuiMetadata_WuiOffset WuiMetadataOffset;

/**
 * FFI-safe representation of compositor opacity metadata.
 * All values are reactive (`Computed`) and can be animated.
 */
typedef struct WuiOpacity {
  /**
   * Opacity value (0 = transparent, 1 = opaque).
   */
  WuiComputed_f32 *value;
} WuiOpacity;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiOpacity {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiOpacity value;
} WuiMetadata_WuiOpacity;

/**
 * Type alias for Metadata<Opacity> FFI struct
 */
typedef struct WuiMetadata_WuiOpacity WuiMetadataOpacity;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_bool WuiBinding_bool;

/**
 * FFI-safe representation of focused state.
 */
typedef struct WuiFocused {
  /**
   * Binding to the focus state (true = focused).
   */
  WuiBinding_bool *binding;
} WuiFocused;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiFocused {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiFocused value;
} WuiMetadata_WuiFocused;

/**
 * Type alias for Metadata<Focused> FFI struct
 */
typedef struct WuiMetadata_WuiFocused WuiMetadataFocused;

/**
 * FFI-safe representation of `IgnoreSafeArea`.
 */
typedef struct WuiIgnoreSafeArea {
  /**
   * Which edges should ignore safe area.
   */
  struct WuiEdgeSet edges;
} WuiIgnoreSafeArea;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiIgnoreSafeArea {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiIgnoreSafeArea value;
} WuiMetadata_WuiIgnoreSafeArea;

/**
 * Type alias for Metadata<IgnoreSafeArea> FFI struct
 */
typedef struct WuiMetadata_WuiIgnoreSafeArea WuiMetadataIgnoreSafeArea;

/**
 * FFI-safe representation of Retain metadata.
 * The actual retained value is opaque - renderers just need to keep it alive.
 */
typedef struct WuiRetain {
  /**
   * Opaque pointer to the retained value (Box<dyn Any>).
   * This must be kept alive and dropped when the view is disposed.
   */
  void *_opaque;
} WuiRetain;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiRetain {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiRetain value;
} WuiMetadata_WuiRetain;

/**
 * Type alias for Metadata<Retain> FFI struct
 */
typedef struct WuiMetadata_WuiRetain WuiMetadataRetain;

/**
 * FFI-safe representation of a path command.
 * All coordinates are normalized (0.0-1.0) and scale with view bounds.
 */
typedef enum WuiPathCommand_Tag {
  /**
   * Move to a position without drawing.
   */
  WuiPathCommand_MoveTo,
  /**
   * Draw a straight line to a position.
   */
  WuiPathCommand_LineTo,
  /**
   * Draw a quadratic bezier curve.
   */
  WuiPathCommand_QuadTo,
  /**
   * Draw a cubic bezier curve.
   */
  WuiPathCommand_CubicTo,
  /**
   * Draw an arc.
   */
  WuiPathCommand_Arc,
  /**
   * Close the current subpath.
   */
  WuiPathCommand_Close,
} WuiPathCommand_Tag;

typedef struct WuiPathCommand_MoveTo_Body {
  /**
   * Target X coordinate.
   */
  float x;
  /**
   * Target Y coordinate.
   */
  float y;
} WuiPathCommand_MoveTo_Body;

typedef struct WuiPathCommand_LineTo_Body {
  /**
   * Target X coordinate.
   */
  float x;
  /**
   * Target Y coordinate.
   */
  float y;
} WuiPathCommand_LineTo_Body;

typedef struct WuiPathCommand_QuadTo_Body {
  /**
   * Control point X coordinate.
   */
  float cx;
  /**
   * Control point Y coordinate.
   */
  float cy;
  /**
   * End point X coordinate.
   */
  float x;
  /**
   * End point Y coordinate.
   */
  float y;
} WuiPathCommand_QuadTo_Body;

typedef struct WuiPathCommand_CubicTo_Body {
  /**
   * First control point X coordinate.
   */
  float c1x;
  /**
   * First control point Y coordinate.
   */
  float c1y;
  /**
   * Second control point X coordinate.
   */
  float c2x;
  /**
   * Second control point Y coordinate.
   */
  float c2y;
  /**
   * End point X coordinate.
   */
  float x;
  /**
   * End point Y coordinate.
   */
  float y;
} WuiPathCommand_CubicTo_Body;

typedef struct WuiPathCommand_Arc_Body {
  /**
   * Center X coordinate.
   */
  float cx;
  /**
   * Center Y coordinate.
   */
  float cy;
  /**
   * Radius along the X axis.
   */
  float rx;
  /**
   * Radius along the Y axis.
   */
  float ry;
  /**
   * Start angle in radians.
   */
  float start;
  /**
   * Sweep angle in radians.
   */
  float sweep;
} WuiPathCommand_Arc_Body;

typedef struct WuiPathCommand {
  WuiPathCommand_Tag tag;
  union {
    WuiPathCommand_MoveTo_Body move_to;
    WuiPathCommand_LineTo_Body line_to;
    WuiPathCommand_QuadTo_Body quad_to;
    WuiPathCommand_CubicTo_Body cubic_to;
    WuiPathCommand_Arc_Body arc;
  };
} WuiPathCommand;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiPathCommand {
  struct WuiPathCommand *head;
  uintptr_t len;
} WuiArraySlice_WuiPathCommand;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiPathCommand {
  void (*drop)(void*);
  struct WuiArraySlice_WuiPathCommand (*slice)(const void*);
} WuiArrayVTable_WuiPathCommand;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiPathCommand {
  NonNull data;
  struct WuiArrayVTable_WuiPathCommand vtable;
} WuiArray_WuiPathCommand;

/**
 * FFI-safe representation of a clip shape.
 * Contains the path commands that define the clipping mask.
 */
typedef struct WuiClipShape {
  /**
   * Array of path commands defining the shape.
   */
  struct WuiArray_WuiPathCommand commands;
} WuiClipShape;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiClipShape {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiClipShape value;
} WuiMetadata_WuiClipShape;

/**
 * Type alias for Metadata<ClipShape> FFI struct
 */
typedef struct WuiMetadata_WuiClipShape WuiMetadataClipShape;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_HorizontalAlignment WuiComputed_HorizontalAlignment;

/**
 *C ABI mirror of `TextConfig`.
 */
typedef struct WuiText {
  /**
   *Mirrors the `content` field of `TextConfig`.
   */
  WuiComputed_StyledStr *content;
  /**
   *Mirrors the `paragraph_alignment` field of `TextConfig`.
   */
  WuiComputed_HorizontalAlignment *paragraph_alignment;
} WuiText;

/**
 * FFI representation of the `SystemIcon` component.
 *
 * Native backends render this as platform-native icons when supported.
 *
 * Apple currently maps this to SF Symbols. Other backends may omit `SystemIcon` support and should use
 * cross-platform icon-pack views instead.
 */
typedef struct WuiSystemIcon {
  /**
   * The name of the system icon.
   */
  struct WuiStr name;
} WuiSystemIcon;

/**
 * FFI-safe shortcut modifier flags.
 */
typedef struct WuiShortcutModifiers {
  /**
   * Command modifier on Apple platforms.
   */
  bool command;
  /**
   * Shift modifier.
   */
  bool shift;
  /**
   * Option/alt modifier.
   */
  bool option;
  /**
   * Control modifier.
   */
  bool control;
} WuiShortcutModifiers;

/**
 * FFI-safe keyboard shortcut payload.
 */
typedef struct WuiShortcut {
  /**
   * The key equivalent.
   */
  struct WuiStr key;
  /**
   * The shortcut modifiers.
   */
  struct WuiShortcutModifiers modifiers;
} WuiShortcut;

/**
 * FFI-safe representation of a menu item.
 */
typedef struct WuiMenuItem {
  /**
   * The menu node kind.
   */
  enum WuiMenuItemTag tag;
  /**
   * The resolved label for commands and nested menus.
   */
  struct WuiText *label;
  /**
   * Optional icon shown alongside the label.
   */
  struct WuiSystemIcon *icon;
  /**
   * The action handler pointer for commands.
   */
  struct WuiSharedAction *action;
  /**
   * Reactive disabled state for commands.
   */
  WuiComputed_bool *disabled;
  /**
   * Reactive selected/checkmark state for commands.
   */
  WuiComputed_bool *selected;
  /**
   * Optional keyboard shortcut metadata for commands.
   */
  struct WuiShortcut *shortcut;
  /**
   * Nested menu items.
   */
  struct WuiAnyViews *items;
} WuiMenuItem;

/**
 * FFI-safe representation of a context menu.
 */
typedef struct WuiContextMenu {
  /**
   * Identity-aware reactive menu items.
   */
  struct WuiAnyViews *items;
} WuiContextMenu;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiContextMenu {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiContextMenu value;
} WuiMetadata_WuiContextMenu;

/**
 * Type alias for Metadata<ContextMenu> FFI struct
 */
typedef struct WuiMetadata_WuiContextMenu WuiMetadataContextMenu;

/**
 * FFI-safe representation of a Menu component.
 */
typedef struct WuiMenu {
  /**
   * The label view displayed on the menu button.
   */
  struct WuiAnyView *label;
  /**
   * Identity-aware reactive menu items.
   */
  struct WuiAnyViews *items;
  /**
   * Semantic accessibility label for the menu trigger.
   */
  WuiComputed_StyledStr *accessibility_label;
} WuiMenu;

/**
 * FFI-safe representation of a draggable metadata.
 */
typedef struct WuiDraggable {
  /**
   * Opaque pointer to the Draggable wrapper.
   */
  struct WuiDraggableWrapper *inner;
} WuiDraggable;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiDraggable {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiDraggable value;
} WuiMetadata_WuiDraggable;

/**
 * Type alias for Metadata<Draggable> FFI struct
 */
typedef struct WuiMetadata_WuiDraggable WuiMetadataDraggable;

/**
 * FFI-safe representation of a drop destination metadata.
 */
typedef struct WuiDropDestination {
  /**
   * Opaque pointer to the drop handler.
   */
  struct WuiDropHandler *handler;
} WuiDropDestination;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiDropDestination {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiDropDestination value;
} WuiMetadata_WuiDropDestination;

/**
 * Type alias for Metadata<DropDestination> FFI struct
 */
typedef struct WuiMetadata_WuiDropDestination WuiMetadataDropDestination;

/**
 * FFI-safe representation of `IgnorableMetadata`<MaterialBackground>
 */
typedef struct WuiIgnorableMetadataMaterialBackground {
  /**
   * The view content wrapped by this metadata
   */
  struct WuiAnyView *content;
  /**
   * The material type for the blur effect
   */
  enum WuiMaterial material;
} WuiIgnorableMetadataMaterialBackground;

/**
 * FFI-safe representation of Hittable metadata.
 */
typedef struct WuiHittable {
  /**
   * Whether hit testing is enabled (reactive).
   */
  WuiComputed_bool *enabled;
} WuiHittable;

/**
 * Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
 * attached metadata value.
 */
typedef struct WuiMetadata_WuiHittable {
  /**
   * The view content wrapped by this metadata node.
   */
  struct WuiAnyView *content;
  /**
   * The metadata value attached to `content`.
   */
  struct WuiHittable value;
} WuiMetadata_WuiHittable;

/**
 * Type alias for Metadata<Hittable> FFI struct
 */
typedef struct WuiMetadata_WuiHittable WuiMetadataHittable;

/**
 *C ABI mirror of `ResolvedColor`.
 */
typedef struct WuiResolvedColor {
  /**
   *Mirrors the `red` field of `ResolvedColor`.
   */
  float red;
  /**
   *Mirrors the `green` field of `ResolvedColor`.
   */
  float green;
  /**
   *Mirrors the `blue` field of `ResolvedColor`.
   */
  float blue;
  /**
   *Mirrors the `opacity` field of `ResolvedColor`.
   */
  float opacity;
  /**
   *Mirrors the `headroom` field of `ResolvedColor`.
   */
  float headroom;
} WuiResolvedColor;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_ResolvedColor WuiComputed_ResolvedColor;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_Color WuiBinding_Color;

/**
 *C ABI mirror of `ResolvedGradientStop`.
 */
typedef struct WuiResolvedGradientStop {
  /**
   *Mirrors the `position` field of `ResolvedGradientStop`.
   */
  float position;
  /**
   *Mirrors the `color` field of `ResolvedGradientStop`.
   */
  struct WuiResolvedColor color;
} WuiResolvedGradientStop;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiResolvedGradientStop {
  struct WuiResolvedGradientStop *head;
  uintptr_t len;
} WuiArraySlice_WuiResolvedGradientStop;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiResolvedGradientStop {
  void (*drop)(void*);
  struct WuiArraySlice_WuiResolvedGradientStop (*slice)(const void*);
} WuiArrayVTable_WuiResolvedGradientStop;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiResolvedGradientStop {
  NonNull data;
  struct WuiArrayVTable_WuiResolvedGradientStop vtable;
} WuiArray_WuiResolvedGradientStop;

/**
 * C ABI mirror of [`ResolvedGradient`], the backend-native gradient payload
 * produced once a gradient's dynamic inputs have been resolved.
 */
typedef struct WuiResolvedGradient {
  /**
   * Gradient kind (linear, radial, angular, or mesh).
   */
  enum WuiGradientType gradient_type;
  /**
   * The gradient's resolved color stops.
   */
  struct WuiArray_WuiResolvedGradientStop stops;
  /**
   * Start point (linear) or center (radial/angular) x-coordinate.
   */
  float start_x;
  /**
   * Start point (linear) or center (radial/angular) y-coordinate.
   */
  float start_y;
  /**
   * End point (linear) x-coordinate.
   */
  float end_x;
  /**
   * End point (linear) y-coordinate.
   */
  float end_y;
  /**
   * Start radius (radial) or start angle (angular).
   */
  float start_value;
  /**
   * End radius (radial) or end angle (angular).
   */
  float end_value;
} WuiResolvedGradient;

/**
 * C ABI mirror of [`ShapeKind`], flattened into a discriminant tag plus the
 * per-corner radii used only by the rounded-rect variants.
 */
typedef struct WuiShapeKind {
  /**
   * Discriminant: 0 = rect, 1 = circle, 2 = ellipse, 3 = rounded rect
   * (uniform radius), 4 = uneven rounded rect (per-corner radii),
   * 5 = capsule, 6 = custom path.
   */
  int32_t tag;
  /**
   * Top-left corner radius, used by tags 3 and 4.
   */
  float top_left;
  /**
   * Top-right corner radius, used by tags 3 and 4.
   */
  float top_right;
  /**
   * Bottom-right corner radius, used by tags 3 and 4.
   */
  float bottom_right;
  /**
   * Bottom-left corner radius, used by tags 3 and 4.
   */
  float bottom_left;
} WuiShapeKind;

/**
 * C ABI mirror of [`ResolvedShape`], the backend-native shape payload
 * rendered directly by native backends.
 */
typedef struct WuiResolvedShape {
  /**
   * Shape kind for backend-side rendering optimization.
   */
  struct WuiShapeKind kind;
  /**
   * Path commands in unit coordinate space.
   */
  struct WuiArray_WuiPathCommand commands;
  /**
   * Read-only signal for the environment-resolved fill color.
   */
  WuiComputed_ResolvedColor *fill;
} WuiResolvedShape;

/**
 * FFI-safe representation of an animation.
 *
 * cbindgen generates a tagged union with:
 * - `WuiAnimation_Tag` enum for variant discrimination
 * - Body structs for each variant with data
 * - `WuiAnimation` struct with tag field and anonymous union
 */
typedef enum WuiAnimation_Tag {
  /**
   * No animation - changes apply immediately
   */
  WuiAnimation_None,
  /**
   * Timed cubic bezier animation with control points
   *
   * Native backends can use these control points with:
   * - Apple: `CAMediaTimingFunction(controlPoints:)`
   * - Android: `PathInterpolator(x1, y1, x2, y2)`
   */
  WuiAnimation_Bezier,
  /**
   * Spring animation with physics-based movement
   */
  WuiAnimation_Spring,
} WuiAnimation_Tag;

typedef struct WuiAnimation_Bezier_Body {
  /**
   * Duration in milliseconds
   */
  uint64_t duration_ms;
  /**
   * First control point X (0.0 to 1.0)
   */
  float x1;
  /**
   * First control point Y
   */
  float y1;
  /**
   * Second control point X (0.0 to 1.0)
   */
  float x2;
  /**
   * Second control point Y
   */
  float y2;
} WuiAnimation_Bezier_Body;

typedef struct WuiAnimation_Spring_Body {
  /**
   * Stiffness of the spring (higher = faster)
   */
  float stiffness;
  /**
   * Damping factor (higher = less bounce)
   */
  float damping;
} WuiAnimation_Spring_Body;

typedef struct WuiAnimation {
  WuiAnimation_Tag tag;
  union {
    WuiAnimation_Bezier_Body bezier;
    WuiAnimation_Spring_Body spring;
  };
} WuiAnimation;

/**
 * FFI-safe representation of drag data.
 */
typedef struct WuiDragData {
  /**
   * The type of data.
   */
  enum WuiDragDataTag tag;
  /**
   * The content (text or URL string).
   */
  struct WuiStr value;
} WuiDragData;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_Str WuiBinding_Str;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_Str WuiComputed_Str;

/**
 *C ABI mirror of `Style`.
 */
typedef struct WuiTextStyle {
  /**
   *Mirrors the `font` field of `Style`.
   */
  struct WuiFont *font;
  /**
   *Mirrors the `italic` field of `Style`.
   */
  bool italic;
  /**
   *Mirrors the `underline` field of `Style`.
   */
  bool underline;
  /**
   *Mirrors the `strikethrough` field of `Style`.
   */
  bool strikethrough;
  /**
   *Mirrors the `foreground` field of `Style`.
   */
  struct WuiColor *foreground;
  /**
   *Mirrors the `background` field of `Style`.
   */
  struct WuiColor *background;
} WuiTextStyle;

/**
 * FFI representation of a single run of text sharing one `WuiTextStyle`.
 */
typedef struct WuiStyledChunk {
  /**
   * The text content of this run.
   */
  struct WuiStr text;
  /**
   * The style applied to this run.
   */
  struct WuiTextStyle style;
} WuiStyledChunk;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiStyledChunk {
  struct WuiStyledChunk *head;
  uintptr_t len;
} WuiArraySlice_WuiStyledChunk;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiStyledChunk {
  void (*drop)(void*);
  struct WuiArraySlice_WuiStyledChunk (*slice)(const void*);
} WuiArrayVTable_WuiStyledChunk;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiStyledChunk {
  NonNull data;
  struct WuiArrayVTable_WuiStyledChunk vtable;
} WuiArray_WuiStyledChunk;

/**
 * FFI representation of a string composed of independently styled runs.
 */
typedef struct WuiStyledStr {
  /**
   * The styled runs that make up the string, in order.
   */
  struct WuiArray_WuiStyledChunk chunks;
} WuiStyledStr;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_StyledStr WuiBinding_StyledStr;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_f64 WuiComputed_f64;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_i32 WuiBinding_i32;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_f32 WuiBinding_f32;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_f64 WuiBinding_f64;

/**
 * C-compatible date-time representation with second precision.
 */
typedef struct WuiDateTime {
  /**
   * Year (e.g., 2024)
   */
  int32_t year;
  /**
   * Month (1-12)
   */
  uint8_t month;
  /**
   * Day of month (1-31)
   */
  uint8_t day;
  /**
   * Hour of day (0-23)
   */
  uint8_t hour;
  /**
   * Minute of hour (0-59)
   */
  uint8_t minute;
  /**
   * Second of minute (0-59)
   */
  uint8_t second;
} WuiDateTime;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_DateTime WuiBinding_DateTime;

/**
 * C-compatible date representation using year, month (1-12), and day (1-31).
 */
typedef struct WuiDate {
  /**
   * Year (e.g., 2024)
   */
  int32_t year;
  /**
   * Month (1-12)
   */
  uint8_t month;
  /**
   * Day of month (1-31)
   */
  uint8_t day;
} WuiDate;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiDate {
  struct WuiDate *head;
  uintptr_t len;
} WuiArraySlice_WuiDate;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiDate {
  void (*drop)(void*);
  struct WuiArraySlice_WuiDate (*slice)(const void*);
} WuiArrayVTable_WuiDate;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiDate {
  NonNull data;
  struct WuiArrayVTable_WuiDate vtable;
} WuiArray_WuiDate;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_Vec_Date WuiBinding_Vec_Date;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_Vec_Date WuiComputed_Vec_Date;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_Secure WuiBinding_Secure;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_Id WuiBinding_Id;

/**
 * One node of a backend's accessibility tree, as it crosses the boundary.
 *
 * Strings are borrowed for the duration of the call: the runtime copies what
 * it keeps, so a backend can build these from whatever it already has without
 * handing over ownership.
 */
typedef struct WuiInspectorNode {
  /**
   * Identifier, stable for as long as the node exists.
   */
  uint64_t id;
  /**
   * Role, lowercase, as the platform names it.
   */
  struct WuiStr role;
  /**
   * Accessibility label; empty when the node has none.
   */
  struct WuiStr label;
  /**
   * Current value; empty when the node has none.
   */
  struct WuiStr value;
  /**
   * Whether `bounds` holds a rectangle.
   */
  bool has_bounds;
  /**
   * Layout rectangle in window coordinates, when `has_bounds`.
   */
  float bounds[4];
  /**
   * Whether the node accepts interaction.
   */
  bool enabled;
  /**
   * Whether the node is hidden from assistive technology.
   */
  bool hidden;
  /**
   * Whether the node is selected.
   */
  bool selected;
  /**
   * Whether `checked` means anything for this node.
   */
  bool has_checked;
  /**
   * Toggle state, when `has_checked`.
   */
  bool checked;
  /**
   * Children, in order.
   */
  const uint64_t *children;
  /**
   * Number of children.
   */
  uintptr_t children_len;
} WuiInspectorNode;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_ColorScheme WuiComputed_ColorScheme;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_ResolvedFont WuiComputed_ResolvedFont;

/**
 * FFI representation of a resolved font.
 */
typedef struct WuiResolvedFont {
  /**
   * Font size in points.
   */
  float size;
  /**
   * Font weight.
   */
  enum WuiFontWeight weight;
  /**
   * Font family name (empty string means system default).
   */
  struct WuiStr family;
} WuiResolvedFont;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiId {
  struct WuiId *head;
  uintptr_t len;
} WuiArraySlice_WuiId;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiId {
  void (*drop)(void*);
  struct WuiArraySlice_WuiId (*slice)(const void*);
} WuiArrayVTable_WuiId;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiId {
  NonNull data;
  struct WuiArrayVTable_WuiId vtable;
} WuiArray_WuiId;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_WindowState WuiBinding_WindowState;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_Rect WuiBinding_Rect;

/**
 * FFI-compatible representation of [`WindowBackground`].
 *
 * Only supports Opaque and Color. Material blur effects are handled
 * via `MaterialBackground` metadata on the window content.
 */
typedef enum WuiWindowBackground_Tag {
  /**
   * Opaque system default background.
   */
  WuiWindowBackground_Opaque,
  /**
   * Solid color background (can be semi-transparent via alpha).
   * Native must resolve the color using the environment.
   */
  WuiWindowBackground_Color,
} WuiWindowBackground_Tag;

typedef struct WuiWindowBackground_Color_Body {
  /**
   * Pointer to the reactive color to resolve and apply as the window background.
   */
  struct WuiColor *color;
} WuiWindowBackground_Color_Body;

typedef struct WuiWindowBackground {
  WuiWindowBackground_Tag tag;
  union {
    WuiWindowBackground_Color_Body color;
  };
} WuiWindowBackground;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_Size WuiComputed_Size;

/**
 * FFI-compatible representation of a window.
 */
typedef struct WuiWindow {
  /**
   * The title of the window.
   */
  WuiComputed_Str *title;
  /**
   * Whether the window is closable.
   */
  bool closable;
  /**
   * Whether the window is resizable.
   */
  bool resizable;
  /**
   * The frame of the window.
   */
  WuiBinding_Rect *frame;
  /**
   * The content of the window.
   */
  struct WuiAnyView *content;
  /**
   * The current state of the window.
   */
  WuiBinding_WindowState *state;
  /**
   * Optional toolbar content (null if none).
   */
  struct WuiAnyView *toolbar;
  /**
   * The visual style of the window.
   */
  enum WuiWindowStyle style;
  /**
   * The background style of the window.
   */
  struct WuiWindowBackground background;
  /**
   * Explicit minimum content size, or null to derive the minimum from the
   * content's layout (the root view measured at a zero proposal).
   */
  WuiComputed_Size *min_size;
  /**
   * Explicit maximum content size, or null for an unconstrained window.
   */
  WuiComputed_Size *max_size;
} WuiWindow;

/**
 * Type alias for the native window show function.
 *
 * This function is called by Rust when a `Window` view needs to be shown.
 * The native implementation should create and display the window.
 * # Parameters
 * - context: The native window-manager owner
 * - `WuiWindow`: The window configuration to show
 */
typedef void (*WindowShowFn)(void *context, struct WuiWindow window);

/**
 * FFI surface for the label slot of every control.
 *
 * Bundles the three pieces backends need: the visual view to render, the
 * accessibility text to announce regardless of visual mode, and the visual
 * mode itself. The `view` field is always non-null but renders to an empty
 * view when `display_mode` is [`WuiLabelDisplayMode::Hidden`].
 */
typedef struct WuiLabel {
  /**
   * Visual chrome rendered by the backend. When `display_mode` is
   * `Hidden` the rendered view collapses to an empty zero-size view.
   */
  struct WuiAnyView *view;
  /**
   * Spoken accessibility text. Always non-null; backends should bind
   * this to platform accessibility APIs (`VoiceOver`, `TalkBack`, etc.).
   */
  WuiComputed_StyledStr *accessibility_label;
  /**
   * Visual presentation mode.
   */
  enum WuiLabelDisplayMode display_mode;
} WuiLabel;

/**
 * FFI representation of the `Button` component.
 */
typedef struct WuiButton {
  /**
   * Semantic label slot. Carries the visual view, accessibility text, and
   * visual mode in a single struct — see [`WuiLabel`].
   */
  struct WuiLabel label;
  /**
   * The action invoked when the button is activated.
   */
  struct WuiAction *action;
  /**
   * The visual presentation style for the button.
   */
  enum WuiButtonStyle style;
} WuiButton;

/**
 * FFI representation of the `TextField` component.
 */
typedef struct WuiTextField {
  /**
   * Semantic label slot for the field — see [`WuiLabel`].
   */
  struct WuiLabel label;
  /**
   * Reactive binding to the field's styled text content.
   */
  WuiBinding_StyledStr *value;
  /**
   * Placeholder text shown when the field is empty.
   */
  struct WuiText prompt;
  /**
   * The on-screen keyboard variant to present while editing.
   */
  enum WuiKeyboardType keyboard;
  /**
   * Context menu items offered when the user selects text in the field.
   */
  struct WuiAnyViews *selection_menu;
  /**
   * Maximum number of lines the field accepts.
   *
   * `1` is a single-line field; a larger value caps a multi-line field; `0`
   * means the field has no line limit. Backends must reject input that would
   * exceed the limit rather than truncating the existing value.
   */
  uintptr_t line_limit;
} WuiTextField;

/**
 *C ABI mirror of `ToggleConfig`.
 */
typedef struct WuiToggle {
  /**
   *Mirrors the `label` field of `ToggleConfig`.
   */
  struct WuiLabel label;
  /**
   *Mirrors the `toggle` field of `ToggleConfig`.
   */
  WuiBinding_bool *toggle;
  /**
   *Mirrors the `style` field of `ToggleConfig`.
   */
  enum WuiToggleStyle style;
} WuiToggle;

/**
 * C representation of a range
 */
typedef struct WuiRange_f64 {
  /**
   * Start of the range
   */
  double start;
  /**
   * End of the range
   */
  double end;
} WuiRange_f64;

/**
 *C ABI mirror of `SliderConfig`.
 */
typedef struct WuiSlider {
  /**
   *Mirrors the `label` field of `SliderConfig`.
   */
  struct WuiLabel label;
  /**
   *Mirrors the `min_value_label` field of `SliderConfig`.
   */
  struct WuiAnyView *min_value_label;
  /**
   *Mirrors the `max_value_label` field of `SliderConfig`.
   */
  struct WuiAnyView *max_value_label;
  /**
   *Mirrors the `range` field of `SliderConfig`.
   */
  struct WuiRange_f64 range;
  /**
   *Mirrors the `value` field of `SliderConfig`.
   */
  WuiBinding_f64 *value;
} WuiSlider;

/**
 * C representation of a range
 */
typedef struct WuiRange_i32 {
  /**
   * Start of the range
   */
  int32_t start;
  /**
   * End of the range
   */
  int32_t end;
} WuiRange_i32;

/**
 *C ABI mirror of `StepperConfig`.
 */
typedef struct WuiStepper {
  /**
   *Mirrors the `value` field of `StepperConfig`.
   */
  WuiBinding_i32 *value;
  /**
   *Mirrors the `step` field of `StepperConfig`.
   */
  WuiComputed_i32 *step;
  /**
   *Mirrors the `label` field of `StepperConfig`.
   */
  struct WuiLabel label;
  /**
   *Mirrors the `value_formatter` field of `StepperConfig`.
   */
  WuiComputed_StyledStr *value_formatter;
  /**
   *Mirrors the `range` field of `StepperConfig`.
   */
  struct WuiRange_i32 range;
} WuiStepper;

/**
 *C ABI mirror of `ColorPickerConfig`.
 */
typedef struct WuiColorPicker {
  /**
   *Mirrors the `label` field of `ColorPickerConfig`.
   */
  struct WuiLabel label;
  /**
   *Mirrors the `value` field of `ColorPickerConfig`.
   */
  WuiBinding_Color *value;
  /**
   *Mirrors the `support_alpha` field of `ColorPickerConfig`.
   */
  bool support_alpha;
  /**
   *Mirrors the `support_hdr` field of `ColorPickerConfig`.
   */
  bool support_hdr;
} WuiColorPicker;

/**
 * FFI representation of the `Picker` component.
 */
typedef struct WuiPicker {
  /**
   * The picker's items, rendered as an identity-tracked view collection.
   */
  struct WuiAnyViews *items;
  /**
   * Reactive binding to the tag of the currently selected item.
   */
  WuiBinding_Id *selection;
  /**
   * The visual presentation style for the picker.
   */
  enum WuiPickerStyle style;
} WuiPicker;

/**
 *C ABI mirror of `SecureFieldConfig`.
 */
typedef struct WuiSecureField {
  /**
   *Mirrors the `label` field of `SecureFieldConfig`.
   */
  struct WuiLabel label;
  /**
   *Mirrors the `value` field of `SecureFieldConfig`.
   */
  WuiBinding_Secure *value;
} WuiSecureField;

/**
 * FFI representation of a single resolved picker item.
 */
typedef struct WuiPickerItem {
  /**
   * The stable identity tag used to track selection for this item.
   */
  struct WuiId tag;
  /**
   * Reactive computed label text displayed for this item.
   */
  WuiComputed_StyledStr *label;
} WuiPickerItem;

/**
 * C representation of a range
 */
typedef struct WuiRange_WuiDateTime {
  /**
   * Start of the range
   */
  struct WuiDateTime start;
  /**
   * End of the range
   */
  struct WuiDateTime end;
} WuiRange_WuiDateTime;

/**
 *C ABI mirror of `DatePickerConfig`.
 */
typedef struct WuiDatePicker {
  /**
   *Mirrors the `label` field of `DatePickerConfig`.
   */
  struct WuiLabel label;
  /**
   *Mirrors the `value` field of `DatePickerConfig`.
   */
  WuiBinding_DateTime *value;
  /**
   *Mirrors the `range` field of `DatePickerConfig`.
   */
  struct WuiRange_WuiDateTime range;
  /**
   *Mirrors the `ty` field of `DatePickerConfig`.
   */
  enum WuiDatePickerType ty;
} WuiDatePicker;

/**
 * C representation of a range
 */
typedef struct WuiRange_WuiDate {
  /**
   * Start of the range
   */
  struct WuiDate start;
  /**
   * End of the range
   */
  struct WuiDate end;
} WuiRange_WuiDate;

/**
 *C ABI mirror of `MultiDatePickerConfig`.
 */
typedef struct WuiMultiDatePicker {
  /**
   *Mirrors the `label` field of `MultiDatePickerConfig`.
   */
  struct WuiLabel label;
  /**
   *Mirrors the `value` field of `MultiDatePickerConfig`.
   */
  WuiBinding_Vec_Date *value;
  /**
   *Mirrors the `range` field of `MultiDatePickerConfig`.
   */
  struct WuiRange_WuiDate range;
  /**
   *Mirrors the `decorated` field of `MultiDatePickerConfig`.
   */
  WuiComputed_Vec_Date *decorated;
} WuiMultiDatePicker;

/**
 *C ABI mirror of `ProgressConfig`.
 */
typedef struct WuiProgress {
  /**
   *Mirrors the `label` field of `ProgressConfig`.
   */
  struct WuiAnyView *label;
  /**
   *Mirrors the `value_label` field of `ProgressConfig`.
   */
  struct WuiAnyView *value_label;
  /**
   *Mirrors the `value` field of `ProgressConfig`.
   */
  WuiComputed_f64 *value;
  /**
   *Mirrors the `style` field of `ProgressConfig`.
   */
  enum WuiProgressStyle style;
  /**
   *Mirrors the `four_color` field of `ProgressConfig`.
   */
  bool four_color;
} WuiProgress;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_Region WuiComputed_Region;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_Vec_Annotation WuiComputed_Vec_Annotation;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_Option_Location WuiComputed_Option_Location;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_MapStatus WuiBinding_MapStatus;

/**
 * FFI representation of the Map component.
 */
typedef struct WuiMap {
  /**
   * The region to display (reactive).
   */
  WuiComputed_Region *region;
  /**
   * Annotations to display (reactive).
   */
  WuiComputed_Vec_Annotation *annotations;
  /**
   * The map display style.
   */
  enum WuiMapStyle style;
  /**
   * Whether to show the user's current location.
   */
  bool shows_user_location;
  /**
   * Reactive `WaterKit` location supplied by the application.
   *
   * Present whenever the application drove the location itself; a backend
   * must prefer this over the platform's own location service so both agree
   * on one source.
   * Null when the application did not supply one, in which case a native
   * backend may fall back to the platform's own location service.
   */
  WuiComputed_Option_Location *user_location;
  /**
   * Sink the backend reports its load lifecycle into, or null when the
   * application is not observing status.
   */
  WuiBinding_MapStatus *status;
  /**
   * Whether the map is interactive (pan/zoom enabled).
   */
  bool is_interactive;
  /**
   * Whether to show the compass.
   */
  bool shows_compass;
  /**
   * Whether to show the scale.
   */
  bool shows_scale;
} WuiMap;

/**
 * FFI representation of a geographic coordinate.
 */
typedef struct WuiCoordinate {
  /**
   * Latitude in degrees (-90 to 90).
   */
  double latitude;
  /**
   * Longitude in degrees (-180 to 180).
   */
  double longitude;
} WuiCoordinate;

/**
 * FFI representation of a map region.
 */
typedef struct WuiRegion {
  /**
   * The center coordinate of the region.
   */
  struct WuiCoordinate center;
  /**
   * The north-to-south span in degrees.
   */
  double latitude_delta;
  /**
   * The east-to-west span in degrees.
   */
  double longitude_delta;
} WuiRegion;

/**
 * FFI representation of a map annotation (pin).
 */
typedef struct WuiAnnotation {
  /**
   * The coordinate where the annotation is placed.
   */
  struct WuiCoordinate coordinate;
  /**
   * The title text.
   */
  struct WuiStr title;
  /**
   * The subtitle text (empty string if none).
   */
  struct WuiStr subtitle;
} WuiAnnotation;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiAnnotation {
  struct WuiAnnotation *head;
  uintptr_t len;
} WuiArraySlice_WuiAnnotation;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiAnnotation {
  void (*drop)(void*);
  struct WuiArraySlice_WuiAnnotation (*slice)(const void*);
} WuiArrayVTable_WuiAnnotation;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiAnnotation {
  NonNull data;
  struct WuiArrayVTable_WuiAnnotation vtable;
} WuiArray_WuiAnnotation;

/**
 * FFI representation of an optional device location reading.
 *
 * A `WaterKit` location carries optional altitude and accuracy, which C cannot
 * express as `Option`. Each optional reading is paired with a `has_*` flag
 * instead of a sentinel value, so a real zero altitude stays distinguishable
 * from "no altitude".
 */
typedef struct WuiLocation {
  /**
   * Whether a location is present at all.
   */
  bool has_location;
  /**
   * Where the device is.
   */
  struct WuiCoordinate coordinate;
  /**
   * Altitude in metres above sea level, when known.
   */
  double altitude;
  /**
   * Whether `altitude` holds a reading.
   */
  bool has_altitude;
  /**
   * Horizontal accuracy radius in metres, when known.
   */
  double horizontal_accuracy;
  /**
   * Whether `horizontal_accuracy` holds a reading.
   */
  bool has_horizontal_accuracy;
  /**
   * When the reading was taken, in milliseconds since the Unix epoch.
   */
  int64_t timestamp_ms;
} WuiLocation;

/**
 * FFI representation of the map's load lifecycle.
 */
typedef struct WuiMapStatus {
  /**
   * Which lifecycle state this is.
   */
  enum WuiMapStatusKind kind;
  /**
   * Failure reason; empty unless `kind` is `Failed`.
   */
  struct WuiStr reason;
} WuiMapStatus;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_____WuiAnyView {
  struct WuiAnyView **head;
  uintptr_t len;
} WuiArraySlice_____WuiAnyView;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_____WuiAnyView {
  void (*drop)(void*);
  struct WuiArraySlice_____WuiAnyView (*slice)(const void*);
} WuiArrayVTable_____WuiAnyView;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_____WuiAnyView {
  NonNull data;
  struct WuiArrayVTable_____WuiAnyView vtable;
} WuiArray_____WuiAnyView;

/**
 * C ABI mirror of [`FixedContainer`], a view that executes an arbitrary
 * [`Layout`] over an eagerly-collected, fixed set of child views.
 */
typedef struct WuiFixedContainer {
  /**
   * Owning handle to the boxed [`Layout`] implementation driving this container.
   */
  struct WuiLayout *layout;
  /**
   * The container's child views, collected eagerly at construction time.
   */
  struct WuiArray_____WuiAnyView contents;
} WuiFixedContainer;

/**
 * C ABI mirror of [`LazyContainer`], a view that executes an arbitrary
 * [`Layout`] over a lazily reconstructed collection of child views.
 */
typedef struct WuiContainer {
  /**
   * Owning handle to the boxed [`Layout`] implementation driving this container.
   */
  struct WuiLayout *layout;
  /**
   * Handle to the lazily reconstructed child view collection.
   */
  struct WuiAnyViews *contents;
} WuiContainer;

/**
 * Native callback invoked when a reactive layout input changes.
 */
typedef void (*WuiLayoutInvalidationCallback)(void *context);

/**
 *C ABI mirror of `Size`.
 */
typedef struct WuiSize {
  /**
   *Mirrors the `width` field of `Size`.
   */
  float width;
  /**
   *Mirrors the `height` field of `Size`.
   */
  float height;
} WuiSize;

/**
 *C ABI mirror of `Point`.
 */
typedef struct WuiPoint {
  /**
   *Mirrors the `x` field of `Point`.
   */
  float x;
  /**
   *Mirrors the `y` field of `Point`.
   */
  float y;
} WuiPoint;

/**
 * C ABI mirror of [`Rect`]: an axis-aligned rectangle expressed as an
 * origin point and a size, relative to its parent's coordinate space.
 */
typedef struct WuiRect {
  struct WuiPoint origin;
  struct WuiSize size;
} WuiRect;

/**
 * C ABI mirror of one explicit horizontal alignment guide entry from
 * [`ViewDimensions`], pairing a named alignment with its measured offset.
 */
typedef struct WuiHorizontalGuide {
  enum WuiHorizontalAlignment alignment;
  float value;
} WuiHorizontalGuide;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiHorizontalGuide {
  struct WuiHorizontalGuide *head;
  uintptr_t len;
} WuiArraySlice_WuiHorizontalGuide;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiHorizontalGuide {
  void (*drop)(void*);
  struct WuiArraySlice_WuiHorizontalGuide (*slice)(const void*);
} WuiArrayVTable_WuiHorizontalGuide;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiHorizontalGuide {
  NonNull data;
  struct WuiArrayVTable_WuiHorizontalGuide vtable;
} WuiArray_WuiHorizontalGuide;

/**
 * C ABI mirror of one explicit vertical alignment guide entry from
 * [`ViewDimensions`], pairing a named alignment with its measured offset.
 */
typedef struct WuiVerticalGuide {
  enum WuiVerticalAlignment alignment;
  float value;
} WuiVerticalGuide;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiVerticalGuide {
  struct WuiVerticalGuide *head;
  uintptr_t len;
} WuiArraySlice_WuiVerticalGuide;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiVerticalGuide {
  void (*drop)(void*);
  struct WuiArraySlice_WuiVerticalGuide (*slice)(const void*);
} WuiArrayVTable_WuiVerticalGuide;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiVerticalGuide {
  NonNull data;
  struct WuiArrayVTable_WuiVerticalGuide vtable;
} WuiArray_WuiVerticalGuide;

/**
 * C ABI mirror of [`ViewDimensions`]: a measured size together with any
 * explicit horizontal/vertical alignment guides the view exposes to its
 * parent layout.
 */
typedef struct WuiViewDimensions {
  struct WuiSize size;
  struct WuiArray_WuiHorizontalGuide horizontal_guides;
  struct WuiArray_WuiVerticalGuide vertical_guides;
} WuiViewDimensions;

/**
 * C ABI mirror of [`ProposalSize`]: the size a parent layout suggests to a
 * child. Either axis may be unspecified, encoded here as `f32::NAN` to mean
 * "the child decides its own size along this axis".
 */
typedef struct WuiProposalSize {
  float width;
  float height;
} WuiProposalSize;

/**
 * `VTable` for `SubView` operations.
 *
 * This structure contains function pointers that allow native code to implement
 * the `SubView` protocol. The native backend provides these callbacks to participate
 * in layout negotiation.
 */
typedef struct WuiSubViewVTable {
  /**
   * Measures the child view given a size proposal.
   * Called potentially multiple times with different proposals during layout.
   */
  struct WuiViewDimensions (*measure)(void *context, struct WuiProposalSize proposal);
  /**
   * Cleans up the context when the subview is no longer needed.
   * Called when the `WuiSubView` is dropped.
   */
  void (*drop)(void *context);
} WuiSubViewVTable;

/**
 * FFI representation of a `SubView` proxy.
 *
 * This allows native code to participate in the layout negotiation protocol
 * by providing callbacks that can be called multiple times with different proposals.
 *
 * # Memory Management
 *
 * The `context` pointer is owned by this struct. When the `WuiSubView` is dropped,
 * the `vtable.drop` function will be called to clean up the context.
 */
typedef struct WuiSubView {
  /**
   * Opaque context pointer (e.g., child view reference, cached data)
   */
  void *context;
  /**
   * `VTable` containing measure and drop functions
   */
  struct WuiSubViewVTable vtable;
  /**
   * Which axis this view stretches to fill available space
   */
  enum WuiStretchAxis stretch_axis;
  /**
   * Layout priority (higher = measured first, gets space preference)
   */
  int32_t priority;
} WuiSubView;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiSubView {
  struct WuiSubView *head;
  uintptr_t len;
} WuiArraySlice_WuiSubView;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiSubView {
  void (*drop)(void*);
  struct WuiArraySlice_WuiSubView (*slice)(const void*);
} WuiArrayVTable_WuiSubView;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiSubView {
  NonNull data;
  struct WuiArrayVTable_WuiSubView vtable;
} WuiArray_WuiSubView;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiRect {
  struct WuiRect *head;
  uintptr_t len;
} WuiArraySlice_WuiRect;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiRect {
  void (*drop)(void*);
  struct WuiArraySlice_WuiRect (*slice)(const void*);
} WuiArrayVTable_WuiRect;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiRect {
  NonNull data;
  struct WuiArrayVTable_WuiRect vtable;
} WuiArray_WuiRect;

/**
 * C ABI mirror of [`ScrollView`], a container that scrolls content larger
 * than its own frame along one or both axes.
 */
typedef struct WuiScrollView {
  /**
   * The axis (or axes) along which this scroll view allows scrolling.
   */
  enum WuiAxis axis;
  /**
   * The scrollable content view.
   */
  struct WuiAnyView *content;
  /**
   * Read-only signal for the horizontal scroll target requested via
   * `ScrollController::scroll_to`, or null if no controller is attached.
   */
  WuiComputed_f32 *target_x;
  /**
   * Read-only signal for the vertical scroll target requested via
   * `ScrollController::scroll_to`, or null if no controller is attached.
   */
  WuiComputed_f32 *target_y;
  /**
   * Monotonically increasing generation that changes each time a new
   * scroll request is issued, letting the backend detect a repeated
   * request to the same target. Null if no controller is attached.
   */
  WuiComputed_i32 *scroll_generation;
} WuiScrollView;

/**
 * FFI representation of a list item.
 *
 * `section_label` and `section_footer` are owned by the consumer — when
 * they're empty the item carries no section break, otherwise the item opens
 * a new logical section visible to the renderer (`UITableView` sections,
 * `NSTableView` group rows, Material list groups, ...). Both fields are
 * passed by value so ownership of the underlying byte buffers transfers
 * cleanly to the backend; no separate drop call is required.
 */
typedef struct WuiListItem {
  /**
   * The content view for this item.
   */
  struct WuiAnyView *content;
  /**
   * Read-only signal indicating whether this item can be deleted.
   */
  WuiComputed_bool *deletable;
  /**
   * Read-only signal marking this item as the current selection; the
   * backend draws its platform's selection chrome while it is true.
   */
  WuiComputed_bool *selected;
  /**
   * Section header carried by this item, or empty when the item does not
   * start a new section.
   */
  struct WuiStr section_label;
  /**
   * Section footer carried by this item, or empty when no footer is set.
   */
  struct WuiStr section_footer;
} WuiListItem;

/**
 * FFI representation of a list.
 */
typedef struct WuiList {
  /**
   * The list contents (array of list items).
   */
  struct WuiAnyViews *contents;
  /**
   * Read-only signal for edit mode state.
   */
  WuiComputed_bool *editing;
  /**
   * Optional delete callback (null if not deletable).
   */
  struct WuiIndexAction *on_delete;
  /**
   * Optional move callback (null if not reorderable).
   */
  struct WuiMoveAction *on_move;
  /**
   * Optional requested row index (null when uncontrolled).
   */
  WuiComputed_i32 *target_index;
  /**
   * Optional scroll request generation (null when uncontrolled).
   */
  WuiComputed_i32 *scroll_generation;
  /**
   * Whether rows carry semantic section markers.
   */
  bool uses_sections;
} WuiList;

/**
 * C ABI mirror of [`TableConfig`], a view arranging its columns in a table layout.
 */
typedef struct WuiTable {
  /**
   * Handle to the table's column collection.
   */
  struct WuiAnyViews *columns;
} WuiTable;

/**
 * C ABI mirror of [`TableColumn`], one labeled column of a table.
 */
typedef struct WuiTableColumn {
  /**
   * The column label as styled text.
   */
  struct WuiText label;
  /**
   * The row views for this column.
   */
  struct WuiAnyViews *rows;
} WuiTableColumn;

/**
 * FFI-owned wrapper around a [`waterui::Computed`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_computed_*`,
 * `waterui_watch_computed_*`, and `waterui_drop_computed_*` functions generated
 * by the `ffi_computed!` macro.
 */
typedef struct Computed_Delivery WuiComputed_Delivery;

/**
 * FFI representation of [`SubtitleSelection`].
 */
typedef struct WuiSubtitleSelection {
  /**
   * Which selection mode is active.
   */
  enum WuiSubtitleSelectionType selection_type;
  /**
   * Track index. Read only when `selection_type` is `Track`.
   */
  uintptr_t track_index;
} WuiSubtitleSelection;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_SubtitleSelection WuiBinding_SubtitleSelection;

/**
 * FFI representation of [`AudioTrackSelection`].
 */
typedef struct WuiAudioTrackSelection {
  /**
   * Which selection mode is active.
   */
  enum WuiAudioTrackSelectionType selection_type;
  /**
   * Track index. Read only when `selection_type` is `Track`.
   */
  uintptr_t track_index;
} WuiAudioTrackSelection;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_AudioTrackSelection WuiBinding_AudioTrackSelection;

/**
 * FFI representation of [`VideoTrackSelection`].
 */
typedef struct WuiVideoTrackSelection {
  /**
   * Which selection mode is active.
   */
  enum WuiVideoTrackSelectionType selection_type;
  /**
   * Track index. Read only when `selection_type` is `Track`.
   */
  uintptr_t track_index;
} WuiVideoTrackSelection;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_VideoTrackSelection WuiBinding_VideoTrackSelection;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_TrackCatalog WuiBinding_TrackCatalog;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiStr {
  struct WuiStr *head;
  uintptr_t len;
} WuiArraySlice_WuiStr;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiStr {
  void (*drop)(void*);
  struct WuiArraySlice_WuiStr (*slice)(const void*);
} WuiArrayVTable_WuiStr;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiStr {
  NonNull data;
  struct WuiArrayVTable_WuiStr vtable;
} WuiArray_WuiStr;

/**
 * Native representation of one selectable audio track.
 */
typedef struct WuiVideoAudioTrackInfo {
  /**
   * Human-readable track label.
   */
  struct WuiStr label;
  /**
   * The track's language, as an empty string when unknown.
   */
  struct WuiStr language;
  /**
   * Track role tags (e.g. "main", "commentary").
   */
  struct WuiArray_WuiStr roles;
} WuiVideoAudioTrackInfo;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiVideoAudioTrackInfo {
  struct WuiVideoAudioTrackInfo *head;
  uintptr_t len;
} WuiArraySlice_WuiVideoAudioTrackInfo;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiVideoAudioTrackInfo {
  void (*drop)(void*);
  struct WuiArraySlice_WuiVideoAudioTrackInfo (*slice)(const void*);
} WuiArrayVTable_WuiVideoAudioTrackInfo;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiVideoAudioTrackInfo {
  NonNull data;
  struct WuiArrayVTable_WuiVideoAudioTrackInfo vtable;
} WuiArray_WuiVideoAudioTrackInfo;

/**
 * Native representation of one selectable video representation.
 */
typedef struct WuiVideoTrackInfo {
  /**
   * Stable identifier for the track.
   */
  struct WuiStr id;
  /**
   * Human-readable track label.
   */
  struct WuiStr label;
  /**
   * Bitrate in bits per second; `0` means unknown.
   */
  uint64_t bandwidth;
  /**
   * Frame width in pixels; `0` together with `height == 0` means unknown.
   */
  uint32_t width;
  /**
   * Frame height in pixels; `0` together with `width == 0` means unknown.
   */
  uint32_t height;
  /**
   * Codec identifiers for this representation (e.g. "hvc1").
   */
  struct WuiArray_WuiStr codecs;
  /**
   * Whether this representation is HDR.
   */
  bool hdr;
} WuiVideoTrackInfo;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiVideoTrackInfo {
  struct WuiVideoTrackInfo *head;
  uintptr_t len;
} WuiArraySlice_WuiVideoTrackInfo;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiVideoTrackInfo {
  void (*drop)(void*);
  struct WuiArraySlice_WuiVideoTrackInfo (*slice)(const void*);
} WuiArrayVTable_WuiVideoTrackInfo;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiVideoTrackInfo {
  NonNull data;
  struct WuiArrayVTable_WuiVideoTrackInfo vtable;
} WuiArray_WuiVideoTrackInfo;

/**
 * Native representation of one selectable subtitle track.
 */
typedef struct WuiVideoSubtitleTrackInfo {
  /**
   * Human-readable track label.
   */
  struct WuiStr label;
  /**
   * The track's language, as an empty string when unknown.
   */
  struct WuiStr language;
  /**
   * Track role tags (e.g. "caption", "commentary").
   */
  struct WuiArray_WuiStr roles;
  /**
   * Whether this track is a forced (director-mandated) subtitle track.
   */
  bool forced;
  /**
   * Where this track was sourced from.
   */
  enum WuiVideoSubtitleTrackOrigin origin;
} WuiVideoSubtitleTrackInfo;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiVideoSubtitleTrackInfo {
  struct WuiVideoSubtitleTrackInfo *head;
  uintptr_t len;
} WuiArraySlice_WuiVideoSubtitleTrackInfo;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiVideoSubtitleTrackInfo {
  void (*drop)(void*);
  struct WuiArraySlice_WuiVideoSubtitleTrackInfo (*slice)(const void*);
} WuiArrayVTable_WuiVideoSubtitleTrackInfo;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiVideoSubtitleTrackInfo {
  NonNull data;
  struct WuiArrayVTable_WuiVideoSubtitleTrackInfo vtable;
} WuiArray_WuiVideoSubtitleTrackInfo;

/**
 * FFI-owned wrapper around a [`waterui::Binding`] signal.
 *
 * Opaque to native code; accessed only through the `waterui_read_binding_*`,
 * `waterui_set_binding_*`, `waterui_watch_binding_*`, and
 * `waterui_drop_binding_*` functions generated by the `ffi_binding!` macro.
 */
typedef struct Binding_Option_LiveWindow WuiBinding_Option_LiveWindow;

/**
 * Atomic native snapshot of a live presentation timeline.
 */
typedef struct WuiVideoLiveWindow {
  /**
   * Whether the remaining fields contain a live timeline.
   */
  bool present;
  /**
   * Earliest seekable presentation position in seconds.
   */
  double seekable_start_seconds;
  /**
   * Latest seekable presentation position in seconds.
   */
  double seekable_end_seconds;
  /**
   * Current live edge in presentation seconds.
   */
  double live_edge_seconds;
  /**
   * Backend-recommended playback position in presentation seconds.
   */
  double target_position_seconds;
} WuiVideoLiveWindow;

/**
 * FFI representation of a video event.
 */
typedef struct WuiVideoEvent {
  /**
   * Which kind of event this is; determines which other fields are meaningful.
   */
  enum WuiVideoEventType event_type;
  /**
   * Rust-owned error string. Non-null exactly when `event_type` is `Error`.
   */
  struct WuiStr *error_message;
  /**
   * Playback position in milliseconds, read when `event_type` is `PlaybackMetrics`.
   */
  uint64_t position_ms;
  /**
   * Buffered-ahead duration in milliseconds.
   */
  uint32_t buffered_ms;
  /**
   * Time from load start to first playable frame, in milliseconds.
   */
  uint64_t startup_time_ms;
  /**
   * Whether `av_drift_ms` carries a measured audio/video drift.
   */
  bool av_drift_available;
  /**
   * Observed audio/video drift in milliseconds, read only when
   * `av_drift_available` is `true`.
   */
  float av_drift_ms;
  /**
   * Total number of video frames dropped so far.
   */
  uint64_t dropped_video_frames;
  /**
   * Total number of rebuffering stalls observed so far.
   */
  uint64_t rebuffer_count;
  /**
   * Cumulative time spent rebuffering, in milliseconds.
   */
  uint64_t rebuffer_duration_ms;
  /**
   * Observed network throughput in bits per second; `0` means unknown.
   */
  uint64_t observed_network_throughput_bps;
  /**
   * Whether Picture-in-Picture is currently active, read when `event_type`
   * is `PictureInPictureChanged`.
   */
  bool picture_in_picture_active;
  /**
   * Whether external playback is currently active, read when `event_type`
   * is `ExternalPlaybackChanged`.
   */
  bool external_playback_active;
  /**
   * Whether playback is currently playing, read when `event_type` is
   * `PlaybackStateChanged`.
   */
  bool playback_active;
  /**
   * The active playback output path, read when `event_type` is
   * `PlaybackOutputPathChanged`.
   */
  enum WuiVideoPlaybackOutputPath playback_output_path;
} WuiVideoEvent;

/**
 * Immutable buffering and realtime strategy for native playback.
 */
typedef struct WuiVideoPlaybackPolicy {
  /**
   * Whether this stream is treated as a realtime/live source rather than
   * video-on-demand.
   */
  bool realtime;
  /**
   * Buffer duration to accumulate before starting VOD playback, in milliseconds.
   */
  uint32_t vod_start_buffer_ms;
  /**
   * Buffer duration to accumulate before resuming VOD playback after a
   * stall, in milliseconds.
   */
  uint32_t vod_resume_buffer_ms;
  /**
   * Buffer duration below which VOD playback is considered stalled, in milliseconds.
   */
  uint32_t vod_stall_buffer_ms;
  /**
   * Maximum allowed live-edge lag before the player is considered behind, in milliseconds.
   */
  uint32_t live_max_video_late_ms;
  /**
   * The required playback power path for this policy.
   */
  enum WuiVideoPlaybackPowerPolicy power;
} WuiVideoPlaybackPolicy;

/**
 * Shared reactive playback state embedded by every native video descriptor.
 */
typedef struct WuiVideoPlaybackDescriptor {
  /**
   * Retained playlist and transport controller.
   */
  struct WuiVideoController *controller;
  /**
   * The video source URL as a string (reactive).
   */
  WuiComputed_Str *source;
  /**
   * The source delivery strategy (reactive).
   */
  WuiComputed_Delivery *delivery;
  /**
   * The media title shown in system media controls.
   */
  WuiComputed_Str *title;
  /**
   * The media artist shown in system media controls.
   */
  WuiComputed_Str *artist;
  /**
   * The media album shown in system media controls.
   */
  WuiComputed_Str *album;
  /**
   * Artwork URL shown in system media controls.
   */
  WuiComputed_Str *artwork_url;
  /**
   * Current playback duration in seconds, or `0.0` when unknown.
   */
  WuiBinding_f64 *duration_seconds;
  /**
   * Current presentation position in seconds.
   */
  WuiBinding_f64 *position_seconds;
  /**
   * Requested playing state.
   */
  WuiBinding_bool *desired_playing;
  /**
   * Requested absolute seek position in presentation seconds.
   */
  WuiBinding_f64 *seek_target_seconds;
  /**
   * Monotonic seek-request generation encoded as a wrapping `i32` token.
   */
  WuiBinding_i32 *seek_generation;
  /**
   * Monotonic forward frame-step generation encoded as a wrapping `i32` token.
   */
  WuiBinding_i32 *step_forward_generation;
  /**
   * Monotonic backward frame-step generation encoded as a wrapping `i32` token.
   */
  WuiBinding_i32 *step_backward_generation;
  /**
   * High-level playback phase encoded by `playback_phase_code`.
   */
  WuiBinding_i32 *phase;
  /**
   * Playlist repeat mode encoded by `repeat_mode_code`.
   */
  WuiBinding_i32 *repeat_mode;
  /**
   * Whether the active queue has a next item.
   */
  WuiBinding_bool *has_next;
  /**
   * Whether the active queue has a previous item.
   */
  WuiBinding_bool *has_previous;
  /**
   * Validated playback volume in the inclusive `0.0..=1.0` range.
   */
  WuiBinding_f32 *volume;
  /**
   * Whether playback audio is muted.
   */
  WuiBinding_bool *muted;
  /**
   * Subtitle selection for the current runtime track list.
   */
  WuiBinding_SubtitleSelection *subtitle_selection;
  /**
   * Audio track selection for the current runtime track list.
   */
  WuiBinding_AudioTrackSelection *audio_track_selection;
  /**
   * Video-quality selection for the current runtime track list.
   */
  WuiBinding_VideoTrackSelection *video_track_selection;
  /**
   * Native write handle for the shared selectable-track catalog.
   */
  WuiBinding_TrackCatalog *track_catalog;
  /**
   * Native write handle for the current atomic live-window snapshot.
   */
  WuiBinding_Option_LiveWindow *live_window;
  /**
   * Playback speed (1.0 = normal speed).
   */
  WuiBinding_f32 *playback_rate;
  /**
   * Whether native playback should preserve pitch when rate changes.
   */
  WuiBinding_bool *preserve_pitch;
  /**
   * Optional event handler for native playback events.
   */
  struct WuiVideoEventHandler *on_event;
  /**
   * Buffering and realtime playback strategy.
   */
  struct WuiVideoPlaybackPolicy playback_policy;
} WuiVideoPlaybackDescriptor;

/**
 * FFI representation of the raw Video component (no native controls).
 */
typedef struct WuiVideo {
  /**
   * Shared reactive playback state.
   */
  struct WuiVideoPlaybackDescriptor playback;
  /**
   * How the picture fills the bounds it is given.
   */
  enum WuiContentMode content_mode;
  /**
   * Projection requested by the semantic component.
   */
  enum WuiVideoProjection projection;
  /**
   * Whether the video should loop when it ends.
   */
  bool loops;
} WuiVideo;

/**
 * FFI representation of the interactive `VideoPlayer` component.
 */
typedef struct WuiVideoPlayer {
  /**
   * Shared reactive playback state.
   */
  struct WuiVideoPlaybackDescriptor playback;
  /**
   * How the picture fills the bounds it is given.
   */
  enum WuiContentMode content_mode;
  /**
   * Projection requested by the semantic component.
   */
  enum WuiVideoProjection projection;
  /**
   * Whether to show interactive playback controls.
   */
  bool show_controls;
} WuiVideoPlayer;

/**
 * FFI representation of a single toolbar item and its placement.
 */
typedef struct WuiNavigationToolbarItem {
  /**
   * Where this item is placed within the bar.
   */
  enum WuiNavigationToolbarPlacement placement;
  /**
   * The item's content view.
   */
  struct WuiAnyView *content;
  /**
   * The item's name, or null when it has no semantic label.
   *
   * The platforms disagree about how much of a toolbar action to draw: a Mac
   * shows the icon alone and keeps the name for the overflow menu, its
   * tooltip and assistive technology, while a phone's navigation bar
   * commonly shows the text. The name and the icon therefore travel apart
   * from the content view, so each backend can draw what its chrome calls
   * for rather than placing a view it cannot interpret.
   */
  WuiComputed_StyledStr *title;
  /**
   * The item's icon as a platform symbol, or null.
   *
   * A backend that knows the symbol should prefer this: it renders at the
   * size and weight the platform's own chrome calls for.
   */
  struct WuiSystemIcon *system_icon;
  /**
   * The item's icon as a view to render, or null.
   *
   * Set when the icon is not a platform symbol — a packaged icon set, say.
   */
  struct WuiAnyView *icon;
} WuiNavigationToolbarItem;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiNavigationToolbarItem {
  struct WuiNavigationToolbarItem *head;
  uintptr_t len;
} WuiArraySlice_WuiNavigationToolbarItem;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiNavigationToolbarItem {
  void (*drop)(void*);
  struct WuiArraySlice_WuiNavigationToolbarItem (*slice)(const void*);
} WuiArrayVTable_WuiNavigationToolbarItem;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiNavigationToolbarItem {
  NonNull data;
  struct WuiArrayVTable_WuiNavigationToolbarItem vtable;
} WuiArray_WuiNavigationToolbarItem;

/**
 * FFI representation of a navigation bar's search field configuration.
 */
typedef struct WuiNavigationSearch {
  /**
   * Reactive binding to the current search query text.
   */
  WuiBinding_Str *text;
  /**
   * Reactive computed placeholder text shown when the query is empty.
   */
  WuiComputed_StyledStr *prompt;
} WuiNavigationSearch;

/**
 * FFI representation of an optional navigation search configuration.
 *
 * Used in place of a nullable pointer because `WuiNavigationSearch` embeds
 * value fields rather than being independently heap-allocated.
 */
typedef struct WuiOptionalNavigationSearch {
  /**
   * `true` if `value` holds a real search configuration.
   */
  bool has_value;
  /**
   * The search configuration, meaningful only when `has_value` is `true`.
   */
  struct WuiNavigationSearch value;
} WuiOptionalNavigationSearch;

/**
 * FFI representation of a `NavigationView`'s bar: title, toolbar, search,
 * and appearance.
 */
typedef struct WuiBar {
  /**
   * The bar's title view.
   */
  struct WuiAnyView *title;
  /**
   * The bar's subtitle view.
   */
  struct WuiAnyView *subtitle;
  /**
   * Toolbar items placed around the bar, keyed by placement.
   */
  struct WuiArray_WuiNavigationToolbarItem toolbar;
  /**
   * The bar's optional search field configuration.
   */
  struct WuiOptionalNavigationSearch search;
  /**
   * Reactive computed bar tint color, resolved against the environment.
   */
  WuiComputed_ResolvedColor *color;
  /**
   * Reactive signal controlling whether the bar is hidden.
   */
  WuiComputed_bool *hidden;
  /**
   * The title's display mode (automatic, inline, or large).
   */
  enum WuiNavigationTitleDisplayMode display_mode;
} WuiBar;

/**
 * FFI representation of a navigation destination's lifecycle and pop state.
 */
typedef struct WuiNavigationDestinationState {
  /**
   * Reactive signal reporting whether an interactive pop gesture is allowed.
   */
  WuiComputed_bool *pop_enabled;
  /**
   * Action invoked when the user attempts an interactive pop.
   */
  struct WuiAction *pop_attempted;
  /**
   * Action invoked when the destination appears on screen.
   */
  struct WuiAction *appear;
  /**
   * Action invoked when the destination disappears from screen.
   */
  struct WuiAction *disappear;
  /**
   * Action that programmatically pops this destination.
   */
  struct WuiAction *pop;
} WuiNavigationDestinationState;

/**
 *C ABI mirror of `NavigationView`.
 */
typedef struct WuiNavigationView {
  /**
   *Mirrors the `bar` field of `NavigationView`.
   */
  struct WuiBar bar;
  /**
   *Mirrors the `content` field of `NavigationView`.
   */
  struct WuiAnyView *content;
  /**
   *Mirrors the `state` field of `NavigationView`.
   */
  struct WuiNavigationDestinationState state;
} WuiNavigationView;

/**
 * FFI representation of a navigation push/pop transition and its source anchor.
 */
typedef struct WuiNavigationTransition {
  /**
   * The kind of transition to perform.
   */
  enum WuiNavigationTransitionKind kind;
  /**
   * The zoom-transition source view identifier, meaningful only when
   * `kind` is `WuiNavigationTransitionKind::Zoom`.
   */
  int32_t source_id;
} WuiNavigationTransition;

/**
 * FFI struct for `NavigationStack`<(),()>
 */
typedef struct WuiNavigationStack {
  /**
   * The root view of the navigation stack.
   */
  struct WuiAnyView *root;
  /**
   * Transition style used for push/pop operations.
   */
  struct WuiNavigationTransition transition;
} WuiNavigationStack;

/**
 * Opaque handle to a Rust-owned resolver for split-view detail content.
 *
 * The resolver builds detail content for a selected split identifier. Native
 * backends only pass this pointer back into the FFI functions below; they
 * never inspect its contents.
 */
typedef struct WuiNavigationSplitDetail {
  uint8_t _private[0];
} WuiNavigationSplitDetail;

/**
 * FFI representation of a resizable sidebar's width constraints.
 */
typedef struct WuiNavigationColumnWidth {
  /**
   * Minimum allowed sidebar width.
   */
  float min;
  /**
   * Preferred (default) sidebar width.
   */
  float ideal;
  /**
   * Maximum allowed sidebar width.
   */
  float max;
} WuiNavigationColumnWidth;

/**
 * FFI representation of a `NavigationSplitLayout`'s columns, selections, and
 * resolvers.
 */
typedef struct WuiNavigationSplitLayout {
  /**
   * Sidebar content.
   */
  struct WuiAnyView *sidebar;
  /**
   * Placeholder content for empty regular-width selection.
   */
  struct WuiAnyView *placeholder;
  /**
   * Primary selection encoded as i32 (0 means no selection).
   */
  WuiBinding_i32 *primary_selection;
  /**
   * Optional resolver for the middle column in a three-column split.
   */
  struct WuiNavigationSplitDetail *content;
  /**
   * Optional secondary selection encoded as i32 (0 means no selection).
   */
  WuiBinding_i32 *secondary_selection;
  /**
   * Resolver handle for building detail content from a selected id.
   */
  struct WuiNavigationSplitDetail *detail;
  /**
   * Reactive requested column visibility.
   */
  WuiComputed_i32 *column_visibility;
  /**
   * Native resizable sidebar width constraints.
   */
  struct WuiNavigationColumnWidth sidebar_width;
  /**
   * Native split style.
   */
  enum WuiNavigationSplitStyle style;
} WuiNavigationSplitLayout;

/**
 * FFI representation of a single tab within a `Tabs` component.
 */
typedef struct WuiTab {
  /**
   * The unique identifier for the tab (raw u64 for FFI compatibility).
   */
  uint64_t id;
  /**
   * Pointer to the tab's label view.
   */
  struct WuiAnyView *label;
  /**
   * Pointer to the tab's content view.
   */
  struct WuiTabContent *content;
  /**
   * Optional reactive badge count.
   */
  WuiComputed_i32 *badge;
  /**
   * Reactive enabled state.
   */
  WuiComputed_bool *enabled;
  /**
   * The tab's icon as a platform symbol, or null.
   *
   * A backend that knows the symbol should prefer this: it renders at the
   * size and weight the platform's own chrome calls for.
   */
  struct WuiSystemIcon *system_icon;
  /**
   * The tab's icon as a view to render, or null.
   *
   * Set when the icon is not a platform symbol — a packaged icon set, say.
   * A backend whose tab item takes an image has to rasterize this itself.
   */
  struct WuiAnyView *icon;
} WuiTab;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiTab {
  struct WuiTab *head;
  uintptr_t len;
} WuiArraySlice_WuiTab;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiTab {
  void (*drop)(void*);
  struct WuiArraySlice_WuiTab (*slice)(const void*);
} WuiArrayVTable_WuiTab;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiTab {
  NonNull data;
  struct WuiArrayVTable_WuiTab vtable;
} WuiArray_WuiTab;

/**
 * FFI representation of the `Tabs` component.
 */
typedef struct WuiTabs {
  /**
   * The currently selected tab identifier.
   */
  WuiBinding_Id *selection;
  /**
   * The collection of tabs to display.
   */
  struct WuiArray_WuiTab tabs;
  /**
   * Native adaptive tab style.
   */
  enum WuiTabStyle style;
} WuiTabs;

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiNavigationView {
  struct WuiNavigationView *head;
  uintptr_t len;
} WuiArraySlice_WuiNavigationView;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiNavigationView {
  void (*drop)(void*);
  struct WuiArraySlice_WuiNavigationView (*slice)(const void*);
} WuiArrayVTable_WuiNavigationView;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiNavigationView {
  NonNull data;
  struct WuiArrayVTable_WuiNavigationView vtable;
} WuiArray_WuiNavigationView;

/**
 * One atomic navigation stack mutation passed to a native backend.
 */
typedef struct WuiNavigationTransaction {
  /**
   * Monotonically increasing identifier used to acknowledge this
   * transaction via `waterui_navigation_transition_completed` /
   * `waterui_navigation_transition_cancelled`.
   */
  uint64_t id;
  /**
   * Number of existing stack entries, counted from the root, that are
   * retained unchanged by this transaction.
   */
  uintptr_t retained_prefix;
  /**
   * Number of existing stack entries removed after the retained prefix.
   */
  uintptr_t removed;
  /**
   * The views to insert after the retained prefix, replacing any removed entries.
   */
  struct WuiArray_WuiNavigationView inserted;
} WuiNavigationTransaction;

/**
 * FFI representation of a `GpuSurface` view.
 *
 * This struct is passed to the native backend when rendering the view tree.
 * The native backend consumes it with `waterui_gpu_surface_create`, then owns
 * the returned state for the semantic view lifetime.
 */
typedef struct WuiGpuSurface {
  /**
   * Opaque pointer to the boxed `GpuSurface`.
   * This is consumed during state creation and should not be used after.
   */
  void *surface;
  /**
   * Whether this surface should register as a picture-in-picture host.
   */
  bool has_picture_in_picture_host_id;
  /**
   * Stable picture-in-picture host id when `has_picture_in_picture_host_id` is true.
   */
  uint64_t picture_in_picture_host_id;
} WuiGpuSurface;

/**
 * GPU surface plus retained CEF input and semantic state.
 */
typedef struct WuiCefSurface {
  /**
   * GPU presenter consumed by WaterUI's native GPU surface host.
   */
  struct WuiGpuSurface gpu_surface;
  /**
   * Opaque input state retained until [`waterui_cef_surface_drop`].
   */
  struct WuiCefSurfaceState *state;
} WuiCefSurface;

/**
 * Modifier snapshot for CEF pointer and keyboard input.
 */
typedef struct WuiCefInputModifiers {
  /**
   * Shift key.
   */
  bool shift;
  /**
   * Control key.
   */
  bool control;
  /**
   * Alt or Option key.
   */
  bool alt;
  /**
   * Command key.
   */
  bool command;
  /**
   * Primary pointer button.
   */
  bool primary_button;
  /**
   * Middle pointer button.
   */
  bool middle_button;
  /**
   * Secondary pointer button.
   */
  bool secondary_button;
} WuiCefInputModifiers;

/**
 * One parsed `waterui.invoke(...)` request.
 */
typedef struct WuiBridgeRequest {
  /**
   * Whether the envelope parsed. When false every other field is empty and the
   * call must be ignored.
   */
  bool ok;
  /**
   * Correlates the reply with the page's pending promise.
   */
  uint64_t id;
  /**
   * The handler name the page asked for.
   */
  struct WuiStr name;
  /**
   * The payload, base64-encoded to match `WuiWebViewMessage`.
   */
  struct WuiStr payload_base64;
} WuiBridgeRequest;

/**
 * FFI representation of a `WebView` event.
 */
typedef struct WuiWebViewEvent {
  /**
   * The type of event.
   */
  enum WuiWebViewEventType event_type;
  /**
   * URL associated with the event (for `WillNavigate`, `SslError`, Error, Redirect from).
   */
  struct WuiStr *url;
  /**
   * Second URL (for Redirect to).
   */
  struct WuiStr *url2;
  /**
   * Error/message string (for `SslError`, Error).
   */
  struct WuiStr *message;
  /**
   * Loading progress (0.0 to 1.0, for Loading event).
   */
  float progress;
  /**
   * Whether can navigate back (for `StateChanged`).
   */
  bool can_go_back;
  /**
   * Whether can navigate forward (for `StateChanged`).
   */
  bool can_go_forward;
} WuiWebViewEvent;

/**
 * A C-compatible function wrapper that can be called multiple times.
 *
 * This structure wraps a Rust `Fn` closure to allow it to be passed across
 * the FFI boundary while maintaining proper memory management.
 */
typedef struct WuiFn_WuiWebViewEvent {
  void *data;
  void (*call)(const void*, struct WuiWebViewEvent);
  void (*drop)(void*);
} WuiFn_WuiWebViewEvent;

/**
 * One-shot reply to one bridge call.
 *
 * Distinct from [`WuiJsCallback`], which completes a `run_javascript` call and
 * has no channel to choose.
 */
typedef struct WuiWebViewReply {
  /**
   * Opaque callback state consumed by `call`.
   */
  void *data;
  /**
   * One-shot completion. `success=true` means `result` is base64 of the
   * handler's output and `kind` says how to read it; `false` means `result`
   * is the error message and `kind` is ignored. Native must invoke it
   * exactly once.
   */
  void (*call)(void *data, bool success, enum WuiJsReplyKind kind, struct WuiStr result);
} WuiWebViewReply;

/**
 * Message payload emitted from JavaScript to a native-registered handler.
 *
 * `payload_base64` is base64-encoded bytes from JavaScript.
 * `reply` must be called exactly once for request/response semantics.
 */
typedef struct WuiWebViewMessage {
  /**
   * Base64-encoded message bytes sent from JavaScript.
   */
  struct WuiStr payload_base64;
  /**
   * One-shot callback used to send a base64-encoded reply back to JavaScript.
   */
  struct WuiWebViewReply reply;
} WuiWebViewMessage;

/**
 * A C-compatible function wrapper that can be called multiple times.
 *
 * This structure wraps a Rust `Fn` closure to allow it to be passed across
 * the FFI boundary while maintaining proper memory management.
 */
typedef struct WuiFn_WuiWebViewMessage {
  void *data;
  void (*call)(const void*, struct WuiWebViewMessage);
  void (*drop)(void*);
} WuiFn_WuiWebViewMessage;

/**
 * One-shot completion callback for an owned string result.
 */
typedef struct WuiStringCallback {
  /**
   * Opaque callback state consumed by `call`.
   */
  void *data;
  /**
   * Completes the operation and transfers ownership of `result`.
   */
  void (*call)(void *data, struct WuiStr result);
} WuiStringCallback;

/**
 * Callback for JavaScript execution results.
 */
typedef struct WuiJsCallback {
  /**
   * Opaque callback state consumed by `call`.
   */
  void *data;
  /**
   * One-shot completion. `success=true` means `result` is the value;
   * `false` means it is the error. Native must invoke it exactly once.
   */
  void (*call)(void *data, bool success, struct WuiStr result);
} WuiJsCallback;

/**
 * FFI representation of a `WebView` handle with function pointers.
 *
 * Native backends create this struct with function pointers to their implementation.
 */
typedef struct WuiWebViewHandle {
  /**
   * Opaque pointer to native `WebView` wrapper.
   */
  void *data;
  /**
   * Navigate back in history.
   */
  void (*go_back)(void*);
  /**
   * Navigate forward in history.
   */
  void (*go_forward)(void*);
  /**
   * Navigate to URL.
   */
  void (*go_to)(void*, struct WuiStr);
  /**
   * Stop loading.
   */
  void (*stop)(void*);
  /**
   * Refresh/reload page.
   */
  void (*refresh)(void*);
  /**
   * Returns whether can go back.
   */
  bool (*can_go_back)(const void*);
  /**
   * Returns whether can go forward.
   */
  bool (*can_go_forward)(const void*);
  /**
   * Set user agent string.
   */
  void (*set_user_agent)(void*, struct WuiStr);
  /**
   * Takes ownership of a reactive redirect-following signal.
   */
  void (*set_redirects_enabled)(void*, WuiComputed_bool*);
  /**
   * Inject a script that runs on every page load.
   */
  void (*inject_script)(void*, struct WuiStr, enum WuiScriptInjectionTime);
  /**
   * Set event callback. Native calls this when events occur.
   */
  void (*watch)(void*, struct WuiFn_WuiWebViewEvent);
  /**
   * Register a named handler that can be called from JavaScript.
   *
   * Backends are expected to provide a Promise-based API where possible:
   * JavaScript sends `payload_base64` and receives a base64 reply.
   */
  void (*add_handler)(void*, struct WuiStr, struct WuiFn_WuiWebViewMessage);
  /**
   * Removes a previously added handler.
   */
  void (*remove_handler)(void*, struct WuiStr);
  /**
   * Restricts which documents may reach the bridge.
   *
   * Receives newline-separated URI patterns, or `*` for every origin.
   */
  void (*set_bridge_origins)(void*, struct WuiStr);
  /**
   * Sets a cookie for the web view. The string is a Set-Cookie header value.
   */
  void (*set_cookie)(void*, struct WuiStr);
  /**
   * Gets cookies asynchronously as newline-separated Set-Cookie strings.
   */
  void (*get_cookies)(const void*, struct WuiStringCallback);
  /**
   * Execute JavaScript on the currently loaded page and call callback with result.
   */
  void (*run_javascript)(void*, struct WuiStr, struct WuiJsCallback);
  /**
   * Release the native handle.
   */
  void (*drop)(void*);
} WuiWebViewHandle;

/**
 * Type for the native function that creates a new `WebView`.
 */
typedef struct WuiWebViewHandle (*WuiCreateWebViewFn)(void);

/**
 * FFI representation of a Metadata<AppliedFilter>.
 */
typedef struct WuiAppliedFilter {
  /**
   * The child view to capture (pointer to `WuiAnyView`).
   */
  struct WuiAnyView *content;
  /**
   * Opaque pointer to the boxed `AppliedFilter`.
   * This is consumed during create and should not be used after.
   */
  void *filter;
} WuiAppliedFilter;

/**
 * Native callback invoked when an idle applied-filter surface becomes dirty.
 */
typedef void (*WuiAppliedFilterRedrawCallback)(void *context);

/**
 * Resolved output size returned to native before render scheduling.
 */
typedef struct WuiAppliedFilterOutputSize {
  /**
   * Output width in pixels.
   */
  uint32_t width;
  /**
   * Output height in pixels.
   */
  uint32_t height;
} WuiAppliedFilterOutputSize;

/**
 * Completion invoked after a GPU runtime is ready for installation.
 */
typedef void (*WuiGpuRuntimeCreateCallback)(void *context, struct WuiGpuRuntime *runtime);

/**
 * Releases the native context retained for asynchronous GPU runtime creation.
 */
typedef void (*WuiGpuRuntimeCreateContextDrop)(void *context);

/**
 * Renderer-driven HDR preference exported to native backends before init.
 *
 * `has_preference = false` means the surface should follow backend/global policy.
 */
typedef struct WuiGpuSurfaceHdrPreference {
  /**
   * Whether the renderer provided an explicit HDR/SDR preference.
   */
  bool has_preference;
  /**
   * Explicit preferred dynamic range when `has_preference` is true.
   */
  bool prefers_hdr;
} WuiGpuSurfaceHdrPreference;

/**
 * Native callback invoked when an idle `GpuSurface` becomes dirty.
 */
typedef void (*WuiGpuSurfaceRedrawCallback)(void *context);

/**
 * Completion function invoked after an external GPU capture submission.
 */
typedef void (*WuiGpuCaptureCompletionCallback)(void *context);

/**
 * Releases the foreign completion context after its callback returns.
 */
typedef void (*WuiGpuCaptureCompletionDrop)(void *context);

/**
 * FFI-safe pointer state for passing from native.
 *
 * Native backends should update this before each render call to provide
 * current pointer/cursor information to the GPU renderer.
 */
typedef struct WuiPointerState {
  /**
   * Whether the pointer is currently over this surface.
   */
  bool has_position;
  /**
   * X coordinate in surface-local pixels.
   */
  float x;
  /**
   * Y coordinate in surface-local pixels.
   */
  float y;
  /**
   * Whether there's an active hit (press/touch in progress).
   */
  bool has_hit;
  /**
   * X coordinate where hit started.
   */
  float hit_x;
  /**
   * Y coordinate where hit started.
   */
  float hit_y;
} WuiPointerState;

/**
 * FFI-safe gesture state for zoom/pan interactions.
 *
 * Native backends should update this when pinch, pan, or double-tap
 * gestures are detected to enable interactive chart zoom/pan.
 */
typedef struct WuiGestureState {
  /**
   * Whether a gesture is currently active.
   */
  bool active;
  /**
   * Cumulative pinch scale factor (1.0 = no scaling).
   */
  float pinch_scale;
  /**
   * Whether a pinch center is present.
   */
  bool has_pinch_center;
  /**
   * X coordinate of pinch center in surface-local pixels.
   */
  float pinch_center_x;
  /**
   * Y coordinate of pinch center in surface-local pixels.
   */
  float pinch_center_y;
  /**
   * Pan offset X in pixels since gesture began.
   */
  float pan_offset_x;
  /**
   * Pan offset Y in pixels since gesture began.
   */
  float pan_offset_y;
  /**
   * Whether a double-tap was detected this frame.
   */
  bool double_tap;
} WuiGestureState;

/**
 * FFI-safe combined input state for a `GpuSurface`.
 *
 * This keeps the native bridge minimal by forwarding pointer and gesture
 * snapshots in one call.
 */
typedef struct WuiGpuSurfaceInput {
  /**
   * Current pointer snapshot.
   */
  struct WuiPointerState pointer;
  /**
   * Current gesture snapshot.
   */
  struct WuiGestureState gesture;
} WuiGpuSurfaceInput;

/**
 * FFI representation of output size.
 */
typedef enum WuiOutputSize_Tag {
  /**
   * Match the input view's size.
   */
  WuiOutputSize_MatchInput,
  /**
   * Fixed pixel dimensions.
   */
  WuiOutputSize_Fixed,
  /**
   * Scale factor relative to input.
   */
  WuiOutputSize_Scale,
} WuiOutputSize_Tag;

typedef struct WuiOutputSize_Fixed_Body {
  /**
   * Output width in pixels.
   */
  uint32_t width;
  /**
   * Output height in pixels.
   */
  uint32_t height;
} WuiOutputSize_Fixed_Body;

typedef struct WuiOutputSize_Scale_Body {
  /**
   * Multiplier applied to the input's width and height.
   */
  float factor;
} WuiOutputSize_Scale_Body;

typedef struct WuiOutputSize {
  WuiOutputSize_Tag tag;
  union {
    WuiOutputSize_Fixed_Body fixed;
    WuiOutputSize_Scale_Body scale;
  };
} WuiOutputSize;

/**
 * FFI representation of a `ViewEffect` view.
 *
 * This struct is passed to the native backend when rendering the view tree.
 * The native backend should:
 * 1. Create capture and output layers
 * 2. Call `waterui_view_effect_create` immediately
 * 3. Attach the output layer once it exists
 * 4. Render the child view to the capture layer
 * 5. Call `waterui_view_effect_render` when rendering is scheduled
 */
typedef struct WuiViewEffect {
  /**
   * The child view to capture (pointer to `WuiAnyView`).
   */
  struct WuiAnyView *content;
  /**
   * Opaque pointer to the boxed effect renderer.
   * This is consumed during init and should not be used after.
   */
  void *effect;
  /**
   * Output size configuration.
   */
  struct WuiOutputSize output_size;
} WuiViewEffect;

/**
 * Native callback invoked when an idle view-effect surface becomes dirty.
 */
typedef void (*WuiViewEffectRedrawCallback)(void *context);

/**
 * Callback for returning rendered RGBA data to Rust.
 */
typedef struct ViewRenderCallback {
  /**
   * Opaque data pointer passed to the callback.
   */
  void *data;
  /**
   * One-shot callback function. Native may invoke it asynchronously, but
   * must invoke it exactly once.
   * - `data`: The opaque data pointer
   * - `rgba_ptr`: Pointer to RGBA pixel data (4 bytes per pixel)
   * - `rgba_len`: Length of the RGBA data in bytes
   * - `width`: Rendered width in pixels
   * - `height`: Rendered height in pixels
   */
  void (*call)(void *data,
               const uint8_t *rgba_ptr,
               uintptr_t rgba_len,
               uint32_t width,
               uint32_t height);
} ViewRenderCallback;

/**
 * Type alias for the native view render function.
 *
 * Native implements this function to render a view to RGBA pixels:
 * 1. Create an offscreen rendering context at the given size
 * 2. Render the `AnyView` hierarchy (native widgets + GPU surfaces)
 * 3. Capture the final composited result to RGBA pixels
 * 4. Call the callback with the pixel data
 *
 * The view pointer is an `AnyView` that native should render.
 */
typedef void (*ViewRenderFn)(void *context,
                             void *view,
                             struct WuiSize size,
                             struct ViewRenderCallback callback);

/**
 * A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
 */
typedef struct WuiArraySlice_WuiWindow {
  struct WuiWindow *head;
  uintptr_t len;
} WuiArraySlice_WuiWindow;

/**
 * The pair of function pointers `WuiArray` uses to view and free its backing storage.
 *
 * `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
 * and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
 */
typedef struct WuiArrayVTable_WuiWindow {
  void (*drop)(void*);
  struct WuiArraySlice_WuiWindow (*slice)(const void*);
} WuiArrayVTable_WuiWindow;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 *
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiWindow {
  NonNull data;
  struct WuiArrayVTable_WuiWindow vtable;
} WuiArray_WuiWindow;

/**
 * FFI-compatible representation of an application.
 *
 * This struct is returned by value from `waterui_app()`.
 * Native code can read fields directly.
 */
typedef struct WuiApp {
  /**
   * Array of windows. The first window is the main window.
   */
  struct WuiArray_WuiWindow windows;
  /**
   * The application menu bar as resolved menu items.
   */
  struct WuiAnyViews *menu_bar;
  /**
   * The application environment containing injected services.
   * Returned to native for use during rendering.
   */
  struct WuiEnv *env;
} WuiApp;









































/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_env(struct WuiEnv *value);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_anyview(struct WuiAnyView *value);

/**
 * Gets the id of the anyview type as a 128-bit value for O(1) comparison.
 */
struct WuiTypeId waterui_anyview_id(void);

/**
 * Clones an existing environment instance
 *
 * # Safety
 * The caller must ensure that `env` is a valid pointer to a properly initialized
 * `waterui::Environment` instance and that the environment remains valid for the
 * duration of this function call.
 */
struct WuiEnv *waterui_clone_env(const struct WuiEnv *env);

/**
 * Returns the disabled state in force at this point in the view tree.
 *
 * Disabled state is a scoped subtree attribute installed by `.disabled(...)`,
 * never a field on an individual control's configuration. Every interactive
 * control reads it here, from the same environment it already receives through
 * [`waterui_view_body`], and renders as inactive and ignores input while the
 * signal is `true`.
 *
 * Always returns a non-null signal; it reads `false` when no `.disabled(...)`
 * scope encloses the view. Returns a new reference to the signal — the caller
 * must drop it when done.
 *
 * # Safety
 * The caller must ensure that `env` is a valid pointer to a properly initialized
 * `waterui::Environment` instance and that the environment remains valid for the
 * duration of this function call.
 */
WuiComputed_bool *waterui_env_disabled(const struct WuiEnv *env);

/**
 * Gets the body of a view given the environment
 *
 * # Safety
 * The caller must ensure that both `view` and `env` are valid pointers to properly
 * initialized instances and that they remain valid for the duration of this function call.
 * The `view` pointer will be consumed and should not be used after this call.
 */
struct WuiAnyView *waterui_view_body(struct WuiAnyView *view, struct WuiEnv *env);

/**
 * Gets the id of a view as a 128-bit value for O(1) comparison.
 *
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to a properly
 * initialized `WuiAnyView` instance and that it remains valid for the
 * duration of this function call.
 */
struct WuiTypeId waterui_view_id(const struct WuiAnyView *view);

/**
 * Gets the stretch axis of a view.
 *
 * Returns the `StretchAxis` that indicates how this view stretches to fill
 * available space. For native views, this returns the layout behavior defined
 * by the `NativeView` trait. For non-native views, this will panic.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to a properly
 * initialized `WuiAnyView` instance and that it remains valid for the
 * duration of this function call.
 */
enum WuiStretchAxis waterui_view_stretch_axis(const struct WuiAnyView *view);

/**
 * Moves an owned string into a Rust allocation for nullable FFI payloads.
 *
 * The returned pointer must be consumed exactly once by the API receiving it.
 */
struct WuiStr *waterui_str_box(struct WuiStr value);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_env_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataEnv waterui_force_as_metadata_env(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_navigation_transition_source_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataNavigationTransitionSource waterui_force_as_metadata_navigation_transition_source(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_navigation_transition_destination_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataNavigationTransitionDestination waterui_force_as_metadata_navigation_transition_destination(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_secure_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataSecure waterui_force_as_metadata_secure(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_standard_dynamic_range_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataStandardDynamicRange waterui_force_as_metadata_standard_dynamic_range(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_high_dynamic_range_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataHighDynamicRange waterui_force_as_metadata_high_dynamic_range(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_gesture_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataGesture waterui_force_as_metadata_gesture(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_lifecycle_hook_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataLifecycleHook waterui_force_as_metadata_lifecycle_hook(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_on_event_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataOnEvent waterui_force_as_metadata_on_event(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_cursor_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataCursor waterui_force_as_metadata_cursor(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_ignorable_metadata_accessibility_identifier_id(void);

/**
 * Force-casts an `AnyView` to this ignorable metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains an `IgnorableMetadata<$ty>`.
 */
struct WuiIgnorableMetadataAccessibilityIdentifier waterui_force_as_ignorable_metadata_accessibility_identifier(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_ignorable_metadata_accessibility_label_id(void);

/**
 * Force-casts an `AnyView` to this ignorable metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains an `IgnorableMetadata<$ty>`.
 */
struct WuiIgnorableMetadataAccessibilityLabel waterui_force_as_ignorable_metadata_accessibility_label(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_ignorable_metadata_accessibility_role_id(void);

/**
 * Force-casts an `AnyView` to this ignorable metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains an `IgnorableMetadata<$ty>`.
 */
struct WuiIgnorableMetadataAccessibilityValue waterui_force_as_ignorable_metadata_accessibility_role(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_ignorable_metadata_accessibility_hidden_id(void);

/**
 * Force-casts an `AnyView` to this ignorable metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains an `IgnorableMetadata<$ty>`.
 */
struct WuiIgnorableMetadataAccessibilityValue waterui_force_as_ignorable_metadata_accessibility_hidden(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_ignorable_metadata_accessibility_children_id(void);

/**
 * Force-casts an `AnyView` to this ignorable metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains an `IgnorableMetadata<$ty>`.
 */
struct WuiIgnorableMetadataAccessibilityValue waterui_force_as_ignorable_metadata_accessibility_children(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_ignorable_metadata_accessibility_state_id(void);

/**
 * Force-casts an `AnyView` to this ignorable metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains an `IgnorableMetadata<$ty>`.
 */
struct WuiIgnorableMetadataAccessibilityState waterui_force_as_ignorable_metadata_accessibility_state(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_ignorable_metadata_accessibility_state_signal_id(void);

/**
 * Force-casts an `AnyView` to this ignorable metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains an `IgnorableMetadata<$ty>`.
 */
struct WuiIgnorableMetadataAccessibilityState waterui_force_as_ignorable_metadata_accessibility_state_signal(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_shadow_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataShadow waterui_force_as_metadata_shadow(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_border_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataBorder waterui_force_as_metadata_border(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_scale_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataScale waterui_force_as_metadata_scale(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_rotation_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataRotation waterui_force_as_metadata_rotation(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_offset_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataOffset waterui_force_as_metadata_offset(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_opacity_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataOpacity waterui_force_as_metadata_opacity(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_focused_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataFocused waterui_force_as_metadata_focused(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_ignore_safe_area_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataIgnoreSafeArea waterui_force_as_metadata_ignore_safe_area(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_retain_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataRetain waterui_force_as_metadata_retain(struct WuiAnyView *view);

/**
 * Drops the retained value.
 *
 * # Safety
 * The caller must ensure that `retain` is a valid pointer returned from
 * `waterui_force_as_metadata_retain` and has not been dropped before.
 */
void waterui_drop_retain(struct WuiRetain retain);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_clip_shape_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataClipShape waterui_force_as_metadata_clip_shape(struct WuiAnyView *view);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_shared_action(struct WuiSharedAction *value);

/**
 * Call a shared action with the given environment.
 *
 * # Safety
 * * `action` must be a valid pointer to a `WuiSharedAction`.
 * * `env` must be a valid pointer to a `WuiEnv`.
 */
void waterui_call_shared_action(const struct WuiSharedAction *action, const struct WuiEnv *env);

/**
 * Takes the label value from an owned menu-item label allocation.
 *
 * # Safety
 *
 * `label` must be consumed exactly once.
 */
struct WuiText waterui_menu_item_take_label(struct WuiText *label);

/**
 * Takes the icon value from an owned menu-item icon allocation.
 *
 * # Safety
 *
 * `icon` must be consumed exactly once.
 */
struct WuiSystemIcon waterui_menu_item_take_icon(struct WuiSystemIcon *icon);

/**
 * Takes the shortcut value from an owned menu-item shortcut allocation.
 *
 * # Safety
 *
 * `shortcut` must be consumed exactly once.
 */
struct WuiShortcut waterui_menu_item_take_shortcut(struct WuiShortcut *shortcut);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiMenuItem waterui_force_as_menu_item(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_menu_item_id(void);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_context_menu_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataContextMenu waterui_force_as_metadata_context_menu(struct WuiAnyView *view);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiMenu waterui_force_as_menu(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_menu_id(void);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_draggable_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataDraggable waterui_force_as_metadata_draggable(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_drop_destination_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataDropDestination waterui_force_as_metadata_drop_destination(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_ignorable_metadata_material_background_id(void);

/**
 * Force-casts an `AnyView` to this ignorable metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains an `IgnorableMetadata<$ty>`.
 */
struct WuiIgnorableMetadataMaterialBackground waterui_force_as_ignorable_metadata_material_background(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_hittable_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataHittable waterui_force_as_metadata_hittable(struct WuiAnyView *view);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_action(struct WuiAction *value);

/**
 * Calls an action with the given environment.
 *
 * # Safety
 *
 * * `action` must be a valid pointer to a `waterui_action` struct.
 * * `env` must be a valid pointer to a `waterui_env` struct.
 */
void waterui_call_action(const struct WuiAction *action, const struct WuiEnv *env);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_index_action(struct WuiIndexAction *value);

/**
 * Calls an index action with the given environment and index.
 *
 * # Safety
 *
 * * `action` must be a valid pointer to a `WuiIndexAction` struct.
 * * `env` must be a valid pointer to a `WuiEnv` struct.
 */
void waterui_call_index_action(const struct WuiIndexAction *action,
                               const struct WuiEnv *env,
                               uintptr_t index);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_move_action(struct WuiMoveAction *value);

/**
 * Calls a move action with the given environment and from/to indices.
 *
 * # Safety
 *
 * * `action` must be a valid pointer to a `WuiMoveAction` struct.
 * * `env` must be a valid pointer to a `WuiEnv` struct.
 */
void waterui_call_move_action(const struct WuiMoveAction *action,
                              const struct WuiEnv *env,
                              uintptr_t from_index,
                              uintptr_t to_index);

/**
 * Installs a locale into the environment using a BCP 47 locale string.
 *
 * This installs a reactive locale binding into the environment and publishes
 * the same value to the shared regional runtime. Invalid tags fail immediately.
 *
 * # Safety
 * - `env` must be a valid pointer returned by `waterui_init()`.
 * - `locale_str` must be a valid null-terminated UTF-8 string.
 */
void waterui_env_install_locale_tag(struct WuiEnv *env, const char *locale_tag);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_color(struct WuiColor *value);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiResolvedColor waterui_read_computed_resolved_color(const WuiComputed_ResolvedColor *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_resolved_color(const WuiComputed_ResolvedColor *computed,
                                                              struct WuiWatcher_ResolvedColor *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_resolved_color(WuiComputed_ResolvedColor *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_ResolvedColor *waterui_new_watcher_resolved_color(void *data,
                                                                    void (*call)(void*,
                                                                                 struct WuiResolvedColor,
                                                                                 struct WuiWatcherMetadata*),
                                                                    void (*drop)(void*));

/**
 * Creates a computed signal from native callbacks.
 * # Safety
 * All function pointers must be valid and follow the expected calling conventions.
 */
WuiComputed_ResolvedColor *waterui_new_computed_resolved_color(void *data,
                                                               struct WuiResolvedColor (*get)(const void*),
                                                               struct WuiWatcherGuard *(*watch)(const void*,
                                                                                                struct WuiWatcher_ResolvedColor*),
                                                               void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiColor *waterui_read_binding_color(const WuiBinding_Color *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_color(WuiBinding_Color *binding, struct WuiColor *value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_color(const WuiBinding_Color *binding,
                                                    struct WuiWatcher_Color *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_color(WuiBinding_Color *binding);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_Color *waterui_new_watcher_color(void *data,
                                                   void (*call)(void*,
                                                                struct WuiColor*,
                                                                struct WuiWatcherMetadata*),
                                                   void (*drop)(void*));

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiResolvedColor waterui_force_as_resolved_color(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_resolved_color_id(void);

/**
 * Consumes a semantic color view and returns its owned resolvable color handle.
 *
 * # Safety
 *
 * `view` must own a native `Color` view and must not be used again.
 */
struct WuiColor *waterui_force_as_color(struct WuiAnyView *view);

/**
 * Returns the native semantic color view type id.
 */
struct WuiTypeId waterui_color_id(void);

/**
 * Creates a new linear sRGBA color with optional HDR headroom.
 *
 * `headroom` is an HDR scale factor where `0.0` means SDR and values above
 * `0.0` allow the renderer to apply an extended range multiplier.
 *
 * # Safety
 *
 * This function returns an owned pointer that must be dropped with
 * `waterui_drop_color` unless it is passed to a binding setter that consumes it.
 */
struct WuiColor *waterui_color_from_linear_rgba_headroom(float red,
                                                         float green,
                                                         float blue,
                                                         float alpha,
                                                         float headroom);

/**
 * Creates a new linear sRGBA color (SDR only).
 *
 * # Safety
 *
 * This function returns an owned pointer that must be dropped with
 * `waterui_drop_color` unless it is passed to a binding setter that consumes it.
 */
struct WuiColor *waterui_color_from_srgba(float red, float green, float blue, float alpha);

/**
 * Resolves a color in the given environment.
 *
 * # Safety
 *
 * Both `color` and `env` must be valid, non-null pointers to their respective types.
 */
WuiComputed_ResolvedColor *waterui_resolve_color(const struct WuiColor *color,
                                                 const struct WuiEnv *env);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiResolvedGradient waterui_force_as_resolved_gradient(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_resolved_gradient_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiResolvedShape waterui_force_as_resolved_shape(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_resolved_shape_id(void);

/**
 * Extracts animation metadata from a watcher context.
 *
 * # Safety
 * The metadata pointer must be valid and point to a properly initialized metadata object.
 */
struct WuiAnimation waterui_get_animation(const struct WuiWatcherMetadata *metadata);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
enum WuiCursorStyle waterui_read_computed_cursor_style(const WuiComputed_CursorStyle *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_cursor_style(const WuiComputed_CursorStyle *computed,
                                                            struct WuiWatcher_CursorStyle *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_cursor_style(WuiComputed_CursorStyle *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_CursorStyle *waterui_new_watcher_cursor_style(void *data,
                                                                void (*call)(void*,
                                                                             enum WuiCursorStyle,
                                                                             struct WuiWatcherMetadata*),
                                                                void (*drop)(void*));

/**
 * Gets the current drag data value from a draggable.
 *
 * # Safety
 *
 * * `draggable` must be a valid pointer to a `WuiDraggable`.
 */
struct WuiDragData waterui_draggable_get_data(const struct WuiDraggable *draggable);

/**
 * Drops a draggable.
 *
 * # Safety
 *
 * * `draggable` must be a valid pointer to a `WuiDraggable`.
 */
void waterui_drop_draggable(struct WuiDraggable *draggable);

/**
 * Calls the drop handler with the given data.
 *
 * # Safety
 *
 * * `handler` must be a valid pointer to a `WuiDropDestination`.
 * * `env` must be a valid pointer to a `WuiEnv`.
 * * `data_tag` must be a valid `WuiDragDataTag` value.
 * * `data_value` must be a valid null-terminated UTF-8 string.
 */
void waterui_call_drop_handler(const struct WuiDropDestination *dest,
                               const struct WuiEnv *env,
                               enum WuiDragDataTag data_tag,
                               const char *data_value);

/**
 * Calls the enter handler if set.
 *
 * # Safety
 *
 * * `dest` must be a valid pointer to a `WuiDropDestination`.
 * * `env` must be a valid pointer to a `WuiEnv`.
 */
void waterui_call_drop_enter_handler(const struct WuiDropDestination *dest,
                                     const struct WuiEnv *env);

/**
 * Calls the exit handler if set.
 *
 * # Safety
 *
 * * `dest` must be a valid pointer to a `WuiDropDestination`.
 * * `env` must be a valid pointer to a `WuiEnv`.
 */
void waterui_call_drop_exit_handler(const struct WuiDropDestination *dest,
                                    const struct WuiEnv *env);

/**
 * Drops a drop destination handler.
 *
 * # Safety
 *
 * * `dest` must be a valid pointer to a `WuiDropDestination`.
 */
void waterui_drop_drop_destination(struct WuiDropDestination *dest);

/**
 * Calls a lifecycle hook handler with the given environment.
 *
 * # Safety
 *
 * * `handler` must be a valid pointer to a `WuiLifecycleHookHandler`.
 * * `env` must be a valid pointer to a `WuiEnv`.
 * * This consumes the handler - it can only be called once.
 */
void waterui_call_lifecycle_hook(struct WuiLifecycleHookHandler *handler, const struct WuiEnv *env);

/**
 * Drops a lifecycle hook handler without calling it.
 *
 * # Safety
 *
 * * `handler` must be a valid pointer to a `WuiLifecycleHookHandler`.
 */
void waterui_drop_lifecycle_hook(struct WuiLifecycleHookHandler *handler);

/**
 * Calls an `OnEvent` handler with the given environment.
 * This handler can be called multiple times (repeatable).
 *
 * # Safety
 *
 * * `handler` must be a valid pointer to a `WuiOnEventHandler`.
 * * `env` must be a valid pointer to a `WuiEnv`.
 */
void waterui_call_on_event(const struct WuiOnEventHandler *handler, const struct WuiEnv *env);

/**
 * Calls an `OnEvent` hover-move handler with the given local pointer position.
 *
 * # Safety
 *
 * * `handler` must be a valid pointer to a `WuiOnEventHandler`.
 * * `env` must be a valid pointer to a `WuiEnv`.
 */
void waterui_call_on_hover_event(const struct WuiOnEventHandler *handler,
                                 const struct WuiEnv *env,
                                 float x,
                                 float y);

/**
 * Drops an `OnEvent` handler.
 *
 * # Safety
 *
 * * `handler` must be a valid pointer to a `WuiOnEventHandler`.
 */
void waterui_drop_on_event(struct WuiOnEventHandler *handler);

/**
 * Drops a `WuiGesture`, recursively freeing any composite variants.
 *
 * # Safety
 *
 * The gesture pointer must be valid and properly initialized.
 */
void waterui_drop_gesture(struct WuiGesture *gesture);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_watcher_metadata(struct WuiWatcherMetadata *value);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_box_watcher_guard(struct WuiWatcherGuard *value);

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiStr waterui_read_binding_str(const WuiBinding_Str *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_str(WuiBinding_Str *binding, struct WuiStr value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_str(const WuiBinding_Str *binding,
                                                  struct WuiWatcher_Str *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_str(WuiBinding_Str *binding);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiStr waterui_read_computed_str(const WuiComputed_Str *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_str(const WuiComputed_Str *computed,
                                                   struct WuiWatcher_Str *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_str(WuiComputed_Str *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_Str *waterui_new_watcher_str(void *data,
                                               void (*call)(void*,
                                                            struct WuiStr,
                                                            struct WuiWatcherMetadata*),
                                               void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiStyledStr waterui_read_binding_styled_str(const WuiBinding_StyledStr *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_styled_str(WuiBinding_StyledStr *binding, struct WuiStyledStr value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_styled_str(const WuiBinding_StyledStr *binding,
                                                         struct WuiWatcher_StyledStr *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_styled_str(WuiBinding_StyledStr *binding);

/**
 * Sets a `Binding<StyledStr>` using borrowed UTF-8 bytes.
 *
 * Native text controls can pass their current editor string without first
 * constructing an owned FFI `WuiStr`.
 *
 * # Panics
 * Panics if `bytes` is null while `len` is non-zero.
 *
 * # Safety
 * `binding` must be a valid pointer to `WuiBinding<StyledStr>`. `bytes` must
 * point to `len` valid UTF-8 bytes unless `len` is zero.
 */
void waterui_set_binding_styled_str_utf8(WuiBinding_StyledStr *binding,
                                         const uint8_t *bytes,
                                         uintptr_t len);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_AnyView *waterui_new_watcher_any_view(void *data,
                                                        void (*call)(void*,
                                                                     struct WuiAnyView*,
                                                                     struct WuiWatcherMetadata*),
                                                        void (*drop)(void*));

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
int32_t waterui_read_computed_i32(const WuiComputed_i32 *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_i32(const WuiComputed_i32 *computed,
                                                   struct WuiWatcher_i32 *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_i32(WuiComputed_i32 *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_i32 *waterui_new_watcher_i32(void *data,
                                               void (*call)(void*,
                                                            int32_t,
                                                            struct WuiWatcherMetadata*),
                                               void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
bool waterui_read_binding_bool(const WuiBinding_bool *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_bool(WuiBinding_bool *binding, bool value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_bool(const WuiBinding_bool *binding,
                                                   struct WuiWatcher_bool *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_bool(WuiBinding_bool *binding);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
bool waterui_read_computed_bool(const WuiComputed_bool *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_bool(const WuiComputed_bool *computed,
                                                    struct WuiWatcher_bool *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_bool(WuiComputed_bool *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_bool *waterui_new_watcher_bool(void *data,
                                                 void (*call)(void*,
                                                              bool,
                                                              struct WuiWatcherMetadata*),
                                                 void (*drop)(void*));

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
float waterui_read_computed_f32(const WuiComputed_f32 *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_f32(const WuiComputed_f32 *computed,
                                                   struct WuiWatcher_f32 *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_f32(WuiComputed_f32 *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_f32 *waterui_new_watcher_f32(void *data,
                                               void (*call)(void*, float, struct WuiWatcherMetadata*),
                                               void (*drop)(void*));

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
double waterui_read_computed_f64(const WuiComputed_f64 *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_f64(const WuiComputed_f64 *computed,
                                                   struct WuiWatcher_f64 *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_f64(WuiComputed_f64 *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_f64 *waterui_new_watcher_f64(void *data,
                                               void (*call)(void*,
                                                            double,
                                                            struct WuiWatcherMetadata*),
                                               void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
int32_t waterui_read_binding_i32(const WuiBinding_i32 *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_i32(WuiBinding_i32 *binding, int32_t value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_i32(const WuiBinding_i32 *binding,
                                                  struct WuiWatcher_i32 *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_i32(WuiBinding_i32 *binding);

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
float waterui_read_binding_f32(const WuiBinding_f32 *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_f32(WuiBinding_f32 *binding, float value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_f32(const WuiBinding_f32 *binding,
                                                  struct WuiWatcher_f32 *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_f32(WuiBinding_f32 *binding);

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
double waterui_read_binding_f64(const WuiBinding_f64 *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_f64(WuiBinding_f64 *binding, double value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_f64(const WuiBinding_f64 *binding,
                                                  struct WuiWatcher_f64 *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_f64(WuiBinding_f64 *binding);

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiDateTime waterui_read_binding_date_time(const WuiBinding_DateTime *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_date_time(WuiBinding_DateTime *binding, struct WuiDateTime value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_date_time(const WuiBinding_DateTime *binding,
                                                        struct WuiWatcher_DateTime *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_date_time(WuiBinding_DateTime *binding);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_DateTime *waterui_new_watcher_date_time(void *data,
                                                          void (*call)(void*,
                                                                       struct WuiDateTime,
                                                                       struct WuiWatcherMetadata*),
                                                          void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiArray_WuiDate waterui_read_binding_date_vec(const WuiBinding_Vec_Date *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_date_vec(WuiBinding_Vec_Date *binding, struct WuiArray_WuiDate value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_date_vec(const WuiBinding_Vec_Date *binding,
                                                       struct WuiWatcher_Vec_Date *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_date_vec(WuiBinding_Vec_Date *binding);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiArray_WuiDate waterui_read_computed_date_vec(const WuiComputed_Vec_Date *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_date_vec(const WuiComputed_Vec_Date *computed,
                                                        struct WuiWatcher_Vec_Date *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_date_vec(WuiComputed_Vec_Date *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_Vec_Date *waterui_new_watcher_date_vec(void *data,
                                                         void (*call)(void*,
                                                                      struct WuiArray_WuiDate,
                                                                      struct WuiWatcherMetadata*),
                                                         void (*drop)(void*));

/**
 * Creates a new watcher guard from raw data and a drop function.
 *
 * # Safety
 * The caller must ensure that the provided data pointer and drop function are valid.
 */
struct WuiWatcherGuard *waterui_new_watcher_guard(void *data, void (*drop)(void*));

/**
 * Reads the current value from a Secure binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiStr waterui_read_binding_secure(const WuiBinding_Secure *binding);

/**
 * Sets the value of a Secure binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_secure(WuiBinding_Secure *binding, struct WuiStr value);

/**
 * Watches for changes in a Secure binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_secure(const WuiBinding_Secure *binding,
                                                     struct WuiWatcher_Secure *watcher);

/**
 * Drops a Secure binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_secure(WuiBinding_Secure *binding);

/**
 * Creates a watcher from native callbacks for Secure
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_Secure *waterui_new_watcher_secure(void *data,
                                                     void (*call)(void*,
                                                                  struct WuiStr,
                                                                  struct WuiWatcherMetadata*),
                                                     void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiId waterui_read_binding_id(const WuiBinding_Id *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_id(WuiBinding_Id *binding, struct WuiId value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_id(const WuiBinding_Id *binding,
                                                 struct WuiWatcher_Id *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_id(WuiBinding_Id *binding);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_Id *waterui_new_watcher_id(void *data,
                                             void (*call)(void*,
                                                          struct WuiId,
                                                          struct WuiWatcherMetadata*),
                                             void (*drop)(void*));

/**
 * Opens the inspector for this application.
 *
 * Reveals nothing in particular: use this where the backend cannot say which
 * element the user meant, such as a menu item that is about the application
 * rather than about a point on screen.
 *
 * # Safety
 *
 * `env` must be a valid environment handle that stays alive for this call.
 */
void waterui_inspector_open(const struct WuiEnv *env);

/**
 * Reveals one node in the inspector, opening one if none is attached.
 *
 * `node` is an accessibility node id, which is what the inspector's tree is
 * keyed by. A backend that publishes no tree has no id to pass and should call
 * [`waterui_inspector_open`] instead.
 *
 * # Safety
 *
 * `env` must be a valid environment handle that stays alive for this call.
 */
void waterui_inspector_inspect_node(const struct WuiEnv *env, uint64_t node);

/**
 * Whether this build offers inspection at all.
 *
 * A backend asks before putting "Inspect element" in front of a user, so that
 * a release build shows nothing rather than an entry that does nothing.
 *
 * # Safety
 *
 * `env` must be a valid environment handle that stays alive for this call.
 */
bool waterui_inspector_is_available(const struct WuiEnv *env);

/**
 * Whether anything is watching the accessibility tree.
 *
 * A backend walks its view hierarchy only when the answer is yes: with no
 * inspector attached the tree costs nothing, which is the whole point of
 * asking before building it.
 *
 * # Safety
 *
 * `env` must be a valid environment handle that stays alive for this call.
 */
bool waterui_inspector_wants_tree(const struct WuiEnv *env);

/**
 * Publishes a backend's accessibility tree to the inspector.
 *
 * The whole tree each time: the runtime turns it into a delta, so a backend
 * does not have to track what changed.
 *
 * # Safety
 *
 * `env` must be a valid environment handle. `nodes` must point to `len`
 * initialised nodes, whose strings and children arrays stay valid for the
 * duration of the call.
 */
void waterui_inspector_publish_tree(const struct WuiEnv *env,
                                    uint64_t root,
                                    bool has_focus,
                                    uint64_t focus,
                                    const struct WuiInspectorNode *nodes,
                                    uintptr_t len);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
enum WuiColorScheme waterui_read_computed_color_scheme(const WuiComputed_ColorScheme *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_color_scheme(const WuiComputed_ColorScheme *computed,
                                                            struct WuiWatcher_ColorScheme *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_color_scheme(WuiComputed_ColorScheme *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_ColorScheme *waterui_new_watcher_color_scheme(void *data,
                                                                void (*call)(void*,
                                                                             enum WuiColorScheme,
                                                                             struct WuiWatcherMetadata*),
                                                                void (*drop)(void*));

/**
 * Creates a computed signal from native callbacks.
 * # Safety
 * All function pointers must be valid and follow the expected calling conventions.
 */
WuiComputed_ColorScheme *waterui_new_computed_color_scheme(void *data,
                                                           enum WuiColorScheme (*get)(const void*),
                                                           struct WuiWatcherGuard *(*watch)(const void*,
                                                                                            struct WuiWatcher_ColorScheme*),
                                                           void (*drop)(void*));

/**
 * Installs a color scheme signal into the environment.
 *
 * # Safety
 * The signal pointer must be valid.
 */
void waterui_theme_install_color_scheme(struct WuiEnv *env, WuiComputed_ColorScheme *signal);

/**
 * Returns the current color scheme signal from the environment.
 *
 * # Safety
 * The returned pointer must be dropped by the caller when no longer needed.
 */
WuiComputed_ColorScheme *waterui_theme_color_scheme(const struct WuiEnv *env);

/**
 * Installs a color signal for a specific slot.
 *
 * Takes ownership of the signal pointer.
 *
 * # Safety
 * The signal pointer must be valid.
 */
void waterui_theme_install_color(struct WuiEnv *env,
                                 enum WuiColorSlot slot,
                                 WuiComputed_ResolvedColor *signal);

/**
 * Returns the color signal for a specific slot.
 *
 * Returns a new reference to the signal. Caller must drop it when done.
 *
 * # Safety
 * The env pointer must be valid.
 */
WuiComputed_ResolvedColor *waterui_theme_color(const struct WuiEnv *env, enum WuiColorSlot slot);

/**
 * Installs a font signal for a specific slot.
 *
 * Takes ownership of the signal pointer.
 *
 * # Safety
 * The env pointer must be valid.
 */
void waterui_theme_install_font(struct WuiEnv *env,
                                enum WuiFontSlot slot,
                                WuiComputed_ResolvedFont *signal);

/**
 * Returns the font signal for a specific slot.
 *
 * Returns a new reference to the signal. Caller must drop it when done.
 *
 * # Safety
 * The env pointer must be valid.
 */
WuiComputed_ResolvedFont *waterui_theme_font(const struct WuiEnv *env, enum WuiFontSlot slot);

/**
 * Calls a `ColorScheme` watcher with the given value.
 * Used by native code to notify Rust when color scheme changes.
 * # Safety
 * The watcher pointer must be valid.
 */
void waterui_call_watcher_color_scheme(const struct WuiWatcher_ColorScheme *watcher,
                                       enum WuiColorScheme value);

/**
 * Drops a `ColorScheme` watcher.
 * # Safety
 * The watcher pointer must be valid.
 */
void waterui_drop_watcher_color_scheme(struct WuiWatcher_ColorScheme *watcher);

/**
 * Calls a `ResolvedColor` watcher with the given value.
 * Used by native code to notify Rust when a color value changes.
 * # Safety
 * The watcher pointer must be valid.
 */
void waterui_call_watcher_resolved_color(const struct WuiWatcher_ResolvedColor *watcher,
                                         struct WuiResolvedColor value);

/**
 * Drops a `ResolvedColor` watcher.
 * # Safety
 * The watcher pointer must be valid.
 */
void waterui_drop_watcher_resolved_color(struct WuiWatcher_ResolvedColor *watcher);

/**
 * Calls a `ResolvedFont` watcher with the given value.
 * Used by native code to notify Rust when a font value changes.
 * # Safety
 * The watcher pointer must be valid.
 */
void waterui_call_watcher_resolved_font(const struct WuiWatcher_ResolvedFont *watcher,
                                        struct WuiResolvedFont value);

/**
 * Drops a `ResolvedFont` watcher.
 * # Safety
 * The watcher pointer must be valid.
 */
void waterui_drop_watcher_resolved_font(struct WuiWatcher_ResolvedFont *watcher);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_anyviews(struct WuiAnyViews *value);

/**
 * Gets a view at the specified index.
 *
 * # Safety
 * The caller must ensure that `anyview` is a valid pointer and `index` is within bounds.
 */
struct WuiAnyView *waterui_anyviews_get_view(const struct WuiAnyViews *anyview, uintptr_t index);

/**
 * Gets the number of views in the collection.
 *
 * # Safety
 * The caller must ensure that `anyviews` is a valid pointer.
 */
uintptr_t waterui_anyviews_len(const struct WuiAnyViews *anyviews);

/**
 * Gets the view IDs in `[start, end)` range.
 *
 * # Safety
 * The caller must ensure that `anyviews` is a valid pointer.
 */
struct WuiArray_WuiId waterui_anyviews_get_ids_in_range(const struct WuiAnyViews *anyviews,
                                                        uintptr_t start,
                                                        uintptr_t end);

/**
 * Watches for changes in a views collection within `[start, end)` range.
 *
 * The callback receives the current list of view IDs in the watched range.
 *
 * # Safety
 * - `anyviews` must be a valid pointer.
 * - `data`, `call`, and `drop` must form a valid callback triplet.
 */
struct WuiWatcherGuard *waterui_anyviews_watch_range(const struct WuiAnyViews *anyviews,
                                                     uintptr_t start,
                                                     uintptr_t end,
                                                     void *data,
                                                     void (*call)(void*,
                                                                  struct WuiArray_WuiId,
                                                                  struct WuiWatcherMetadata*),
                                                     void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
enum WuiWindowState waterui_read_binding_window_state(const WuiBinding_WindowState *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_window_state(WuiBinding_WindowState *binding, enum WuiWindowState value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_window_state(const WuiBinding_WindowState *binding,
                                                           struct WuiWatcher_WindowState *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_window_state(WuiBinding_WindowState *binding);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_WindowState *waterui_new_watcher_window_state(void *data,
                                                                void (*call)(void*,
                                                                             enum WuiWindowState,
                                                                             struct WuiWatcherMetadata*),
                                                                void (*drop)(void*));

/**
 * Installs a `WindowManager` into the environment from a native function pointer.
 *
 * Native backends call this during initialization to register their window
 * management implementation. When `Window` views are rendered, the provided
 * callback will be invoked to create and display native windows.
 *
 * # Safety
 *
 * The caller must ensure that:
 * - `env` is a valid pointer to a `WuiEnv`
 * - `context` remains valid until `drop_context` releases it
 * - `show_fn` is valid for `context` and can create native windows
 * - `drop_context` releases `context` exactly once
 */
void waterui_env_install_window_manager(struct WuiEnv *env,
                                        void *context,
                                        WindowShowFn show_fn,
                                        void (*drop_context)(void*));

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiStr waterui_force_as_plain(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_plain_id(void);

/**
 * Returns the type ID for empty views as a 128-bit value.
 */
struct WuiTypeId waterui_empty_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiButton waterui_force_as_button(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_button_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiTextField waterui_force_as_text_field(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_text_field_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiToggle waterui_force_as_toggle(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_toggle_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiSlider waterui_force_as_slider(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_slider_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiStepper waterui_force_as_stepper(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_stepper_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiColorPicker waterui_force_as_color_picker(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_color_picker_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiPicker waterui_force_as_picker(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_picker_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiSecureField waterui_force_as_secure_field(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_secure_field_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiPickerItem waterui_force_as_picker_item(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_picker_item_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiDatePicker waterui_force_as_date_picker(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_date_picker_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiMultiDatePicker waterui_force_as_multi_date_picker(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_multi_date_picker_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiProgress waterui_force_as_progress(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_progress_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiMap waterui_force_as_map(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_map_id(void);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiRegion waterui_read_computed_region(const WuiComputed_Region *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_region(const WuiComputed_Region *computed,
                                                      struct WuiWatcher_Region *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_region(WuiComputed_Region *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_Region *waterui_new_watcher_region(void *data,
                                                     void (*call)(void*,
                                                                  struct WuiRegion,
                                                                  struct WuiWatcherMetadata*),
                                                     void (*drop)(void*));

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiArray_WuiAnnotation waterui_read_computed_annotations(const WuiComputed_Vec_Annotation *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_annotations(const WuiComputed_Vec_Annotation *computed,
                                                           struct WuiWatcher_Vec_Annotation *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_annotations(WuiComputed_Vec_Annotation *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_Vec_Annotation *waterui_new_watcher_annotations(void *data,
                                                                  void (*call)(void*,
                                                                               struct WuiArray_WuiAnnotation,
                                                                               struct WuiWatcherMetadata*),
                                                                  void (*drop)(void*));

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiLocation waterui_read_computed_user_location(const WuiComputed_Option_Location *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_user_location(const WuiComputed_Option_Location *computed,
                                                             struct WuiWatcher_Option_Location *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_user_location(WuiComputed_Option_Location *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_Option_Location *waterui_new_watcher_user_location(void *data,
                                                                     void (*call)(void*,
                                                                                  struct WuiLocation,
                                                                                  struct WuiWatcherMetadata*),
                                                                     void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiMapStatus waterui_read_binding_map_status(const WuiBinding_MapStatus *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_map_status(WuiBinding_MapStatus *binding, struct WuiMapStatus value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_map_status(const WuiBinding_MapStatus *binding,
                                                         struct WuiWatcher_MapStatus *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_map_status(WuiBinding_MapStatus *binding);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_layout(struct WuiLayout *value);

/**
 * Returns the type ID for Spacer views as a 128-bit value.
 * `Spacer` is a raw view that stretches to fill available space.
 */
struct WuiTypeId waterui_spacer_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiFixedContainer waterui_force_as_fixed_container(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_fixed_container_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiContainer waterui_force_as_layout_container(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_layout_container_id(void);

/**
 * Watches the reactive fields used by a layout.
 *
 * # Safety
 *
 * `layout` and `context` must remain valid until the returned watcher is
 * dropped. Both callbacks must run on the layout's owning UI thread.
 */
struct WuiLayoutWatcher *waterui_layout_watch_invalidation(const struct WuiLayout *layout,
                                                           void *context,
                                                           WuiLayoutInvalidationCallback invalidate,
                                                           WuiLayoutInvalidationCallback drop_callback);

/**
 * Drops a layout invalidation watcher.
 *
 * # Safety
 *
 * `watcher` must be returned by [`waterui_layout_watch_invalidation`] and not
 * previously dropped.
 */
void waterui_layout_watcher_drop(struct WuiLayoutWatcher *watcher);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiSize waterui_read_computed_size(const WuiComputed_Size *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_size(const WuiComputed_Size *computed,
                                                    struct WuiWatcher_Size *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_size(WuiComputed_Size *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_Size *waterui_new_watcher_size(void *data,
                                                 void (*call)(void*,
                                                              struct WuiSize,
                                                              struct WuiWatcherMetadata*),
                                                 void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiRect waterui_read_binding_rect(const WuiBinding_Rect *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_rect(WuiBinding_Rect *binding, struct WuiRect value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_rect(const WuiBinding_Rect *binding,
                                                   struct WuiWatcher_Rect *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_rect(WuiBinding_Rect *binding);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_Rect *waterui_new_watcher_rect(void *data,
                                                 void (*call)(void*,
                                                              struct WuiRect,
                                                              struct WuiWatcherMetadata*),
                                                 void (*drop)(void*));

/**
 * Calculates the size required by the layout given a proposal and child proxies.
 *
 * This function implements the new SubView-based negotiation protocol where
 * layouts can query children multiple times with different proposals.
 *
 * # Safety
 *
 * - The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
 * - The `children` array must contain valid `WuiSubView` entries.
 * - The measure callbacks in each child must be safe to call.
 * - The `children` array will be consumed and dropped after this call.
 */
struct WuiViewDimensions waterui_layout_measure(struct WuiLayout *layout,
                                                struct WuiProposalSize proposal,
                                                struct WuiArray_WuiSubView children);

/**
 * Places child views within the specified bounds.
 *
 * Returns an array of Rect values representing the position and size of each child.
 *
 * # Safety
 *
 * - The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
 * - The `children` array must contain valid `WuiSubView` entries.
 * - The measure callbacks in each child must be safe to call.
 * - The `children` array will be consumed and dropped after this call.
 */
struct WuiArray_WuiRect waterui_layout_place(struct WuiLayout *layout,
                                             struct WuiRect bounds,
                                             struct WuiArray_WuiSubView children);

/**
 * Returns the lazy-stack axis the layout advertises, if any.
 *
 * # Safety
 *
 * The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
 */
enum WuiLazyStackAxis waterui_layout_lazy_stack_axis(struct WuiLayout *layout);

/**
 * Returns the lazy-stack inter-item spacing the layout requires.
 *
 * # Safety
 *
 * The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
 */
float waterui_layout_lazy_stack_spacing(struct WuiLayout *layout);

/**
 * Returns the lazy-stack cross-axis horizontal alignment the layout requires.
 *
 * # Safety
 *
 * The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
 */
enum WuiHorizontalAlignment waterui_layout_lazy_stack_horizontal_alignment(struct WuiLayout *layout);

/**
 * Returns the lazy-stack cross-axis vertical alignment the layout requires.
 *
 * # Safety
 *
 * The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
 */
enum WuiVerticalAlignment waterui_layout_lazy_stack_vertical_alignment(struct WuiLayout *layout);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiScrollView waterui_force_as_scroll_view(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_scroll_view_id(void);

/**
 * Consumes a view handle and reinterprets it as a `WuiListItem`.
 *
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `ListItem`; it is consumed by this call and must not be used afterwards.
 */
struct WuiListItem waterui_force_as_list_item(struct WuiAnyView *view);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiList waterui_force_as_list(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_list_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiTable waterui_force_as_table(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_table_id(void);

/**
 * Consumes a table-column descriptor view extracted from a table's known column collection.
 *
 * # Safety
 *
 * `view` must own a `Native<TableColumn>` and must not be used after this call.
 */
struct WuiTableColumn waterui_force_as_table_column(struct WuiAnyView *view);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
enum WuiVideoDelivery waterui_read_computed_video_delivery(const WuiComputed_Delivery *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_video_delivery(const WuiComputed_Delivery *computed,
                                                              struct WuiWatcher_Delivery *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_video_delivery(WuiComputed_Delivery *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_Delivery *waterui_new_watcher_video_delivery(void *data,
                                                               void (*call)(void*,
                                                                            enum WuiVideoDelivery,
                                                                            struct WuiWatcherMetadata*),
                                                               void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiSubtitleSelection waterui_read_binding_subtitle_selection(const WuiBinding_SubtitleSelection *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_subtitle_selection(WuiBinding_SubtitleSelection *binding,
                                            struct WuiSubtitleSelection value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_subtitle_selection(const WuiBinding_SubtitleSelection *binding,
                                                                 struct WuiWatcher_SubtitleSelection *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_subtitle_selection(WuiBinding_SubtitleSelection *binding);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_SubtitleSelection *waterui_new_watcher_subtitle_selection(void *data,
                                                                            void (*call)(void*,
                                                                                         struct WuiSubtitleSelection,
                                                                                         struct WuiWatcherMetadata*),
                                                                            void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiAudioTrackSelection waterui_read_binding_audio_track_selection(const WuiBinding_AudioTrackSelection *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_audio_track_selection(WuiBinding_AudioTrackSelection *binding,
                                               struct WuiAudioTrackSelection value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_audio_track_selection(const WuiBinding_AudioTrackSelection *binding,
                                                                    struct WuiWatcher_AudioTrackSelection *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_audio_track_selection(WuiBinding_AudioTrackSelection *binding);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_AudioTrackSelection *waterui_new_watcher_audio_track_selection(void *data,
                                                                                 void (*call)(void*,
                                                                                              struct WuiAudioTrackSelection,
                                                                                              struct WuiWatcherMetadata*),
                                                                                 void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiVideoTrackSelection waterui_read_binding_video_track_selection(const WuiBinding_VideoTrackSelection *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_video_track_selection(WuiBinding_VideoTrackSelection *binding,
                                               struct WuiVideoTrackSelection value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_video_track_selection(const WuiBinding_VideoTrackSelection *binding,
                                                                    struct WuiWatcher_VideoTrackSelection *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_video_track_selection(WuiBinding_VideoTrackSelection *binding);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_VideoTrackSelection *waterui_new_watcher_video_track_selection(void *data,
                                                                                 void (*call)(void*,
                                                                                              struct WuiVideoTrackSelection,
                                                                                              struct WuiWatcherMetadata*),
                                                                                 void (*drop)(void*));

/**
 * Replaces the audio portion of a native player's shared track catalog.
 *
 * # Safety
 *
 * `binding` must be a live pointer from a video descriptor. `tracks` transfers
 * ownership of its outer array and every nested string to Rust.
 */
void waterui_video_track_catalog_replace_audio(const WuiBinding_TrackCatalog *binding,
                                               struct WuiArray_WuiVideoAudioTrackInfo tracks);

/**
 * Replaces the video portion of a native player's shared track catalog.
 *
 * # Safety
 *
 * `binding` must be a live pointer from a video descriptor. `tracks` transfers
 * ownership of its outer array and every nested string to Rust.
 */
void waterui_video_track_catalog_replace_video(const WuiBinding_TrackCatalog *binding,
                                               struct WuiArray_WuiVideoTrackInfo tracks);

/**
 * Replaces the subtitle portion of a native player's shared track catalog.
 *
 * # Safety
 *
 * `binding` must be a live pointer from a video descriptor. `tracks` transfers
 * ownership of its outer array and every nested string to Rust.
 */
void waterui_video_track_catalog_replace_subtitles(const WuiBinding_TrackCatalog *binding,
                                                   struct WuiArray_WuiVideoSubtitleTrackInfo tracks);

/**
 * Drops the native write handle for a shared selectable-track catalog.
 *
 * # Safety
 *
 * `binding` must be a live pointer from a video descriptor and must be
 * consumed exactly once.
 */
void waterui_drop_video_track_catalog_binding(WuiBinding_TrackCatalog *binding);

/**
 * Replaces the current live-window snapshot atomically.
 *
 * # Safety
 *
 * `binding` must be a live pointer from a video descriptor.
 */
void waterui_video_live_window_replace(const WuiBinding_Option_LiveWindow *binding,
                                       struct WuiVideoLiveWindow window);

/**
 * Drops the native write handle for a live-window binding.
 *
 * # Safety
 *
 * `binding` must be a live pointer from a video descriptor and must be
 * consumed exactly once.
 */
void waterui_drop_video_live_window_binding(WuiBinding_Option_LiveWindow *binding);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_video_event_handler(struct WuiVideoEventHandler *value);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_video_controller(struct WuiVideoController *value);

/**
 * Selects the next item through the session controller retained by a native player.
 *
 * # Panics
 *
 * Panics if the native "next" command is enabled but the controller cannot
 * resolve a next playlist item.
 *
 * # Safety
 *
 * `controller` must be a live pointer from a video playback descriptor.
 */
void waterui_video_controller_next(const struct WuiVideoController *controller);

/**
 * Selects the previous item through the session controller retained by a native player.
 *
 * # Panics
 *
 * Panics if the native "previous" command is enabled but the controller
 * cannot resolve a previous playlist item.
 *
 * # Safety
 *
 * `controller` must be a live pointer from a video playback descriptor.
 */
void waterui_video_controller_previous(const struct WuiVideoController *controller);

/**
 * Delivers one native playback event to an installed Rust handler.
 *
 * # Safety
 *
 * `handler` must be a live pointer from a video playback descriptor. Tagged
 * event fields must satisfy the invariant documented by [`WuiVideoEvent`].
 */
void waterui_video_event_handler_call(const struct WuiVideoEventHandler *handler,
                                      struct WuiVideoEvent event);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiVideo waterui_force_as_video(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_video_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiVideoPlayer waterui_force_as_video_player(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_video_player_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiNavigationView waterui_force_as_navigation_view(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_navigation_view_id(void);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiNavigationStack waterui_force_as_navigation_stack(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_navigation_stack_id(void);

/**
 * Resolves a stack root after the native backend installs its controller.
 *
 * # Safety
 *
 * - `root` must be the owning root pointer returned by
 *   `waterui_force_as_navigation_stack` and is consumed exactly once.
 * - `env` must be the live controller-scoped environment for this stack.
 */
struct WuiAnyView *waterui_navigation_stack_root(struct WuiAnyView *root, const struct WuiEnv *env);

/**
 * Releases a navigation split-detail handle.
 *
 * # Safety
 *
 * `value` must be a valid, owning `WuiNavigationSplitDetail` handle that has
 * not already been dropped; it must not be used after this call.
 */
void waterui_drop_split_navigation_detail(struct WuiNavigationSplitDetail *value);

/**
 * Resolves the active detail navigation view for a selected split identifier.
 *
 * # Safety
 *
 * - `handler` must be a valid pointer to a `WuiNavigationSplitDetail`.
 * - `selected` must encode a valid non-zero split selection id.
 */
struct WuiNavigationView waterui_split_navigation_detail_content(struct WuiNavigationSplitDetail *handler,
                                                                 struct WuiId selected,
                                                                 const struct WuiEnv *env);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiNavigationSplitLayout waterui_force_as_split_navigation_container(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_split_navigation_container_id(void);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_tab_content(struct WuiTabContent *value);

/**
 * Creates a navigation view from tab content.
 *
 * # Safety
 *
 * This function is unsafe because:
 * - `handler` must be a valid, non-null pointer to a `WuiTabContent`
 * - Both pointers must remain valid for the duration of the function call
 * - The caller must ensure proper memory management of the returned view
 */
struct WuiNavigationView waterui_tab_content(struct WuiTabContent *handler,
                                             const struct WuiEnv *env);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiTabs waterui_force_as_tabs(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_tabs_id(void);

/**
 * Installs native navigation callbacks into an environment.
 *
 * # Safety
 *
 * - `env` must be a valid, exclusively borrowed environment pointer.
 * - `data` must remain valid until `drop_context` releases it.
 * - All callback function pointers must be valid and safe to call
 * - `drop_context` must release `data` exactly once.
 */
void waterui_env_install_navigation_controller(struct WuiEnv *env,
                                               void *context,
                                               void (*apply)(void*, struct WuiNavigationTransaction),
                                               void (*drop_context)(void*));

/**
 * Checks if a navigation controller is installed in the environment.
 *
 * Returns true if a `NavigationController` is available, false otherwise.
 * Use this to determine whether to show a back button in navigation views.
 *
 * # Safety
 *
 * - `env` must be a valid pointer to a `WuiEnv`
 */
bool waterui_env_has_navigation_controller(const struct WuiEnv *env);

/**
 * Requests a user-initiated pop from the Rust navigation state.
 *
 * If no `NavigationController` is installed in the environment, this function does nothing.
 *
 * # Safety
 *
 * - `env` must be a valid pointer to a `WuiEnv`
 */
void waterui_navigation_pop(const struct WuiEnv *env);

/**
 * Commits a pop already completed interactively by the native container.
 *
 * # Panics
 *
 * Panics if no `NavigationController` is installed in the environment.
 *
 * # Safety
 *
 * `env` must point to a live environment containing the controller for the
 * native stack that completed the pop.
 */
void waterui_navigation_complete_native_pop(const struct WuiEnv *env, uintptr_t count);

/**
 * Acknowledges successful completion of a native navigation transaction.
 *
 * Stale acknowledgements are ignored because a newer transaction owns the
 * native stack projection.
 *
 * # Panics
 *
 * Panics if no `NavigationController` is installed in the environment.
 *
 * # Safety
 *
 * `env` must point to the live environment for the reporting stack.
 */
bool waterui_navigation_transition_completed(const struct WuiEnv *env, uint64_t id);

/**
 * Acknowledges cancellation of a native navigation transaction.
 *
 * # Panics
 *
 * Panics if no `NavigationController` is installed in the environment.
 *
 * # Safety
 *
 * `env` must point to the live environment for the reporting stack.
 */
bool waterui_navigation_transition_cancelled(const struct WuiEnv *env, uint64_t id);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiCefSurface waterui_force_as_chromium(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_chromium_id(void);

/**
 * Consumes a standard WebView whose selected engine is CEF.
 *
 * # Safety
 *
 * `view` must be a valid owning `WuiAnyView` containing `Native<WebView>`.
 */
struct WuiCefSurface waterui_force_as_cef_webview(struct WuiAnyView *view);

/**
 * Updates focus for a CEF surface.
 *
 * # Safety
 *
 * `state` must be a live state returned by a CEF force-as function.
 */
void waterui_cef_surface_set_focus(const struct WuiCefSurfaceState *state, bool focused);

/**
 * Requests one compositor frame for a visible CEF surface.
 *
 * # Safety
 *
 * `state` must be a live state returned by a CEF force-as function.
 */
void waterui_cef_surface_request_frame(const struct WuiCefSurfaceState *state);

/**
 * Updates the logical browser viewport and device scale.
 *
 * # Safety
 *
 * `state` must be a live state returned by a CEF force-as function.
 */
void waterui_cef_surface_set_viewport(const struct WuiCefSurfaceState *state,
                                      uint32_t width,
                                      uint32_t height,
                                      double scale);

/**
 * Navigates the CEF surface backward.
 *
 * # Safety
 *
 * `state` must be a live state returned by a CEF force-as function.
 */
void waterui_cef_surface_go_back(const struct WuiCefSurfaceState *state);

/**
 * Navigates the CEF surface forward.
 *
 * # Safety
 *
 * `state` must be a live state returned by a CEF force-as function.
 */
void waterui_cef_surface_go_forward(const struct WuiCefSurfaceState *state);

/**
 * Sends pointer movement in surface-local logical coordinates.
 *
 * # Safety
 *
 * `state` must be a live state returned by a CEF force-as function.
 */
void waterui_cef_surface_pointer_move(const struct WuiCefSurfaceState *state,
                                      double x,
                                      double y,
                                      struct WuiCefInputModifiers modifiers);

/**
 * Sends one pointer button transition.
 *
 * # Safety
 *
 * `state` must be a live state returned by a CEF force-as function.
 */
void waterui_cef_surface_pointer_button(const struct WuiCefSurfaceState *state,
                                        bool pressed,
                                        enum WuiCefPointerButton button,
                                        double x,
                                        double y,
                                        struct WuiCefInputModifiers modifiers);

/**
 * Sends a CEF wheel event.
 *
 * # Safety
 *
 * `state` must be a live state returned by a CEF force-as function.
 */
void waterui_cef_surface_scroll(const struct WuiCefSurfaceState *state,
                                double x,
                                double y,
                                double delta_x,
                                double delta_y,
                                struct WuiCefInputModifiers modifiers);

/**
 * Sends one native key transition.
 *
 * `character` is a Unicode scalar value, or zero when the key has no text.
 *
 * # Safety
 *
 * `state` must be a live state returned by a CEF force-as function.
 */
void waterui_cef_surface_key(const struct WuiCefSurfaceState *state,
                             bool pressed,
                             uint32_t native_keycode,
                             uint32_t keyval,
                             uint32_t character,
                             struct WuiCefInputModifiers modifiers);

/**
 * Commits text to the focused Chromium editor.
 *
 * # Safety
 *
 * `state` and `text` must be valid owning FFI values.
 */
void waterui_cef_surface_commit_text(const struct WuiCefSurfaceState *state,
                                     struct WuiStr text,
                                     uint32_t replacement_start,
                                     uint32_t replacement_end);

/**
 * Updates active IME composition text and its UTF-16 selection.
 *
 * # Safety
 *
 * `state` and `text` must be valid owning FFI values.
 */
void waterui_cef_surface_set_composition(const struct WuiCefSurfaceState *state,
                                         struct WuiStr text,
                                         uint32_t selection_start,
                                         uint32_t selection_end,
                                         uint32_t replacement_start,
                                         uint32_t replacement_end);

/**
 * Finishes active IME composition.
 *
 * # Safety
 *
 * `state` must be a live state returned by a CEF force-as function.
 */
void waterui_cef_surface_finish_composition(const struct WuiCefSurfaceState *state,
                                            bool keep_selection);

/**
 * Cancels active IME composition.
 *
 * # Safety
 *
 * `state` must be a live state returned by a CEF force-as function.
 */
void waterui_cef_surface_cancel_composition(const struct WuiCefSurfaceState *state);

/**
 * Executes an editing command in Chromium's focused frame.
 *
 * # Safety
 *
 * `state` must be a live state returned by a CEF force-as function.
 */
void waterui_cef_surface_edit(const struct WuiCefSurfaceState *state,
                              enum WuiCefEditCommand command);

/**
 * Drops retained CEF input and semantic state.
 *
 * # Safety
 *
 * `state` must be returned by a CEF force-as function and consumed once.
 */
void waterui_cef_surface_drop(struct WuiCefSurfaceState *state);

/**
 * Installs the CEF-compatible `NSApplication` subclass before AppKit starts.
 */
void waterui_cef_prepare_macos_application(void);

/**
 * Runs one packaged CEF helper subprocess and returns its exit status.
 */
int32_t waterui_cef_run_packaged_subprocess(void);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_dynamic(struct WuiDynamic *value);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiDynamic *waterui_force_as_dynamic(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_dynamic_id(void);

/**
 * Connects a watcher to a dynamic view.
 * # Safety
 * - The dynamic pointer must be valid.
 * - The watcher pointer will be consumed and freed when the Dynamic is dropped.
 */
void waterui_dynamic_connect(struct WuiDynamic *dynamic, struct WuiWatcher_AnyView *watcher);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiSystemIcon waterui_force_as_system_icon(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_system_icon_id(void);

/**
 * Returns the bridge script every backend injects at document start.
 *
 * Backends that route messages themselves — the Apple backend keeps its handler
 * table in Swift — use this together with
 * [`waterui_webview_parse_bridge_request`] and
 * [`waterui_webview_bridge_reply_script`], so the envelope format stays defined
 * only in `waterui_webview::bridge`.
 */
struct WuiStr waterui_webview_bridge_script(void);

/**
 * Parses one envelope produced by the bridge script.
 *
 * # Safety
 *
 * `envelope` must be an owning `WuiStr`; it is consumed.
 */
struct WuiBridgeRequest waterui_webview_parse_bridge_request(struct WuiStr envelope);

/**
 * Renders the JavaScript that settles one bridge call.
 *
 * # Safety
 *
 * `payload_base64` must be an owning `WuiStr`; it is consumed. On success it is
 * base64 handler output and `kind` says whether those bytes are a JSON value or
 * opaque; otherwise it is an error message and `kind` is ignored.
 */
struct WuiStr waterui_webview_bridge_reply_script(uint64_t id,
                                                  bool success,
                                                  enum WuiJsReplyKind kind,
                                                  struct WuiStr payload_base64);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_web_view(struct WuiWebView *value);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiWebView *waterui_force_as_webview(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_webview_id(void);

/**
 * Gets the native handle pointer from a `WebView`.
 *
 * Returns the opaque pointer to the native `WebView` wrapper (Swift/Kotlin).
 * This pointer can be used by native backends to access the underlying
 * `WKWebView` or Android `WebView`.
 *
 * # Safety
 *
 * - The caller must ensure that `webview` is a valid pointer to a `WuiWebView`.
 * - The `WebView` must have been created via the FFI `WebViewController` (i.e., the handle
 *   must be an `FfiWebViewHandle`). This is guaranteed when the native backend properly
 *   installed the `WebViewController` via `waterui_env_install_webview_controller`.
 *
 * # Panics
 *
 * Panics if the `WebView`'s handle was not created via the FFI `WebViewController`
 * (i.e., it does not downcast to `FfiWebViewHandle`).
 */
void *waterui_webview_native_handle(struct WuiWebView *webview);

/**
 * Installs a `WebViewController` into the environment from a native factory function.
 *
 * Native backends call this during initialization to register their `WebView` factory.
 * The factory creates blank `WebViews` that can be navigated with `go_to()`.
 *
 * # Safety
 *
 * The caller must ensure that:
 * - `env` is a valid pointer to a `WuiEnv`
 * - `create_fn` is a valid function pointer that returns a properly initialized `WuiWebViewHandle`
 */
void waterui_env_install_webview_controller(struct WuiEnv *env, WuiCreateWebViewFn create_fn);

/**
 * Returns whether a `WebView` controller is already installed in the environment.
 *
 * # Safety
 *
 * `env` must be a valid pointer to a live `WuiEnv`.
 */
bool waterui_env_has_webview_controller(const struct WuiEnv *env);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_font(struct WuiFont *value);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiStyledStr waterui_read_computed_styled_str(const WuiComputed_StyledStr *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_styled_str(const WuiComputed_StyledStr *computed,
                                                          struct WuiWatcher_StyledStr *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_styled_str(WuiComputed_StyledStr *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_StyledStr *waterui_new_watcher_styled_str(void *data,
                                                            void (*call)(void*,
                                                                         struct WuiStyledStr,
                                                                         struct WuiWatcherMetadata*),
                                                            void (*drop)(void*));

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
enum WuiHorizontalAlignment waterui_read_computed_horizontal_alignment(const WuiComputed_HorizontalAlignment *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_horizontal_alignment(const WuiComputed_HorizontalAlignment *computed,
                                                                    struct WuiWatcher_HorizontalAlignment *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_horizontal_alignment(WuiComputed_HorizontalAlignment *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_HorizontalAlignment *waterui_new_watcher_horizontal_alignment(void *data,
                                                                                void (*call)(void*,
                                                                                             enum WuiHorizontalAlignment,
                                                                                             struct WuiWatcherMetadata*),
                                                                                void (*drop)(void*));

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiText waterui_force_as_text(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_text_id(void);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiResolvedFont waterui_read_computed_resolved_font(const WuiComputed_ResolvedFont *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_resolved_font(const WuiComputed_ResolvedFont *computed,
                                                             struct WuiWatcher_ResolvedFont *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_resolved_font(WuiComputed_ResolvedFont *computed);

/**
 * Creates a watcher from native callbacks.
 *
 * # Safety
 *
 * All function pointers must be valid and `data` must remain valid
 * until `drop` is called exactly once.
 */
struct WuiWatcher_ResolvedFont *waterui_new_watcher_resolved_font(void *data,
                                                                  void (*call)(void*,
                                                                               struct WuiResolvedFont,
                                                                               struct WuiWatcherMetadata*),
                                                                  void (*drop)(void*));

/**
 * Creates a computed signal from native callbacks.
 * # Safety
 * All function pointers must be valid and follow the expected calling conventions.
 */
WuiComputed_ResolvedFont *waterui_new_computed_resolved_font(void *data,
                                                             struct WuiResolvedFont (*get)(const void*),
                                                             struct WuiWatcherGuard *(*watch)(const void*,
                                                                                              struct WuiWatcher_ResolvedFont*),
                                                             void (*drop)(void*));

/**
 * Creates a new `WuiResolvedFont` with a properly initialized empty family string.
 *
 * This function is needed for native code (Android JNI) to create `WuiResolvedFont`
 * structs with valid vtables for the family field.
 */
struct WuiResolvedFont waterui_resolved_font_new(float size, enum WuiFontWeight weight);

/**
 * Creates a concrete `Font` from resolved font properties.
 *
 * `family` can be an empty string to indicate system font.
 *
 * # Safety
 * `family` must contain valid UTF-8 bytes.
 */
struct WuiFont *waterui_font_from_resolved(float size,
                                           enum WuiFontWeight weight,
                                           struct WuiStr family);

/**
 * Resolves a font in the given environment.
 *
 * # Safety
 * Both `font` and `env` must be valid, non-null pointers.
 */
WuiComputed_ResolvedFont *waterui_resolve_font(const struct WuiFont *font,
                                               const struct WuiEnv *env);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's `TypeId` (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_applied_filter_id(void);

/**
 * Force-casts an `AnyView` to this metadata type.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
struct WuiAppliedFilter waterui_force_as_metadata_applied_filter(struct WuiAnyView *view);

/**
 * Creates persistent state and immediately consumes the semantic filter.
 *
 * Presentation targets are attached later with [`waterui_applied_filter_attach`],
 * so a view destroyed before layout still has one clear Rust owner.
 *
 * # Safety
 *
 * `filter_ffi` must be a valid, unconsumed descriptor returned by
 * `waterui_force_as_metadata_applied_filter`.
 * `env` must contain an installed GPU runtime.
 *
 * # Panics
 *
 * Panics if `filter_ffi`'s inner filter pointer is null, meaning the descriptor
 * was already consumed by a previous call to this function.
 */
struct WuiAppliedFilterState *waterui_applied_filter_create(struct WuiAppliedFilter *filter_ffi,
                                                            const struct WuiEnv *env);

/**
 * Attaches a native presentation target while preserving the semantic filter.
 *
 * # Safety
 *
 * - `state` must come from [`waterui_applied_filter_create`].
 * - `output_layer` must remain valid until [`waterui_applied_filter_detach`].
 * - The state must currently be detached.
 *
 * # Panics
 *
 * Panics if `state` already has an output surface attached, or if
 * `input_width`/`input_height` is zero.
 */
void waterui_applied_filter_attach(struct WuiAppliedFilterState *state,
                                   void *output_layer,
                                   uint32_t input_width,
                                   uint32_t input_height,
                                   bool prefers_hdr);

/**
 * Detaches the presentation target without destroying the semantic filter.
 *
 * # Safety
 *
 * `state` must be valid and currently attached.
 *
 * # Panics
 *
 * Panics if `state` does not currently have an output surface attached.
 */
void waterui_applied_filter_detach(struct WuiAppliedFilterState *state);

/**
 * Installs the native wake target for reactive filter redraw requests.
 *
 * The wake callback may be called from any thread. `drop_callback` releases
 * `context` after the callback is replaced or the applied filter is destroyed.
 *
 * # Safety
 *
 * - `state` and `context` must be valid.
 * - Both callbacks must be thread-safe and use `context` only for the lifetime
 *   retained by `drop_callback`.
 */
void waterui_applied_filter_set_redraw_callback(struct WuiAppliedFilterState *state,
                                                void *context,
                                                WuiAppliedFilterRedrawCallback wake,
                                                WuiAppliedFilterRedrawCallback drop_callback);

/**
 * Starts filter setup on the main-thread local executor.
 *
 * # Arguments
 *
 * * `state` - Pointer to attached state from `waterui_applied_filter_create`
 *
 * # Safety
 *
 * - `state` must be a valid pointer from `waterui_applied_filter_create`
 * - A presentation target must be attached
 */
void waterui_applied_filter_setup(struct WuiAppliedFilterState *state);

/**
 * Returns whether asynchronous filter setup has completed.
 *
 * Completion also triggers the installed redraw callback.
 *
 * # Safety
 *
 * `state` must be a valid pointer returned by [`waterui_applied_filter_create`].
 */
bool waterui_applied_filter_is_ready(const struct WuiAppliedFilterState *state);

/**
 * Render the filter.
 *
 * This function applies the filter to the captured input and renders to the output.
 * Pass current width/height - resources are recreated if size changed.
 *
 * # Arguments
 *
 * * `state` - Pointer to attached persistent state
 * * `width` - Current width in pixels
 * * `height` - Current height in pixels
 *
 * # Returns
 *
 * Whether another frame is needed for animation or an effect callback that
 * arrived while this frame was rendering.
 *
 * # Safety
 *
 * - `state` must be a valid pointer from `waterui_applied_filter_create`
 * - A presentation target must be attached
 * - `waterui_applied_filter_setup` must have completed
 *
 * # Panics
 *
 * Panics if the asynchronous setup started by `waterui_applied_filter_setup`
 * has not completed yet.
 */
bool waterui_applied_filter_render(struct WuiAppliedFilterState *state,
                                   uint32_t width,
                                   uint32_t height);

/**
 * Resolve the current output size from the latest observed filter state.
 *
 * # Safety
 *
 * - `state` must be a valid pointer from `waterui_applied_filter_create`
 */
struct WuiAppliedFilterOutputSize waterui_applied_filter_resolve_output_size(struct WuiAppliedFilterState *state,
                                                                             uint32_t input_width,
                                                                             uint32_t input_height);

/**
 * Prepare the capture texture for rendering.
 *
 * Ensures the capture texture matches the requested dimensions.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_applied_filter_create` with an attached target.
 *
 * # Panics
 *
 * Panics if `state` does not currently have an output surface attached.
 */
void waterui_applied_filter_prepare_capture(struct WuiAppliedFilterState *state,
                                            uint32_t width,
                                            uint32_t height);

/**
 * Get a pointer to the Metal texture backing the capture texture (Apple only).
 *
 * This exposes the underlying `MTLTexture` so native code can render directly
 * into the wgpu capture texture without extra copies.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_applied_filter_create` with an attached target.
 *
 * # Panics
 *
 * Panics if `state` does not currently have a capture texture, meaning the
 * presentation target is detached.
 */
void *waterui_applied_filter_get_capture_metal_texture(struct WuiAppliedFilterState *state);

/**
 * Clean up `AppliedFilter` resources.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_applied_filter_create`,
 * and must not be used after this call.
 */
void waterui_applied_filter_drop(struct WuiAppliedFilterState *state);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_gpu_runtime(struct WuiGpuRuntime *value);

/**
 * Creates a GPU runtime asynchronously.
 *
 * `complete` receives ownership of the runtime. Rust releases `context`
 * through `drop_context` immediately after the completion returns.
 *
 * # Safety
 *
 * `context`, `complete`, and `drop_context` must remain valid until completion.
 */
void waterui_gpu_runtime_create(void *context,
                                WuiGpuRuntimeCreateCallback complete,
                                WuiGpuRuntimeCreateContextDrop drop_context);

/**
 * Installs a GPU runtime into an environment, consuming the runtime handle.
 *
 * # Safety
 *
 * `env` and `runtime` must be valid owning pointers. `runtime` is consumed.
 */
void waterui_env_install_gpu_runtime(struct WuiEnv *env, struct WuiGpuRuntime *runtime);

/**
 * Returns the Metal device owned by the environment's GPU runtime.
 *
 * The returned `MTLDevice` pointer is borrowed and remains valid while the
 * environment or one of its clones retains the installed runtime.
 *
 * # Safety
 *
 * `env` must be valid and contain an installed GPU runtime.
 */
void *waterui_gpu_runtime_metal_device(const struct WuiEnv *env);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiGpuSurface waterui_force_as_gpu_surface(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_gpu_surface_id(void);

/**
 * Returns the renderer-driven HDR preference for a `WuiGpuSurface`.
 *
 * This must be called before `waterui_gpu_surface_create` consumes the surface.
 *
 * # Safety
 *
 * - `surface` must be a valid pointer obtained from `waterui_force_as_gpu_surface`
 * - `surface` must not have been consumed by `waterui_gpu_surface_create`
 */
struct WuiGpuSurfaceHdrPreference waterui_gpu_surface_hdr_preference(const struct WuiGpuSurface *surface);

/**
 * Creates the persistent state for one semantic `GpuSurface`.
 *
 * This consumes the renderer exactly once. Native presentation surfaces are
 * attached and detached independently with [`waterui_gpu_surface_attach`] and
 * [`waterui_gpu_surface_detach`].
 *
 * # Safety
 *
 * - `surface` must be a valid, unconsumed descriptor returned by
 *   `waterui_force_as_gpu_surface`.
 * - `env` must remain valid for this call; the state stores its own clone.
 *
 * # Panics
 *
 * Panics if `surface`'s inner `GpuSurface` pointer is null, meaning the
 * descriptor was already consumed by a previous call to this function.
 */
struct WuiGpuSurfaceState *waterui_gpu_surface_create(struct WuiGpuSurface *surface,
                                                      const struct WuiEnv *env);

/**
 * Measures the semantic GPU view without touching presentation resources.
 *
 * # Safety
 *
 * `state` must be valid and this function must run on the renderer's owning
 * thread.
 */
struct WuiViewDimensions waterui_gpu_surface_measure(const struct WuiGpuSurfaceState *state,
                                                     struct WuiProposalSize proposal);

/**
 * Returns the layout priority declared by the semantic GPU view.
 *
 * # Safety
 *
 * `state` must be valid and this function must run on the renderer's owning
 * thread.
 */
int32_t waterui_gpu_surface_priority(const struct WuiGpuSurfaceState *state);

/**
 * Replaces the native presentation surface while preserving the semantic
 * `GpuView` and its persistent renderer resources.
 *
 * Android calls this when `SurfaceView` receives a replacement `Surface`.
 *
 * # Safety
 *
 * - `state` must be a valid pointer returned by [`waterui_gpu_surface_create`].
 * - `layer` must remain valid until [`waterui_gpu_surface_detach`] is called.
 * - The state must currently be detached.
 *
 * # Panics
 *
 * Panics if `state` already has a native surface attached, or if `width` or
 * `height` is zero.
 */
void waterui_gpu_surface_attach(struct WuiGpuSurfaceState *state,
                                void *layer,
                                uint32_t width,
                                uint32_t height,
                                bool prefers_hdr);

/**
 * Detaches the current native presentation surface without destroying the
 * semantic `GpuView` or its persistent renderer resources.
 *
 * # Safety
 *
 * `state` must be valid and currently have an attached native surface.
 *
 * # Panics
 *
 * Panics if `state` does not currently have a native surface attached.
 */
void waterui_gpu_surface_detach(struct WuiGpuSurfaceState *state);

/**
 * Installs the native wake target for renderer-driven redraw requests.
 *
 * The wake callback may be called from any thread. `drop_callback` releases
 * `context` after the callback is replaced or the GPU state is destroyed.
 *
 * # Safety
 *
 * - `state` and `context` must be valid.
 * - Both callbacks must be thread-safe and use `context` only for the lifetime
 *   retained by `drop_callback`.
 */
void waterui_gpu_surface_set_redraw_callback(struct WuiGpuSurfaceState *state,
                                             void *context,
                                             WuiGpuSurfaceRedrawCallback wake,
                                             WuiGpuSurfaceRedrawCallback drop_callback);

/**
 * Returns whether the renderer's asynchronous setup has completed.
 *
 * Setup completion also triggers the installed redraw callback, so native
 * backends can use this value to resolve first-paint readiness without polling.
 *
 * # Safety
 *
 * `state` must be a valid pointer returned by [`waterui_gpu_surface_create`].
 */
bool waterui_gpu_surface_is_ready(const struct WuiGpuSurfaceState *state);

/**
 * Render a single frame.
 *
 * This function should be called when the surface is dirty (size/input/state changed)
 * and backend should schedule another frame when `needs_redraw` is true.
 *
 * # Arguments
 *
 * * `state` - Pointer to the persistent state from `waterui_gpu_surface_create`
 * * `width` - Current surface width in pixels (from layout)
 * * `height` - Current surface height in pixels (from layout)
 *
 * # Returns
 *
 * Whether another frame should be scheduled immediately.
 *
 * # Safety
 *
 * `state` must be valid and have an attached native surface.
 *
 * # Panics
 *
 * Panics if `width` or `height` is zero.
 */
bool waterui_gpu_surface_render(struct WuiGpuSurfaceState *state, uint32_t width, uint32_t height);

/**
 * Starts asynchronous renderer setup for an external Metal render target.
 *
 * Completion triggers the installed redraw callback. Native must wait until
 * [`waterui_gpu_surface_is_ready`] returns true before rendering into the texture.
 *
 * # Safety
 *
 * `state` must be valid and `texture` must point to a live `MTLTexture`.
 */
void waterui_gpu_surface_prepare_metal_texture(struct WuiGpuSurfaceState *state, void *texture);

/**
 * Render a single frame into an external Metal texture (Apple only).
 *
 * # Safety
 * `state` must be valid, `texture` must point to a `MTLTexture`.
 *
 * # Panics
 *
 * Panics if `texture` is null.
 */
struct WuiGpuCaptureFence *waterui_gpu_surface_render_to_metal_texture(struct WuiGpuSurfaceState *state,
                                                                       void *texture,
                                                                       uint32_t width,
                                                                       uint32_t height);

/**
 * Schedules one external capture submission completion and consumes its fence.
 *
 * `callback` runs exactly once on the shared GPU completion thread. `drop`
 * then runs exactly once on that same thread, including if the completion
 * driver fails before invoking `callback`. Native callbacks should only
 * enqueue their next platform-specific stage and return immediately.
 *
 * # Safety
 *
 * `fence` must be a valid owning pointer returned by
 * [`waterui_gpu_surface_render_to_metal_texture`] and must be consumed once.
 * `context`, `callback`, and `drop` must remain valid until completion; Rust
 * consumes the context and releases it through `drop`.
 */
void waterui_gpu_capture_fence_on_complete(struct WuiGpuCaptureFence *fence,
                                           void *context,
                                           WuiGpuCaptureCompletionCallback callback,
                                           WuiGpuCaptureCompletionDrop drop);

/**
 * Clean up GPU resources.
 *
 * This function should be called when the `GpuSurface` view is destroyed.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_gpu_surface_create`,
 * and must not be used after this call.
 */
void waterui_gpu_surface_drop(struct WuiGpuSurfaceState *state);

/**
 * Update both pointer and gesture state for a `GpuSurface`.
 *
 * Native backends should prefer this API to minimize bridge calls.
 *
 * # Arguments
 *
 * * `state` - Pointer to the persistent state from `waterui_gpu_surface_create`
 * * `input` - Combined pointer + gesture snapshot
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_gpu_surface_create`.
 */
void waterui_gpu_surface_set_input(struct WuiGpuSurfaceState *state,
                                   struct WuiGpuSurfaceInput input);

/**
 * # Safety
 *
 * `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
 * `Native<_>` of the expected view type; it is consumed by this call and must
 * not be used afterwards.
 */
struct WuiViewEffect waterui_force_as_view_effect(struct WuiAnyView *view);

/**
 * Returns the stable `TypeId` identifying this view type across the FFI.
 */
struct WuiTypeId waterui_view_effect_id(void);

/**
 * Creates persistent state and immediately consumes the semantic effect renderer.
 *
 * Presentation targets are attached later with [`waterui_view_effect_attach`],
 * so a view destroyed before layout still has one clear Rust owner.
 *
 * # Safety
 *
 * `effect` must be a valid, unconsumed descriptor returned by
 * `waterui_force_as_view_effect`.
 * `env` must contain an installed GPU runtime.
 *
 * # Panics
 *
 * Panics if `effect`'s inner effect pointer is null, meaning the descriptor
 * was already consumed by a previous call to this function.
 */
struct WuiViewEffectState *waterui_view_effect_create(struct WuiViewEffect *effect,
                                                      const struct WuiEnv *env);

/**
 * Attaches a native presentation target while preserving the effect renderer.
 *
 * # Safety
 *
 * - `state` must come from [`waterui_view_effect_create`].
 * - `layer` must remain valid until [`waterui_view_effect_detach`].
 * - The state must currently be detached.
 *
 * # Panics
 *
 * Panics if `state` already has an output surface attached, if
 * `input_width`/`input_height` is zero, or if the computed output size is zero.
 */
void waterui_view_effect_attach(struct WuiViewEffectState *state,
                                void *layer,
                                uint32_t input_width,
                                uint32_t input_height,
                                bool prefers_hdr);

/**
 * Detaches the native presentation target without destroying the effect renderer.
 *
 * # Safety
 *
 * `state` must be valid and currently attached.
 *
 * # Panics
 *
 * Panics if `state` does not currently have an output surface attached.
 */
void waterui_view_effect_detach(struct WuiViewEffectState *state);

/**
 * Installs the native wake target for renderer-driven redraw requests.
 *
 * # Safety
 *
 * - `state` and `context` must be valid.
 * - Both callbacks must be thread-safe and use `context` only until
 *   `drop_callback` releases it.
 */
void waterui_view_effect_set_redraw_callback(struct WuiViewEffectState *state,
                                             void *context,
                                             WuiViewEffectRedrawCallback wake,
                                             WuiViewEffectRedrawCallback drop_callback);

/**
 * Imports the Metal texture containing the captured child view.
 *
 * # Safety
 *
 * - state must be a valid pointer from `waterui_view_effect_create`.
 * - texture must point to a live `MTLTexture` for the duration of this call.
 *
 * # Panics
 *
 * Panics if the imported Metal texture did not resolve to a supported
 * texture format.
 */
void waterui_view_effect_set_input_metal_texture(struct WuiViewEffectState *state,
                                                 void *texture,
                                                 uint32_t width,
                                                 uint32_t height);

/**
 * Returns whether asynchronous effect setup has completed.
 *
 * Completion also triggers the installed redraw callback.
 *
 * # Safety
 *
 * `state` must be a valid pointer returned by [`waterui_view_effect_create`].
 */
bool waterui_view_effect_is_ready(const struct WuiViewEffectState *state);

/**
 * Render the effect.
 *
 * This function applies the effect to the captured input and renders to the output.
 *
 * # Arguments
 *
 * * `state` - Pointer to attached persistent state
 *
 * # Returns
 *
 * Whether another frame should be scheduled immediately.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_view_effect_create` with an attached target.
 *
 * # Panics
 *
 * Panics if no input texture was imported before this call.
 */
bool waterui_view_effect_render(struct WuiViewEffectState *state);

/**
 * Clean up `ViewEffect` resources.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_view_effect_create`,
 * and must not be used after this call.
 */
void waterui_view_effect_drop(struct WuiViewEffectState *state);

/**
 * Installs a `ViewRenderer` into the environment from a native function pointer.
 *
 * Native backends call this during initialization to register their view
 * rendering implementation. The renderer is used to capture views as RGBA
 * pixels for the preview system.
 *
 * # Safety
 *
 * The caller must ensure that:
 * - `env` is a valid pointer to a `WuiEnv`
 * - `context` remains valid until `drop_context` releases it
 * - `render_fn` is valid for `context`
 * - `drop_context` releases `context` exactly once
 */
void waterui_env_install_view_renderer(struct WuiEnv *env,
                                       void *context,
                                       ViewRenderFn render_fn,
                                       void (*drop_context)(void*));

WuiEnv* waterui_init(void);

struct WuiApp waterui_app(WuiEnv *env);

#ifdef __cplusplus
}
#endif
