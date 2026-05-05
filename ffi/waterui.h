// Generate by generate_header.rs, do not modify by hand.

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
 * FFI representation of StretchAxis enum.
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
typedef enum WuiLifeCycle {
  WuiLifeCycle_Appear,
  WuiLifeCycle_Disappear,
} WuiLifeCycle;

/**
 * FFI event enum for repeatable interaction events.
 */
typedef enum WuiEvent {
  WuiEvent_HoverEnter,
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
 * Maps to SwiftUI's Material types on Apple platforms.
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
 * FFI-safe horizontal paragraph alignment.
 */
typedef enum WuiHorizontalAlignment {
  WuiHorizontalAlignment_Leading = 0,
  WuiHorizontalAlignment_Center = 1,
  WuiHorizontalAlignment_Trailing = 2,
} WuiHorizontalAlignment;

typedef enum WuiVerticalAlignment {
  WuiVerticalAlignment_Top = 0,
  WuiVerticalAlignment_Center = 1,
  WuiVerticalAlignment_Bottom = 2,
  WuiVerticalAlignment_FirstBaseline = 3,
  WuiVerticalAlignment_LastBaseline = 4,
} WuiVerticalAlignment;

typedef enum WuiLazyStackAxis {
  WuiLazyStackAxis_Unsupported = 0,
  WuiLazyStackAxis_Vertical = 1,
  WuiLazyStackAxis_Horizontal = 2,
} WuiLazyStackAxis;

typedef enum WuiAxis {
  WuiAxis_Horizontal,
  WuiAxis_Vertical,
  WuiAxis_All,
} WuiAxis;

typedef enum WuiButtonStyle {
  WuiButtonStyle_Automatic,
  WuiButtonStyle_Plain,
  WuiButtonStyle_Link,
  WuiButtonStyle_Borderless,
  WuiButtonStyle_Bordered,
  WuiButtonStyle_BorderedProminent,
} WuiButtonStyle;

typedef enum WuiFontWeight {
  WuiFontWeight_Thin,
  WuiFontWeight_UltraLight,
  WuiFontWeight_Light,
  WuiFontWeight_Normal,
  WuiFontWeight_Medium,
  WuiFontWeight_SemiBold,
  WuiFontWeight_Bold,
  WuiFontWeight_UltraBold,
  WuiFontWeight_Black,
} WuiFontWeight;

typedef enum WuiKeyboardType {
  WuiKeyboardType_Text,
  WuiKeyboardType_Email,
  WuiKeyboardType_URL,
  WuiKeyboardType_Number,
  WuiKeyboardType_PhoneNumber,
} WuiKeyboardType;

typedef enum WuiToggleStyle {
  WuiToggleStyle_Automatic,
  WuiToggleStyle_Switch,
  WuiToggleStyle_Checkbox,
} WuiToggleStyle;

typedef enum WuiPickerStyle {
  WuiPickerStyle_Automatic,
  WuiPickerStyle_Menu,
  WuiPickerStyle_Radio,
} WuiPickerStyle;

typedef enum WuiDatePickerType {
  WuiDatePickerType_Date,
  WuiDatePickerType_HourAndMinute,
  WuiDatePickerType_HourMinuteAndSecond,
  WuiDatePickerType_DateHourAndMinute,
  WuiDatePickerType_DateHourMinuteAndSecond,
} WuiDatePickerType;

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
   * Always use large title.
   */
  WuiNavigationTitleDisplayMode_Large = 2,
} WuiNavigationTitleDisplayMode;

/**
 * FFI struct for NavigationStack<(),()>
 */
typedef enum WuiNavigationTransition {
  WuiNavigationTransition_PushPop = 0,
  WuiNavigationTransition_Fade = 1,
  WuiNavigationTransition_None = 2,
} WuiNavigationTransition;

/**
 * Position of the tab bar within the tab container.
 */
typedef enum WuiTabPosition {
  /**
   * Tab bar is positioned at the top of the container.
   */
  WuiTabPosition_Top = 0,
  /**
   * Tab bar is positioned at the bottom of the container.
   */
  WuiTabPosition_Bottom = 1,
} WuiTabPosition;

typedef enum WuiAspectRatio {
  WuiAspectRatio_Fit = 0,
  WuiAspectRatio_Fill = 1,
  WuiAspectRatio_Stretch = 2,
} WuiAspectRatio;

/**
 * FFI representation of video events.
 */
typedef enum WuiVideoEventType {
  WuiVideoEventType_ReadyToPlay = 0,
  WuiVideoEventType_Ended = 1,
  WuiVideoEventType_Error = 2,
  WuiVideoEventType_Buffering = 3,
  WuiVideoEventType_BufferingEnded = 4,
  WuiVideoEventType_BufferLevel = 5,
  WuiVideoEventType_PlaybackMetrics = 6,
  WuiVideoEventType_PictureInPictureChanged = 7,
  WuiVideoEventType_NextRequested = 8,
  WuiVideoEventType_PreviousRequested = 9,
} WuiVideoEventType;

typedef enum WuiProgressStyle {
  WuiProgressStyle_Linear,
  WuiProgressStyle_Circular,
} WuiProgressStyle;

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
 * FFI representation of WebView event types.
 */
typedef enum WuiWebViewEventType {
  /**
   * No event (initial state).
   */
  WuiWebViewEventType_None = 0,
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
 * Input texture type for ViewEffect.
 */
typedef enum WuiInputType {
  /**
   * wgpu texture pointer (from GpuSurface child - zero copy optimization)
   */
  WuiInputType_WgpuTexture,
  /**
   * MTLTexture handle (Apple - zero copy)
   * The native side should create the MTLTexture from IOSurface
   */
  WuiInputType_MetalTexture,
  /**
   * AHardwareBuffer handle (Android - zero copy)
   */
  WuiInputType_AHardwareBuffer,
  /**
   * Raw pixel data (fallback with copy)
   */
  WuiInputType_PixelData,
} WuiInputType;

/**
 * FFI-safe cursor style enum.
 */
typedef enum WuiCursorStyle {
  WuiCursorStyle_Arrow = 0,
  WuiCursorStyle_PointingHand = 1,
  WuiCursorStyle_IBeam = 2,
  WuiCursorStyle_Crosshair = 3,
  WuiCursorStyle_OpenHand = 4,
  WuiCursorStyle_ClosedHand = 5,
  WuiCursorStyle_NotAllowed = 6,
  WuiCursorStyle_ResizeLeft = 7,
  WuiCursorStyle_ResizeRight = 8,
  WuiCursorStyle_ResizeUp = 9,
  WuiCursorStyle_ResizeDown = 10,
  WuiCursorStyle_ResizeLeftRight = 11,
  WuiCursorStyle_ResizeUpDown = 12,
  WuiCursorStyle_Move = 13,
  WuiCursorStyle_Wait = 14,
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

typedef enum WuiGradientType {
  WuiGradientType_Linear = 0,
  WuiGradientType_Radial = 1,
  WuiGradientType_Angular = 2,
  WuiGradientType_Mesh = 3,
} WuiGradientType;

/**
 * Locale enum for common locales (for convenience).
 *
 * For locales not in this enum, use `waterui_env_install_locale_string()`.
 */
typedef enum WuiLocale {
  /**
   * English (US)
   */
  WuiLocale_EnUs = 0,
  /**
   * English (UK)
   */
  WuiLocale_EnGb = 1,
  /**
   * Chinese (Simplified, China)
   */
  WuiLocale_ZhCn = 2,
  /**
   * Chinese (Traditional, Taiwan)
   */
  WuiLocale_ZhTw = 3,
  /**
   * Chinese (Traditional, Hong Kong)
   */
  WuiLocale_ZhHk = 4,
  /**
   * Japanese
   */
  WuiLocale_Ja = 5,
  /**
   * Korean
   */
  WuiLocale_Ko = 6,
  /**
   * German
   */
  WuiLocale_De = 7,
  /**
   * French
   */
  WuiLocale_Fr = 8,
  /**
   * Spanish
   */
  WuiLocale_Es = 9,
  /**
   * Russian
   */
  WuiLocale_Ru = 10,
  /**
   * Arabic
   */
  WuiLocale_Ar = 11,
} WuiLocale;

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
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_AnyView Binding_AnyView;

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
typedef struct Binding_Date Binding_Date;

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
typedef struct Binding_Font Binding_Font;

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
typedef struct Binding_Vec_Date Binding_Vec_Date;

/**
 * A `Binding<T>` represents a mutable value of type `T` that can be observed.
 *
 * Bindings provide a reactive way to work with values. When a binding's value
 * changes, it can notify watchers that have registered interest in the value.
 */
typedef struct Binding_Volume Binding_Volume;

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
typedef struct Computed_AnyView Computed_AnyView;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_AnyViews_AnyView Computed_AnyViews_AnyView;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_Color Computed_Color;

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
typedef struct Computed_Date Computed_Date;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_DateTime Computed_DateTime;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_Font Computed_Font;

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
typedef struct Computed_Id Computed_Id;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_MenuItems Computed_MenuItems;

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
typedef struct Computed_Vec_PickerItem_Id Computed_Vec_PickerItem_Id;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_Vec_ResolvedMenuItem Computed_Vec_ResolvedMenuItem;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_Vec_TableColumn Computed_Vec_TableColumn;

/**
 * A wrapper around a boxed implementation of the `ComputedImpl` trait.
 *
 * This type represents a computation that can be evaluated to produce a result of type `T`.
 * The computation is stored as a boxed trait object, allowing for dynamic dispatch.
 */
typedef struct Computed_Video Computed_Video;

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
 * Normalized coordinates (0.0-1.0) for positioning.
 *
 * Used to specify both anchor points on views and target positions in parent.
 * Values outside 0.0-1.0 are valid and will position outside bounds.
 */
typedef struct UnitPoint UnitPoint;

typedef struct WuiAction WuiAction;

typedef struct WuiAnyView WuiAnyView;

typedef struct WuiAnyViews WuiAnyViews;

/**
 * Opaque state held by the native backend after initialization.
 */
typedef struct WuiAppliedFilterState WuiAppliedFilterState;

typedef struct WuiColor WuiColor;

/**
 * Opaque wrapper for Draggable.
 */
typedef struct WuiDraggableWrapper WuiDraggableWrapper;

/**
 * Wrapper for DropDestination to avoid orphan rule issues.
 */
typedef struct WuiDropHandler WuiDropHandler;

typedef struct WuiDynamic WuiDynamic;

typedef struct WuiEnv WuiEnv;

typedef struct WuiFont WuiFont;

/**
 * Opaque state held by the native backend after initialization.
 *
 * Uses shared device/queue from `SharedGpuContext` for efficiency.
 * Only the Surface is created per-view.
 */
typedef struct WuiGpuSurfaceState WuiGpuSurfaceState;

typedef struct WuiIndexAction WuiIndexAction;

typedef struct WuiLayout WuiLayout;

/**
 * Wrapper for LifeCycleHook to avoid orphan rule issues.
 */
typedef struct WuiLifeCycleHookHandler WuiLifeCycleHookHandler;

typedef struct WuiMoveAction WuiMoveAction;

/**
 * Wrapper for OnEvent to avoid orphan rule issues.
 */
typedef struct WuiOnEventHandler WuiOnEventHandler;

typedef struct WuiSharedAction WuiSharedAction;

typedef struct WuiTabContent WuiTabContent;

/**
 * Opaque state held by the native backend after initialization.
 */
typedef struct WuiViewEffectState WuiViewEffectState;

typedef struct WuiWatcherGuard WuiWatcherGuard;

typedef struct WuiWatcherMetadata WuiWatcherMetadata;

typedef struct WuiWatcher_AnyView WuiWatcher_AnyView;

typedef struct WuiWatcher_AnyViews_AnyView WuiWatcher_AnyViews_AnyView;

typedef struct WuiWatcher_Color WuiWatcher_Color;

typedef struct WuiWatcher_ColorScheme WuiWatcher_ColorScheme;

typedef struct WuiWatcher_CursorStyle WuiWatcher_CursorStyle;

typedef struct WuiWatcher_Date WuiWatcher_Date;

typedef struct WuiWatcher_DateTime WuiWatcher_DateTime;

typedef struct WuiWatcher_Font WuiWatcher_Font;

typedef struct WuiWatcher_HorizontalAlignment WuiWatcher_HorizontalAlignment;

typedef struct WuiWatcher_Id WuiWatcher_Id;

typedef struct WuiWatcher_MenuItems WuiWatcher_MenuItems;

typedef struct WuiWatcher_Region WuiWatcher_Region;

typedef struct WuiWatcher_ResolvedColor WuiWatcher_ResolvedColor;

typedef struct WuiWatcher_ResolvedFont WuiWatcher_ResolvedFont;

typedef struct WuiWatcher_Secure WuiWatcher_Secure;

typedef struct WuiWatcher_Str WuiWatcher_Str;

typedef struct WuiWatcher_StyledStr WuiWatcher_StyledStr;

typedef struct WuiWatcher_Vec_Annotation WuiWatcher_Vec_Annotation;

typedef struct WuiWatcher_Vec_Date WuiWatcher_Vec_Date;

typedef struct WuiWatcher_Vec_PickerItem_Id WuiWatcher_Vec_PickerItem_Id;

typedef struct WuiWatcher_Vec_TableColumn WuiWatcher_Vec_TableColumn;

typedef struct WuiWatcher_Video WuiWatcher_Video;

typedef struct WuiWatcher_WindowState WuiWatcher_WindowState;

typedef struct WuiWatcher_bool WuiWatcher_bool;

typedef struct WuiWatcher_f32 WuiWatcher_f32;

typedef struct WuiWatcher_f64 WuiWatcher_f64;

typedef struct WuiWatcher_i32 WuiWatcher_i32;

typedef struct WuiWebView WuiWebView;

/**
 * Type ID as a 128-bit value for O(1) comparison.
 *
 * Uses 128-bit FNV-1a hash of `type_name()` for stability across dylib boundaries,
 * which is required for the preview system that loads user code as a dylib.
 */
typedef struct WuiTypeId {
  uint64_t low;
  uint64_t high;
} WuiTypeId;

typedef struct WuiMetadata_____WuiEnv {
  struct WuiAnyView *content;
  struct WuiEnv *value;
} WuiMetadata_____WuiEnv;

/**
 * Type alias for Metadata<Environment> FFI struct
 * Layout: { content: *mut WuiAnyView, value: *mut WuiEnv }
 */
typedef struct WuiMetadata_____WuiEnv WuiMetadataEnv;

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

typedef struct WuiMetadata_WuiSecureMarker {
  struct WuiAnyView *content;
  struct WuiSecureMarker value;
} WuiMetadata_WuiSecureMarker;

/**
 * Type alias for Metadata<Secure> FFI struct
 * Layout: { content: *mut WuiAnyView, value: WuiSecureMarker }
 */
typedef struct WuiMetadata_WuiSecureMarker WuiMetadataSecure;

/**
 * C-compatible empty marker struct for dynamic range metadata.
 */
typedef struct WuiDynamicRangeMarker {
  uint8_t _marker;
} WuiDynamicRangeMarker;

typedef struct WuiMetadata_WuiDynamicRangeMarker {
  struct WuiAnyView *content;
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
  uint32_t count;
} WuiGesture_Tap_Body;

typedef struct WuiGesture_LongPress_Body {
  uint32_t duration;
} WuiGesture_LongPress_Body;

typedef struct WuiGesture_Drag_Body {
  float min_distance;
} WuiGesture_Drag_Body;

typedef struct WuiGesture_Magnification_Body {
  float initial_scale;
} WuiGesture_Magnification_Body;

typedef struct WuiGesture_Rotation_Body {
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

typedef struct WuiMetadata_WuiGestureObserver {
  struct WuiAnyView *content;
  struct WuiGestureObserver value;
} WuiMetadata_WuiGestureObserver;

/**
 * Type alias for Metadata<GestureObserver> FFI struct
 */
typedef struct WuiMetadata_WuiGestureObserver WuiMetadataGesture;

/**
 * FFI-safe representation of a lifecycle hook.
 */
typedef struct WuiLifeCycleHook {
  /**
   * The lifecycle event to listen for.
   */
  enum WuiLifeCycle lifecycle;
  /**
   * Opaque pointer to the LifeCycleHook (owns the handler).
   */
  struct WuiLifeCycleHookHandler *handler;
} WuiLifeCycleHook;

typedef struct WuiMetadata_WuiLifeCycleHook {
  struct WuiAnyView *content;
  struct WuiLifeCycleHook value;
} WuiMetadata_WuiLifeCycleHook;

/**
 * Type alias for Metadata<LifeCycleHook> FFI struct
 */
typedef struct WuiMetadata_WuiLifeCycleHook WuiMetadataLifeCycleHook;

/**
 * FFI-safe representation of an event handler.
 */
typedef struct WuiOnEvent {
  /**
   * The event type to listen for.
   */
  enum WuiEvent event;
  /**
   * Opaque pointer to the OnEvent (owns the handler).
   */
  struct WuiOnEventHandler *handler;
} WuiOnEvent;

typedef struct WuiMetadata_WuiOnEvent {
  struct WuiAnyView *content;
  struct WuiOnEvent value;
} WuiMetadata_WuiOnEvent;

/**
 * Type alias for Metadata<OnEvent> FFI struct
 */
typedef struct WuiMetadata_WuiOnEvent WuiMetadataOnEvent;

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

typedef struct WuiMetadata_WuiCursor {
  struct WuiAnyView *content;
  struct WuiCursor value;
} WuiMetadata_WuiCursor;

/**
 * Type alias for Metadata<Cursor> FFI struct
 */
typedef struct WuiMetadata_WuiCursor WuiMetadataCursor;

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

typedef struct WuiMetadata_WuiShadow {
  struct WuiAnyView *content;
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

typedef struct WuiMetadata_WuiBorder {
  struct WuiAnyView *content;
  struct WuiBorder value;
} WuiMetadata_WuiBorder;

/**
 * Type alias for Metadata<Border> FFI struct
 */
typedef struct WuiMetadata_WuiBorder WuiMetadataBorder;

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

typedef struct WuiMetadata_WuiScale {
  struct WuiAnyView *content;
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

typedef struct WuiMetadata_WuiRotation {
  struct WuiAnyView *content;
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

typedef struct WuiMetadata_WuiOffset {
  struct WuiAnyView *content;
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

typedef struct WuiMetadata_WuiOpacity {
  struct WuiAnyView *content;
  struct WuiOpacity value;
} WuiMetadata_WuiOpacity;

/**
 * Type alias for Metadata<Opacity> FFI struct
 */
typedef struct WuiMetadata_WuiOpacity WuiMetadataOpacity;

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

typedef struct WuiMetadata_WuiFocused {
  struct WuiAnyView *content;
  struct WuiFocused value;
} WuiMetadata_WuiFocused;

/**
 * Type alias for Metadata<Focused> FFI struct
 */
typedef struct WuiMetadata_WuiFocused WuiMetadataFocused;

/**
 * FFI-safe representation of IgnoreSafeArea.
 */
typedef struct WuiIgnoreSafeArea {
  /**
   * Which edges should ignore safe area.
   */
  struct WuiEdgeSet edges;
} WuiIgnoreSafeArea;

typedef struct WuiMetadata_WuiIgnoreSafeArea {
  struct WuiAnyView *content;
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

typedef struct WuiMetadata_WuiRetain {
  struct WuiAnyView *content;
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
  float x;
  float y;
} WuiPathCommand_MoveTo_Body;

typedef struct WuiPathCommand_LineTo_Body {
  float x;
  float y;
} WuiPathCommand_LineTo_Body;

typedef struct WuiPathCommand_QuadTo_Body {
  float cx;
  float cy;
  float x;
  float y;
} WuiPathCommand_QuadTo_Body;

typedef struct WuiPathCommand_CubicTo_Body {
  float c1x;
  float c1y;
  float c2x;
  float c2y;
  float x;
  float y;
} WuiPathCommand_CubicTo_Body;

typedef struct WuiPathCommand_Arc_Body {
  float cx;
  float cy;
  float rx;
  float ry;
  float start;
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

typedef struct WuiArraySlice_WuiPathCommand {
  struct WuiPathCommand *head;
  uintptr_t len;
} WuiArraySlice_WuiPathCommand;

typedef struct WuiArrayVTable_WuiPathCommand {
  void (*drop)(void*);
  struct WuiArraySlice_WuiPathCommand (*slice)(const void*);
} WuiArrayVTable_WuiPathCommand;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
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

typedef struct WuiMetadata_WuiClipShape {
  struct WuiAnyView *content;
  struct WuiClipShape value;
} WuiMetadata_WuiClipShape;

/**
 * Type alias for Metadata<ClipShape> FFI struct
 */
typedef struct WuiMetadata_WuiClipShape WuiMetadataClipShape;

typedef struct Computed_MenuItems WuiComputed_MenuItems;

/**
 * FFI-safe representation of a context menu.
 */
typedef struct WuiContextMenu {
  /**
   * The menu items as a computed array.
   */
  WuiComputed_MenuItems *items;
} WuiContextMenu;

typedef struct WuiMetadata_WuiContextMenu {
  struct WuiAnyView *content;
  struct WuiContextMenu value;
} WuiMetadata_WuiContextMenu;

/**
 * Type alias for Metadata<ContextMenu> FFI struct
 */
typedef struct WuiMetadata_WuiContextMenu WuiMetadataContextMenu;

typedef struct Computed_StyledStr WuiComputed_StyledStr;

typedef struct Computed_HorizontalAlignment WuiComputed_HorizontalAlignment;

typedef struct WuiText {
  WuiComputed_StyledStr *content;
  WuiComputed_HorizontalAlignment *paragraph_alignment;
} WuiText;

typedef struct WuiArraySlice_u8 {
  uint8_t *head;
  uintptr_t len;
} WuiArraySlice_u8;

typedef struct WuiArrayVTable_u8 {
  void (*drop)(void*);
  struct WuiArraySlice_u8 (*slice)(const void*);
} WuiArrayVTable_u8;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_u8 {
  NonNull data;
  struct WuiArrayVTable_u8 vtable;
} WuiArray_u8;

typedef struct WuiStr {
  struct WuiArray_u8 _0;
} WuiStr;

/**
 * FFI representation of the SystemIcon component.
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

typedef struct Computed_bool WuiComputed_bool;

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
  struct WuiText label;
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
  WuiComputed_MenuItems *items;
} WuiMenuItem;

typedef struct WuiArraySlice_WuiMenuItem {
  struct WuiMenuItem *head;
  uintptr_t len;
} WuiArraySlice_WuiMenuItem;

typedef struct WuiArrayVTable_WuiMenuItem {
  void (*drop)(void*);
  struct WuiArraySlice_WuiMenuItem (*slice)(const void*);
} WuiArrayVTable_WuiMenuItem;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiMenuItem {
  NonNull data;
  struct WuiArrayVTable_WuiMenuItem vtable;
} WuiArray_WuiMenuItem;

/**
 * FFI-safe representation of a Menu component.
 */
typedef struct WuiMenu {
  /**
   * The label view displayed on the menu button.
   */
  struct WuiAnyView *label;
  /**
   * The fully resolved menu items as a computed array.
   */
  WuiComputed_MenuItems *items;
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

typedef struct WuiMetadata_WuiDraggable {
  struct WuiAnyView *content;
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

typedef struct WuiMetadata_WuiDropDestination {
  struct WuiAnyView *content;
  struct WuiDropDestination value;
} WuiMetadata_WuiDropDestination;

/**
 * Type alias for Metadata<DropDestination> FFI struct
 */
typedef struct WuiMetadata_WuiDropDestination WuiMetadataDropDestination;

/**
 * FFI-safe representation of IgnorableMetadata<MaterialBackground>
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

typedef struct WuiMetadata_WuiHittable {
  struct WuiAnyView *content;
  struct WuiHittable value;
} WuiMetadata_WuiHittable;

/**
 * Type alias for Metadata<Hittable> FFI struct
 */
typedef struct WuiMetadata_WuiHittable WuiMetadataHittable;

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

typedef struct WuiResolvedColor {
  float red;
  float green;
  float blue;
  float opacity;
  float headroom;
} WuiResolvedColor;

typedef struct Computed_ResolvedColor WuiComputed_ResolvedColor;

typedef struct Binding_Color WuiBinding_Color;

typedef struct Computed_Color WuiComputed_Color;

typedef struct WuiArraySlice_____WuiAnyView {
  struct WuiAnyView **head;
  uintptr_t len;
} WuiArraySlice_____WuiAnyView;

typedef struct WuiArrayVTable_____WuiAnyView {
  void (*drop)(void*);
  struct WuiArraySlice_____WuiAnyView (*slice)(const void*);
} WuiArrayVTable_____WuiAnyView;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_____WuiAnyView {
  NonNull data;
  struct WuiArrayVTable_____WuiAnyView vtable;
} WuiArray_____WuiAnyView;

typedef struct WuiFixedContainer {
  struct WuiLayout *layout;
  struct WuiArray_____WuiAnyView contents;
} WuiFixedContainer;

typedef struct WuiContainer {
  struct WuiLayout *layout;
  struct WuiAnyViews *contents;
} WuiContainer;

typedef struct WuiSize {
  float width;
  float height;
} WuiSize;

typedef struct WuiHorizontalGuide {
  enum WuiHorizontalAlignment alignment;
  float value;
} WuiHorizontalGuide;

typedef struct WuiArraySlice_WuiHorizontalGuide {
  struct WuiHorizontalGuide *head;
  uintptr_t len;
} WuiArraySlice_WuiHorizontalGuide;

typedef struct WuiArrayVTable_WuiHorizontalGuide {
  void (*drop)(void*);
  struct WuiArraySlice_WuiHorizontalGuide (*slice)(const void*);
} WuiArrayVTable_WuiHorizontalGuide;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiHorizontalGuide {
  NonNull data;
  struct WuiArrayVTable_WuiHorizontalGuide vtable;
} WuiArray_WuiHorizontalGuide;

typedef struct WuiVerticalGuide {
  enum WuiVerticalAlignment alignment;
  float value;
} WuiVerticalGuide;

typedef struct WuiArraySlice_WuiVerticalGuide {
  struct WuiVerticalGuide *head;
  uintptr_t len;
} WuiArraySlice_WuiVerticalGuide;

typedef struct WuiArrayVTable_WuiVerticalGuide {
  void (*drop)(void*);
  struct WuiArraySlice_WuiVerticalGuide (*slice)(const void*);
} WuiArrayVTable_WuiVerticalGuide;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiVerticalGuide {
  NonNull data;
  struct WuiArrayVTable_WuiVerticalGuide vtable;
} WuiArray_WuiVerticalGuide;

typedef struct WuiViewDimensions {
  struct WuiSize size;
  struct WuiArray_WuiHorizontalGuide horizontal_guides;
  struct WuiArray_WuiVerticalGuide vertical_guides;
} WuiViewDimensions;

typedef struct WuiProposalSize {
  float width;
  float height;
} WuiProposalSize;

/**
 * VTable for SubView operations.
 *
 * This structure contains function pointers that allow native code to implement
 * the SubView protocol. The native backend provides these callbacks to participate
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
   * Called when the WuiSubView is dropped.
   */
  void (*drop)(void *context);
} WuiSubViewVTable;

/**
 * FFI representation of a SubView proxy.
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
   * VTable containing measure and drop functions
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

typedef struct WuiArraySlice_WuiSubView {
  struct WuiSubView *head;
  uintptr_t len;
} WuiArraySlice_WuiSubView;

typedef struct WuiArrayVTable_WuiSubView {
  void (*drop)(void*);
  struct WuiArraySlice_WuiSubView (*slice)(const void*);
} WuiArrayVTable_WuiSubView;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiSubView {
  NonNull data;
  struct WuiArrayVTable_WuiSubView vtable;
} WuiArray_WuiSubView;

typedef struct WuiPoint {
  float x;
  float y;
} WuiPoint;

typedef struct WuiRect {
  struct WuiPoint origin;
  struct WuiSize size;
} WuiRect;

typedef struct WuiArraySlice_WuiRect {
  struct WuiRect *head;
  uintptr_t len;
} WuiArraySlice_WuiRect;

typedef struct WuiArrayVTable_WuiRect {
  void (*drop)(void*);
  struct WuiArraySlice_WuiRect (*slice)(const void*);
} WuiArrayVTable_WuiRect;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiRect {
  NonNull data;
  struct WuiArrayVTable_WuiRect vtable;
} WuiArray_WuiRect;

typedef struct WuiScrollView {
  enum WuiAxis axis;
  struct WuiAnyView *content;
} WuiScrollView;

typedef struct WuiButton {
  struct WuiAnyView *label;
  struct WuiAction *action;
  enum WuiButtonStyle style;
  WuiComputed_StyledStr *accessibility_label;
} WuiButton;

typedef struct WuiTextStyle {
  struct WuiFont *font;
  bool italic;
  bool underline;
  bool strikethrough;
  struct WuiColor *foreground;
  struct WuiColor *background;
} WuiTextStyle;

typedef struct WuiStyledChunk {
  struct WuiStr text;
  struct WuiTextStyle style;
} WuiStyledChunk;

typedef struct WuiArraySlice_WuiStyledChunk {
  struct WuiStyledChunk *head;
  uintptr_t len;
} WuiArraySlice_WuiStyledChunk;

typedef struct WuiArrayVTable_WuiStyledChunk {
  void (*drop)(void*);
  struct WuiArraySlice_WuiStyledChunk (*slice)(const void*);
} WuiArrayVTable_WuiStyledChunk;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiStyledChunk {
  NonNull data;
  struct WuiArrayVTable_WuiStyledChunk vtable;
} WuiArray_WuiStyledChunk;

typedef struct WuiStyledStr {
  struct WuiArray_WuiStyledChunk chunks;
} WuiStyledStr;

typedef struct Binding_Font WuiBinding_Font;

typedef struct Computed_Font WuiComputed_Font;

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

typedef struct Computed_ResolvedFont WuiComputed_ResolvedFont;

typedef struct Binding_StyledStr WuiBinding_StyledStr;

typedef struct Computed_Vec_ResolvedMenuItem WuiComputed_Vec_ResolvedMenuItem;

typedef struct WuiTextField {
  struct WuiAnyView *label;
  WuiBinding_StyledStr *value;
  struct WuiText prompt;
  enum WuiKeyboardType keyboard;
  WuiComputed_Vec_ResolvedMenuItem *selection_menu;
} WuiTextField;

typedef struct WuiToggle {
  struct WuiAnyView *label;
  WuiBinding_bool *toggle;
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

typedef struct Binding_f64 WuiBinding_f64;

typedef struct WuiSlider {
  struct WuiAnyView *label;
  struct WuiAnyView *min_value_label;
  struct WuiAnyView *max_value_label;
  struct WuiRange_f64 range;
  WuiBinding_f64 *value;
} WuiSlider;

typedef struct Binding_i32 WuiBinding_i32;

typedef struct Computed_i32 WuiComputed_i32;

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

typedef struct WuiStepper {
  WuiBinding_i32 *value;
  WuiComputed_i32 *step;
  struct WuiAnyView *label;
  struct WuiRange_i32 range;
} WuiStepper;

typedef struct WuiColorPicker {
  struct WuiAnyView *label;
  WuiBinding_Color *value;
  bool support_alpha;
  bool support_hdr;
} WuiColorPicker;

typedef struct Computed_Vec_PickerItem_Id WuiComputed_Vec_PickerItem_Id;

typedef struct Binding_Id WuiBinding_Id;

typedef struct WuiPicker {
  WuiComputed_Vec_PickerItem_Id *items;
  WuiBinding_Id *selection;
  enum WuiPickerStyle style;
} WuiPicker;

typedef struct Binding_Secure WuiBinding_Secure;

typedef struct WuiSecureField {
  struct WuiAnyView *label;
  WuiBinding_Secure *value;
} WuiSecureField;

typedef struct Binding_DateTime WuiBinding_DateTime;

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

typedef struct WuiDatePicker {
  struct WuiAnyView *label;
  WuiBinding_DateTime *value;
  struct WuiRange_WuiDateTime range;
  enum WuiDatePickerType ty;
} WuiDatePicker;

typedef struct Binding_Vec_Date WuiBinding_Vec_Date;

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

typedef struct Computed_Vec_Date WuiComputed_Vec_Date;

typedef struct WuiMultiDatePicker {
  struct WuiAnyView *label;
  WuiBinding_Vec_Date *value;
  struct WuiRange_WuiDate range;
  WuiComputed_Vec_Date *decorated;
} WuiMultiDatePicker;

typedef struct Binding_Str WuiBinding_Str;

typedef struct WuiNavigationSearch {
  WuiBinding_Str *text;
  struct WuiAnyView *prompt;
} WuiNavigationSearch;

typedef struct WuiBar {
  struct WuiAnyView *title;
  struct WuiAnyView *leading;
  struct WuiAnyView *trailing;
  struct WuiNavigationSearch *search;
  WuiComputed_Color *color;
  WuiComputed_bool *hidden;
  enum WuiNavigationTitleDisplayMode display_mode;
} WuiBar;

typedef struct WuiNavigationView {
  struct WuiBar bar;
  struct WuiAnyView *content;
} WuiNavigationView;

/**
 * FFI struct for NavigationStack<(),()>
 */
typedef struct WuiNavigationStack {
  /**
   * The root view of the navigation stack.
   */
  struct WuiAnyView *root;
  /**
   * Transition style used for push/pop operations.
   */
  enum WuiNavigationTransition transition;
} WuiNavigationStack;

typedef struct WuiNavigationSplitDetail {
  uint8_t _private[0];
} WuiNavigationSplitDetail;

typedef struct WuiId {
  /**
   * The inner integer value of the ID.
   */
  int32_t inner;
} WuiId;

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
   * The currently selected detail identifier encoded as i32 (0 means no selection).
   */
  WuiBinding_i32 *selection;
  /**
   * Resolver handle for building detail content from a selected id.
   */
  struct WuiNavigationSplitDetail *detail;
  /**
   * Preferred sidebar width in logical points.
   */
  float sidebar_width;
} WuiNavigationSplitLayout;

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
} WuiTab;

typedef struct WuiArraySlice_WuiTab {
  struct WuiTab *head;
  uintptr_t len;
} WuiArraySlice_WuiTab;

typedef struct WuiArrayVTable_WuiTab {
  void (*drop)(void*);
  struct WuiArraySlice_WuiTab (*slice)(const void*);
} WuiArrayVTable_WuiTab;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiTab {
  NonNull data;
  struct WuiArrayVTable_WuiTab vtable;
} WuiArray_WuiTab;

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
   * Position of the tab bar (top or bottom).
   */
  enum WuiTabPosition position;
} WuiTabs;

/**
 * FFI-compatible navigation controller that bridges native push/pop callbacks to Rust.
 *
 * Native backends create this controller with callback function pointers, then install
 * it into the environment. When Rust views call `NavigationController::push()` or `pop()`,
 * those calls are forwarded to the native callbacks.
 */
typedef struct WuiNavigationController {
  /**
   * Opaque data pointer passed to all callbacks (typically a pointer to native controller).
   */
  void *data;
  /**
   * Callback invoked when a view is pushed onto the navigation stack.
   */
  void (*push)(void*, struct WuiNavigationView);
  /**
   * Callback invoked when popping the top view from the navigation stack.
   */
  void (*pop)(void*);
  /**
   * Callback invoked when the controller is dropped (for cleanup).
   */
  void (*drop)(void*);
} WuiNavigationController;

typedef struct Computed_Str WuiComputed_Str;

typedef struct Computed_f64 WuiComputed_f64;

typedef struct Binding_Volume WuiBinding_Volume;

typedef struct Binding_f32 WuiBinding_f32;

/**
 * FFI representation of a video event.
 */
typedef struct WuiVideoEvent {
  enum WuiVideoEventType event_type;
  struct WuiStr error_message;
  uint32_t buffered_ms;
  float av_drift_ms;
  uint64_t dropped_video_frames;
  bool picture_in_picture_active;
} WuiVideoEvent;

/**
 * A C-compatible function wrapper that can be called multiple times.
 *
 * This structure wraps a Rust `Fn` closure to allow it to be passed across
 * the FFI boundary while maintaining proper memory management.
 */
typedef struct WuiFn_WuiVideoEvent {
  void *data;
  void (*call)(const void*, struct WuiVideoEvent);
  void (*drop)(void*);
} WuiFn_WuiVideoEvent;

/**
 * FFI representation of the raw Video component (no native controls).
 */
typedef struct WuiVideo {
  /**
   * The video source URL as a string (reactive).
   * Swift expects WuiStr, so we convert Url -> Str.
   */
  WuiComputed_Str *source;
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
   * Preferred playback duration in seconds, or `-1.0` when unknown.
   */
  WuiComputed_f64 *duration_seconds;
  /**
   * Whether the active queue has a next item.
   */
  WuiBinding_bool *has_next;
  /**
   * Whether the active queue has a previous item.
   */
  WuiBinding_bool *has_previous;
  /**
   * The volume of the video.
   */
  WuiBinding_Volume *volume;
  /**
   * Playback speed (1.0 = normal speed).
   */
  WuiBinding_f32 *playback_rate;
  /**
   * Whether native playback should preserve pitch when rate changes.
   */
  WuiBinding_bool *preserve_pitch;
  /**
   * The aspect ratio mode for video playback.
   */
  enum WuiAspectRatio aspect_ratio;
  /**
   * Whether the video should loop when it ends.
   */
  bool loops;
  /**
   * The event handler for video events.
   */
  struct WuiFn_WuiVideoEvent on_event;
} WuiVideo;

/**
 * FFI representation of the VideoPlayer component (with native controls).
 */
typedef struct WuiVideoPlayer {
  /**
   * The video source URL as a string (reactive).
   * Swift expects WuiStr, so we convert Url -> Str.
   */
  WuiComputed_Str *source;
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
   * Preferred playback duration in seconds, or `-1.0` when unknown.
   */
  WuiComputed_f64 *duration_seconds;
  /**
   * Whether the active queue has a next item.
   */
  WuiBinding_bool *has_next;
  /**
   * Whether the active queue has a previous item.
   */
  WuiBinding_bool *has_previous;
  /**
   * The volume of the video player.
   */
  WuiBinding_Volume *volume;
  /**
   * Playback speed (1.0 = normal speed).
   */
  WuiBinding_f32 *playback_rate;
  /**
   * Whether native playback should preserve pitch when rate changes.
   */
  WuiBinding_bool *preserve_pitch;
  /**
   * The aspect ratio mode for video playback.
   */
  enum WuiAspectRatio aspect_ratio;
  /**
   * Whether to show native playback controls.
   */
  bool show_controls;
  /**
   * The event handler for the video player.
   */
  struct WuiFn_WuiVideoEvent on_event;
} WuiVideoPlayer;

/**
 * FFI representation of a Video source for Computed signals.
 */
typedef struct WuiComputedVideo {
  /**
   * The URL of the video source.
   */
  struct WuiStr url;
} WuiComputedVideo;

typedef struct Computed_Video WuiComputed_Video;

/**
 * FFI representation of a list item.
 *
 * `section_label` and `section_footer` are owned by the consumer — when
 * they're empty the item carries no section break, otherwise the item opens
 * a new logical section visible to the renderer (UITableView sections,
 * NSTableView group rows, Material list groups, ...). Both fields are
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
} WuiList;

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

typedef struct WuiArraySlice_WuiTableColumn {
  struct WuiTableColumn *head;
  uintptr_t len;
} WuiArraySlice_WuiTableColumn;

typedef struct WuiArrayVTable_WuiTableColumn {
  void (*drop)(void*);
  struct WuiArraySlice_WuiTableColumn (*slice)(const void*);
} WuiArrayVTable_WuiTableColumn;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiTableColumn {
  NonNull data;
  struct WuiArrayVTable_WuiTableColumn vtable;
} WuiArray_WuiTableColumn;

typedef struct Computed_Vec_TableColumn WuiComputed_Vec_TableColumn;

typedef struct WuiTable {
  WuiComputed_Vec_TableColumn *columns;
} WuiTable;

typedef struct WuiProgress {
  struct WuiAnyView *label;
  struct WuiAnyView *value_label;
  WuiComputed_f64 *value;
  enum WuiProgressStyle style;
} WuiProgress;

/**
 * FFI representation of a GpuSurface view.
 *
 * This struct is passed to the native backend when rendering the view tree.
 * The native backend should call `waterui_gpu_surface_init` to initialize
 * the GPU resources, then `waterui_gpu_surface_render` each frame.
 */
typedef struct WuiGpuSurface {
  /**
   * Opaque pointer to the boxed GpuSurface.
   * This is consumed during init and should not be used after.
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
  /**
   * Final dynamic range preference resolved by Rust (`GpuSurface` + env policy).
   */
  bool resolved_prefers_hdr;
} WuiGpuSurfaceHdrPreference;

/**
 * Result returned by a GpuSurface render invocation.
 */
typedef struct WuiGpuSurfaceRenderResult {
  /**
   * Whether rendering succeeded.
   */
  bool ok;
  /**
   * Whether another frame should be scheduled immediately.
   */
  bool needs_redraw;
} WuiGpuSurfaceRenderResult;

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
 * FFI-safe combined input state for a GpuSurface.
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
 * FFI representation of a WebView event.
 */
typedef struct WuiWebViewEvent {
  /**
   * The type of event.
   */
  enum WuiWebViewEventType event_type;
  /**
   * URL associated with the event (for WillNavigate, SslError, Error, Redirect from).
   */
  struct WuiStr url;
  /**
   * Second URL (for Redirect to).
   */
  struct WuiStr url2;
  /**
   * Error/message string (for SslError, Error).
   */
  struct WuiStr message;
  /**
   * Loading progress (0.0 to 1.0, for Loading event).
   */
  float progress;
  /**
   * Whether can navigate back (for StateChanged).
   */
  bool can_go_back;
  /**
   * Whether can navigate forward (for StateChanged).
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
 * Callback for JavaScript execution results.
 */
typedef struct WuiJsCallback {
  /**
   * Opaque pointer to callback data.
   */
  void *data;
  /**
   * Function to call with result. success=true means result is the value, false means error.
   */
  void (*call)(void *data, bool success, struct WuiStr result);
} WuiJsCallback;

/**
 * Message payload emitted from JavaScript to a native-registered handler.
 *
 * `payload_base64` is base64-encoded bytes from JavaScript.
 * `reply` must be called exactly once for request/response semantics.
 */
typedef struct WuiWebViewMessage {
  struct WuiStr payload_base64;
  struct WuiJsCallback reply;
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
 * FFI representation of a WebView handle with function pointers.
 *
 * Native backends create this struct with function pointers to their implementation.
 */
typedef struct WuiWebViewHandle {
  /**
   * Opaque pointer to native WebView wrapper.
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
   * Enable or disable following redirects.
   */
  void (*set_redirects_enabled)(void*, bool);
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
   * Sets a cookie for the web view. The string is a Set-Cookie header value.
   */
  void (*set_cookie)(void*, struct WuiStr);
  /**
   * Gets cookies as newline-separated Set-Cookie strings.
   */
  struct WuiStr (*get_cookies)(const void*);
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
 * Type for the native function that creates a new WebView.
 */
typedef struct WuiWebViewHandle (*WuiCreateWebViewFn)(void);

typedef struct Computed_Region WuiComputed_Region;

typedef struct Computed_Vec_Annotation WuiComputed_Vec_Annotation;

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

typedef struct WuiArraySlice_WuiAnnotation {
  struct WuiAnnotation *head;
  uintptr_t len;
} WuiArraySlice_WuiAnnotation;

typedef struct WuiArrayVTable_WuiAnnotation {
  void (*drop)(void*);
  struct WuiArraySlice_WuiAnnotation (*slice)(const void*);
} WuiArrayVTable_WuiAnnotation;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiAnnotation {
  NonNull data;
  struct WuiArrayVTable_WuiAnnotation vtable;
} WuiArray_WuiAnnotation;

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
  uint32_t width;
  uint32_t height;
} WuiOutputSize_Fixed_Body;

typedef struct WuiOutputSize_Scale_Body {
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
 * FFI representation of a ViewEffect view.
 *
 * This struct is passed to the native backend when rendering the view tree.
 * The native backend should:
 * 1. Create capture and output layers
 * 2. Call `waterui_view_effect_init` to initialize GPU resources
 * 3. Render the child view to the capture layer
 * 4. Call `waterui_view_effect_render` when rendering is scheduled
 */
typedef struct WuiViewEffect {
  /**
   * The child view to capture (pointer to WuiAnyView).
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
 * Result returned by a ViewEffect render invocation.
 */
typedef struct WuiViewEffectRenderResult {
  /**
   * Whether rendering succeeded.
   */
  bool success;
  /**
   * Whether another frame should be scheduled immediately.
   */
  bool needs_redraw;
} WuiViewEffectRenderResult;

/**
 * Native drop callback type for external resources.
 *
 * Android: used to release an acquired `AHardwareBuffer*` without Rust linking to API-26+ symbols.
 */
typedef void (*WuiExternalDropFn)(void *user_data);

/**
 * FFI representation of a Metadata<AppliedFilter>.
 */
typedef struct WuiAppliedFilter {
  /**
   * The child view to capture (pointer to WuiAnyView).
   */
  struct WuiAnyView *content;
  /**
   * Opaque pointer to the boxed AppliedFilter.
   * This is consumed during init and should not be used after.
   */
  void *filter;
} WuiAppliedFilter;

/**
 * Result of a filter render operation.
 */
typedef struct WuiAppliedFilterRenderResult {
  /**
   * Whether rendering succeeded.
   */
  bool success;
  /**
   * Whether another frame is needed (animation in progress).
   * Only valid if `success` is true.
   */
  bool needs_redraw;
} WuiAppliedFilterRenderResult;

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
 * FFI representation of `FilteredView<Blur>`.
 */
typedef struct WuiFilteredBlur {
  /**
   * Child content pointer consumed by native backend.
   */
  struct WuiAnyView *content;
  /**
   * Reactive blur radius pointer.
   */
  WuiComputed_f32 *radius;
} WuiFilteredBlur;

/**
 * Callback for returning rendered RGBA data to Rust.
 */
typedef struct ViewRenderCallback {
  /**
   * Opaque data pointer passed to the callback.
   */
  void *data;
  /**
   * Callback function.
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
typedef void (*ViewRenderFn)(void *view, struct WuiSize size, struct ViewRenderCallback callback);

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

typedef struct WuiResolvedGradientStop {
  float position;
  struct WuiResolvedColor color;
} WuiResolvedGradientStop;

typedef struct WuiArraySlice_WuiResolvedGradientStop {
  struct WuiResolvedGradientStop *head;
  uintptr_t len;
} WuiArraySlice_WuiResolvedGradientStop;

typedef struct WuiArrayVTable_WuiResolvedGradientStop {
  void (*drop)(void*);
  struct WuiArraySlice_WuiResolvedGradientStop (*slice)(const void*);
} WuiArrayVTable_WuiResolvedGradientStop;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiResolvedGradientStop {
  NonNull data;
  struct WuiArrayVTable_WuiResolvedGradientStop vtable;
} WuiArray_WuiResolvedGradientStop;

typedef struct WuiResolvedGradient {
  enum WuiGradientType gradient_type;
  struct WuiArray_WuiResolvedGradientStop stops;
  float start_x;
  float start_y;
  float end_x;
  float end_y;
  float start_value;
  float end_value;
} WuiResolvedGradient;

typedef struct Computed_Id WuiComputed_Id;

typedef struct Binding_AnyView WuiBinding_AnyView;

typedef struct Computed_AnyView WuiComputed_AnyView;

typedef struct Binding_Date WuiBinding_Date;

typedef struct Computed_Date WuiComputed_Date;

typedef struct Computed_DateTime WuiComputed_DateTime;

typedef struct WuiArraySlice_WuiDate {
  struct WuiDate *head;
  uintptr_t len;
} WuiArraySlice_WuiDate;

typedef struct WuiArrayVTable_WuiDate {
  void (*drop)(void*);
  struct WuiArraySlice_WuiDate (*slice)(const void*);
} WuiArrayVTable_WuiDate;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiDate {
  NonNull data;
  struct WuiArrayVTable_WuiDate vtable;
} WuiArray_WuiDate;

typedef struct WuiPickerItem {
  struct WuiId tag;
  struct WuiText content;
} WuiPickerItem;

typedef struct WuiArraySlice_WuiPickerItem {
  struct WuiPickerItem *head;
  uintptr_t len;
} WuiArraySlice_WuiPickerItem;

typedef struct WuiArrayVTable_WuiPickerItem {
  void (*drop)(void*);
  struct WuiArraySlice_WuiPickerItem (*slice)(const void*);
} WuiArrayVTable_WuiPickerItem;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiPickerItem {
  NonNull data;
  struct WuiArrayVTable_WuiPickerItem vtable;
} WuiArray_WuiPickerItem;

typedef struct WuiShapeKind {
  int32_t tag;
  float top_left;
  float top_right;
  float bottom_right;
  float bottom_left;
} WuiShapeKind;

typedef struct WuiResolvedShape {
  struct WuiShapeKind kind;
  struct WuiArray_WuiPathCommand commands;
  struct WuiResolvedColor fill;
} WuiResolvedShape;

typedef struct Computed_ColorScheme WuiComputed_ColorScheme;

typedef struct WuiArraySlice_WuiId {
  struct WuiId *head;
  uintptr_t len;
} WuiArraySlice_WuiId;

typedef struct WuiArrayVTable_WuiId {
  void (*drop)(void*);
  struct WuiArraySlice_WuiId (*slice)(const void*);
} WuiArrayVTable_WuiId;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiId {
  NonNull data;
  struct WuiArrayVTable_WuiId vtable;
} WuiArray_WuiId;

typedef struct Computed_AnyViews_AnyView WuiComputed_AnyViews_AnyView;

typedef struct Binding_WindowState WuiBinding_WindowState;

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
  struct WuiColor *color;
} WuiWindowBackground_Color_Body;

typedef struct WuiWindowBackground {
  WuiWindowBackground_Tag tag;
  union {
    WuiWindowBackground_Color_Body color;
  };
} WuiWindowBackground;

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
} WuiWindow;

/**
 * Type alias for the native window show function.
 *
 * This function is called by Rust when a `Window` view needs to be shown.
 * The native implementation should create and display the window.
 * Native code should use the global environment to render the window content.
 *
 * # Parameters
 * - `WuiWindow`: The window configuration to show
 */
typedef void (*WindowShowFn)(struct WuiWindow);

typedef struct WuiArraySlice_WuiWindow {
  struct WuiWindow *head;
  uintptr_t len;
} WuiArraySlice_WuiWindow;

typedef struct WuiArrayVTable_WuiWindow {
  void (*drop)(void*);
  struct WuiArraySlice_WuiWindow (*slice)(const void*);
} WuiArrayVTable_WuiWindow;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
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
  WuiComputed_MenuItems *menu_bar;
  /**
   * The application environment containing injected services.
   * Returned to native for use during rendering.
   */
  struct WuiEnv *env;
} WuiApp;





































/**
 * Type ID for `FilteredView<Blur>`, used by native backends to intercept
 * before `body` expansion.
 */
struct WuiTypeId waterui_filtered_blur_id(void);

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
 * Creates a new environment instance
 */
struct WuiEnv *waterui_env_new(void);

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

struct WuiAnyView *waterui_empty_anyview(void);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_env_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataEnv waterui_force_as_metadata_env(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_secure_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataSecure waterui_force_as_metadata_secure(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_standard_dynamic_range_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataStandardDynamicRange waterui_force_as_metadata_standard_dynamic_range(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_high_dynamic_range_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataHighDynamicRange waterui_force_as_metadata_high_dynamic_range(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_gesture_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataGesture waterui_force_as_metadata_gesture(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_lifecycle_hook_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataLifeCycleHook waterui_force_as_metadata_lifecycle_hook(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_on_event_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataOnEvent waterui_force_as_metadata_on_event(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_cursor_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataCursor waterui_force_as_metadata_cursor(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_shadow_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataShadow waterui_force_as_metadata_shadow(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_border_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataBorder waterui_force_as_metadata_border(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_scale_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataScale waterui_force_as_metadata_scale(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_rotation_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataRotation waterui_force_as_metadata_rotation(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_offset_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataOffset waterui_force_as_metadata_offset(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_opacity_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataOpacity waterui_force_as_metadata_opacity(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_focused_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataFocused waterui_force_as_metadata_focused(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_ignore_safe_area_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataIgnoreSafeArea waterui_force_as_metadata_ignore_safe_area(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_retain_id(void);

/**
 * Force-casts an AnyView to this metadata type
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
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_clip_shape_id(void);

/**
 * Force-casts an AnyView to this metadata type
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
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_context_menu_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataContextMenu waterui_force_as_metadata_context_menu(struct WuiAnyView *view);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiArray_WuiMenuItem waterui_read_computed_menu_items(const WuiComputed_MenuItems *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_menu_items(const WuiComputed_MenuItems *computed,
                                                          struct WuiWatcher_MenuItems *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_menu_items(WuiComputed_MenuItems *computed);

/**
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_MenuItems *waterui_clone_computed_menu_items(const WuiComputed_MenuItems *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_MenuItems *waterui_new_watcher_menu_items(void *data,
                                                            void (*call)(void*,
                                                                         struct WuiArray_WuiMenuItem,
                                                                         struct WuiWatcherMetadata*),
                                                            void (*drop)(void*));

struct WuiMenu waterui_force_as_menu(struct WuiAnyView *view);

struct WuiTypeId waterui_menu_id(void);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_draggable_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataDraggable waterui_force_as_metadata_draggable(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_drop_destination_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataDropDestination waterui_force_as_metadata_drop_destination(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_ignorable_metadata_material_background_id(void);

/**
 * Force-casts an AnyView to this ignorable metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains an `IgnorableMetadata<$ty>`.
 */
struct WuiIgnorableMetadataMaterialBackground waterui_force_as_ignorable_metadata_material_background(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_hittable_id(void);

/**
 * Force-casts an AnyView to this metadata type
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
void waterui_call_action(struct WuiAction *action, const struct WuiEnv *env);

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
void waterui_call_index_action(struct WuiIndexAction *action,
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
void waterui_call_move_action(struct WuiMoveAction *action,
                              const struct WuiEnv *env,
                              uintptr_t from_index,
                              uintptr_t to_index);

/**
 * Extracts animation metadata from a watcher context.
 *
 * # Safety
 * The metadata pointer must be valid and point to a properly initialized metadata object.
 */
struct WuiAnimation waterui_get_animation(const struct WuiWatcherMetadata *metadata);

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
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_ResolvedColor *waterui_clone_computed_resolved_color(const WuiComputed_ResolvedColor *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
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
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiColor *waterui_read_computed_color(const WuiComputed_Color *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_color(const WuiComputed_Color *computed,
                                                     struct WuiWatcher_Color *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_color(WuiComputed_Color *computed);

/**
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_Color *waterui_clone_computed_color(const WuiComputed_Color *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_Color *waterui_new_watcher_color(void *data,
                                                   void (*call)(void*,
                                                                struct WuiColor*,
                                                                struct WuiWatcherMetadata*),
                                                   void (*drop)(void*));

struct WuiResolvedColor waterui_force_as_resolved_color(struct WuiAnyView *view);

struct WuiTypeId waterui_resolved_color_id(void);

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

struct WuiStr waterui_force_as_plain(struct WuiAnyView *view);

struct WuiTypeId waterui_plain_id(void);

/**
 * Returns the type ID for empty views as a 128-bit value.
 */
struct WuiTypeId waterui_empty_id(void);

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

struct WuiFixedContainer waterui_force_as_fixed_container(struct WuiAnyView *view);

struct WuiTypeId waterui_fixed_container_id(void);

struct WuiContainer waterui_force_as_layout_container(struct WuiAnyView *view);

struct WuiTypeId waterui_layout_container_id(void);

void waterui_drop_view_dimensions(struct WuiViewDimensions dimensions);

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

struct WuiSize waterui_layout_size_that_fits(struct WuiLayout *layout,
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

enum WuiLazyStackAxis waterui_layout_lazy_stack_axis(struct WuiLayout *layout);

float waterui_layout_lazy_stack_spacing(struct WuiLayout *layout);

enum WuiHorizontalAlignment waterui_layout_lazy_stack_horizontal_alignment(struct WuiLayout *layout);

enum WuiVerticalAlignment waterui_layout_lazy_stack_vertical_alignment(struct WuiLayout *layout);

struct WuiScrollView waterui_force_as_scroll_view(struct WuiAnyView *view);

struct WuiTypeId waterui_scroll_view_id(void);

struct WuiButton waterui_force_as_button(struct WuiAnyView *view);

struct WuiTypeId waterui_button_id(void);

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
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_StyledStr *waterui_clone_computed_styled_str(const WuiComputed_StyledStr *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
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
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_HorizontalAlignment *waterui_clone_computed_horizontal_alignment(const WuiComputed_HorizontalAlignment *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_HorizontalAlignment *waterui_new_watcher_horizontal_alignment(void *data,
                                                                                void (*call)(void*,
                                                                                             enum WuiHorizontalAlignment,
                                                                                             struct WuiWatcherMetadata*),
                                                                                void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiFont *waterui_read_binding_font(const WuiBinding_Font *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_font(WuiBinding_Font *binding, struct WuiFont *value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_font(const WuiBinding_Font *binding,
                                                   struct WuiWatcher_Font *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_font(WuiBinding_Font *binding);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiFont *waterui_read_computed_font(const WuiComputed_Font *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_font(const WuiComputed_Font *computed,
                                                    struct WuiWatcher_Font *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_font(WuiComputed_Font *computed);

/**
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_Font *waterui_clone_computed_font(const WuiComputed_Font *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_Font *waterui_new_watcher_font(void *data,
                                                 void (*call)(void*,
                                                              struct WuiFont*,
                                                              struct WuiWatcherMetadata*),
                                                 void (*drop)(void*));

struct WuiText waterui_force_as_text(struct WuiAnyView *view);

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
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_ResolvedFont *waterui_clone_computed_resolved_font(const WuiComputed_ResolvedFont *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
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
 * Creates a new WuiResolvedFont with a properly initialized empty family string.
 *
 * This function is needed for native code (Android JNI) to create WuiResolvedFont
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

struct WuiTextField waterui_force_as_text_field(struct WuiAnyView *view);

struct WuiTypeId waterui_text_field_id(void);

struct WuiToggle waterui_force_as_toggle(struct WuiAnyView *view);

struct WuiTypeId waterui_toggle_id(void);

struct WuiSlider waterui_force_as_slider(struct WuiAnyView *view);

struct WuiTypeId waterui_slider_id(void);

struct WuiStepper waterui_force_as_stepper(struct WuiAnyView *view);

struct WuiTypeId waterui_stepper_id(void);

struct WuiColorPicker waterui_force_as_color_picker(struct WuiAnyView *view);

struct WuiTypeId waterui_color_picker_id(void);

struct WuiPicker waterui_force_as_picker(struct WuiAnyView *view);

struct WuiTypeId waterui_picker_id(void);

struct WuiSecureField waterui_force_as_secure_field(struct WuiAnyView *view);

struct WuiTypeId waterui_secure_field_id(void);

struct WuiDatePicker waterui_force_as_date_picker(struct WuiAnyView *view);

struct WuiTypeId waterui_date_picker_id(void);

struct WuiMultiDatePicker waterui_force_as_multi_date_picker(struct WuiAnyView *view);

struct WuiTypeId waterui_multi_date_picker_id(void);

struct WuiNavigationView waterui_force_as_navigation_view(struct WuiAnyView *view);

struct WuiTypeId waterui_navigation_view_id(void);

struct WuiNavigationStack waterui_force_as_navigation_stack(struct WuiAnyView *view);

struct WuiTypeId waterui_navigation_stack_id(void);

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
                                                                 struct WuiId selected);

struct WuiNavigationSplitLayout waterui_force_as_split_navigation_container(struct WuiAnyView *view);

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
struct WuiNavigationView waterui_tab_content(struct WuiTabContent *handler);

struct WuiTabs waterui_force_as_tabs(struct WuiAnyView *view);

struct WuiTypeId waterui_tabs_id(void);

/**
 * Creates a new navigation controller from native callbacks.
 *
 * # Arguments
 *
 * * `data` - Opaque pointer passed to all callbacks (typically pointer to native controller)
 * * `push` - Callback invoked when pushing a view onto the navigation stack
 * * `pop` - Callback invoked when popping the top view
 * * `drop` - Callback invoked when the controller is destroyed (for cleanup)
 *
 * # Safety
 *
 * - `data` must remain valid for the lifetime of the returned controller
 * - All callback function pointers must be valid and safe to call
 * - The `drop` callback must properly clean up resources associated with `data`
 */
struct WuiNavigationController *waterui_navigation_controller_new(void *data,
                                                                  void (*push)(void*,
                                                                               struct WuiNavigationView),
                                                                  void (*pop)(void*),
                                                                  void (*drop)(void*));

/**
 * Installs a navigation controller into the environment.
 *
 * After calling this function, views rendered with this environment can extract
 * the `NavigationController` and use it to push/pop navigation views.
 *
 * # Safety
 *
 * - `env` must be a valid pointer to a `WuiEnv`
 * - `controller` must be a valid pointer returned by `waterui_navigation_controller_new`
 * - `controller` is consumed by this function and must not be used afterward
 */
void waterui_env_install_navigation_controller(struct WuiEnv *env,
                                               struct WuiNavigationController *controller);

/**
 * Drops a navigation controller.
 *
 * # Safety
 *
 * - `controller` must be a valid pointer returned by `waterui_navigation_controller_new`
 * - `controller` must not have been previously dropped or consumed
 */
void waterui_drop_navigation_controller(struct WuiNavigationController *controller);

/**
 * Checks if a navigation controller is installed in the environment.
 *
 * Returns true if a NavigationController is available, false otherwise.
 * Use this to determine whether to show a back button in navigation views.
 *
 * # Safety
 *
 * - `env` must be a valid pointer to a `WuiEnv`
 */
bool waterui_env_has_navigation_controller(const struct WuiEnv *env);

/**
 * Pops the top view from the navigation stack.
 *
 * If no NavigationController is installed in the environment, this function does nothing.
 *
 * # Safety
 *
 * - `env` must be a valid pointer to a `WuiEnv`
 */
void waterui_navigation_pop(const struct WuiEnv *env);

struct WuiVideo waterui_force_as_video(struct WuiAnyView *view);

struct WuiTypeId waterui_video_id(void);

struct WuiVideoPlayer waterui_force_as_video_player(struct WuiAnyView *view);

struct WuiTypeId waterui_video_player_id(void);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiComputedVideo waterui_read_computed_video(const WuiComputed_Video *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_video(const WuiComputed_Video *computed,
                                                     struct WuiWatcher_Video *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_video(WuiComputed_Video *computed);

/**
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_Video *waterui_clone_computed_video(const WuiComputed_Video *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_Video *waterui_new_watcher_video(void *data,
                                                   void (*call)(void*,
                                                                struct WuiComputedVideo,
                                                                struct WuiWatcherMetadata*),
                                                   void (*drop)(void*));

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_dynamic(struct WuiDynamic *value);

struct WuiDynamic *waterui_force_as_dynamic(struct WuiAnyView *view);

struct WuiTypeId waterui_dynamic_id(void);

/**
 * Connects a watcher to a dynamic view.
 * # Safety
 * - The dynamic pointer must be valid.
 * - The watcher pointer will be consumed and freed when the Dynamic is dropped.
 */
void waterui_dynamic_connect(struct WuiDynamic *dynamic, struct WuiWatcher_AnyView *watcher);

struct WuiListItem waterui_force_as_list_item(struct WuiAnyView *view);

struct WuiTypeId waterui_list_item_id(void);

struct WuiList waterui_force_as_list(struct WuiAnyView *view);

struct WuiTypeId waterui_list_id(void);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiArray_WuiTableColumn waterui_read_computed_table_cols(const WuiComputed_Vec_TableColumn *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_table_cols(const WuiComputed_Vec_TableColumn *computed,
                                                          struct WuiWatcher_Vec_TableColumn *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_table_cols(WuiComputed_Vec_TableColumn *computed);

/**
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_Vec_TableColumn *waterui_clone_computed_table_cols(const WuiComputed_Vec_TableColumn *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_Vec_TableColumn *waterui_new_watcher_table_cols(void *data,
                                                                  void (*call)(void*,
                                                                               struct WuiArray_WuiTableColumn,
                                                                               struct WuiWatcherMetadata*),
                                                                  void (*drop)(void*));

struct WuiTable waterui_force_as_table(struct WuiAnyView *view);

struct WuiTypeId waterui_table_id(void);

struct WuiTableColumn waterui_force_as_table_column(struct WuiAnyView *view);

struct WuiTypeId waterui_table_column_id(void);

struct WuiProgress waterui_force_as_progress(struct WuiAnyView *view);

struct WuiTypeId waterui_progress_id(void);

struct WuiGpuSurface waterui_force_as_gpu_surface(struct WuiAnyView *view);

struct WuiTypeId waterui_gpu_surface_id(void);

/**
 * Returns the renderer-driven HDR preference for a `WuiGpuSurface`.
 *
 * This must be called before `waterui_gpu_surface_init` consumes the surface.
 *
 * # Safety
 *
 * - `surface` must be a valid pointer obtained from `waterui_force_as_gpu_surface`
 * - `surface` must not have been consumed by `waterui_gpu_surface_init`
 */
struct WuiGpuSurfaceHdrPreference waterui_gpu_surface_hdr_preference(const struct WuiGpuSurface *surface);

/**
 * Initialize a GpuSurface with a native layer.
 *
 * This function creates wgpu resources (Instance, Adapter, Device, Queue, Surface)
 * from the provided native layer and calls the user's `setup()` method.
 *
 * # Arguments
 *
 * * `surface` - Pointer to the WuiGpuSurface FFI struct (consumed)
 * * `layer` - Platform-specific layer pointer:
 *   - Apple: `CAMetalLayer*`
 *   - Android: `ANativeWindow*`
 * * `width` - Initial surface width in pixels
 * * `height` - Initial surface height in pixels
 *
 * # Returns
 *
 * Opaque pointer to the initialized state, or null on failure.
 *
 * # Safety
 *
 * - `surface` must be a valid pointer obtained from `waterui_force_as_gpu_surface`
 * - `layer` must be a valid platform-specific layer pointer
 * - The layer must remain valid for the lifetime of the returned state
 */
struct WuiGpuSurfaceState *waterui_gpu_surface_init(struct WuiGpuSurface *surface,
                                                    void *layer,
                                                    uint32_t width,
                                                    uint32_t height,
                                                    struct WuiEnv *env);

/**
 * Render a single frame.
 *
 * This function should be called when the surface is dirty (size/input/state changed)
 * and backend should schedule another frame when `needs_redraw` is true.
 *
 * # Arguments
 *
 * * `state` - Pointer to the initialized state from `waterui_gpu_surface_init`
 * * `width` - Current surface width in pixels (from layout)
 * * `height` - Current surface height in pixels (from layout)
 *
 * # Returns
 *
 * Render result containing success + redraw intent.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_gpu_surface_init`.
 */
struct WuiGpuSurfaceRenderResult waterui_gpu_surface_render(struct WuiGpuSurfaceState *state,
                                                            uint32_t width,
                                                            uint32_t height);

/**
 * Query whether the renderer currently requests another frame.
 *
 * This is a lightweight probe used by strict on-demand backends before they
 * schedule a render. It must not render or mutate GPU resources.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_gpu_surface_init`.
 */
bool waterui_gpu_surface_needs_redraw(struct WuiGpuSurfaceState *state);

/**
 * Render a single frame into an external texture.
 *
 * This is used for GPU-based view captures (e.g., filter pipelines) so a
 * GpuSurface can render directly into a provided texture.
 *
 * # Arguments
 *
 * * `state` - Pointer to the initialized state from `waterui_gpu_surface_init`
 * * `texture` - Pointer to a `wgpu::Texture` to render into
 * * `width` - Target width in pixels
 * * `height` - Target height in pixels
 *
 * # Returns
 *
 * `true` if rendering succeeded, `false` on error.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_gpu_surface_init`.
 * `texture` must be a valid pointer to a `wgpu::Texture` with RENDER_ATTACHMENT usage.
 */
bool waterui_gpu_surface_render_to_texture(struct WuiGpuSurfaceState *state,
                                           void *texture,
                                           uint32_t width,
                                           uint32_t height);

/**
 * Render a single frame into an external Metal texture (Apple only).
 *
 * # Safety
 * `state` must be valid, `texture` must point to a `MTLTexture`.
 */
bool waterui_gpu_surface_render_to_metal_texture(struct WuiGpuSurfaceState *state,
                                                 void *texture,
                                                 uint32_t width,
                                                 uint32_t height);

/**
 * Setup the GpuSurface and render the first frame.
 *
 * This function performs setup in the synchronous FFI render path,
 * then renders the first frame. Native code should call this before showing
 * the window to ensure all visible GpuSurfaces are ready.
 *
 * # Arguments
 *
 * * `state` - Pointer to initialized state from `waterui_gpu_surface_init`
 *
 * # Safety
 *
 * - `state` must be a valid pointer from `waterui_gpu_surface_init`
 */
bool waterui_gpu_surface_await_ready(struct WuiGpuSurfaceState *state);

/**
 * Clean up GPU resources.
 *
 * This function should be called when the GpuSurface view is destroyed.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_gpu_surface_init`,
 * and must not be used after this call.
 */
void waterui_gpu_surface_drop(struct WuiGpuSurfaceState *state);

/**
 * Update both pointer and gesture state for a GpuSurface.
 *
 * Native backends should prefer this API to minimize bridge calls.
 *
 * # Arguments
 *
 * * `state` - Pointer to the initialized state from `waterui_gpu_surface_init`
 * * `input` - Combined pointer + gesture snapshot
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_gpu_surface_init`.
 */
void waterui_gpu_surface_set_input(struct WuiGpuSurfaceState *state,
                                   struct WuiGpuSurfaceInput input);

struct WuiSystemIcon waterui_force_as_system_icon(struct WuiAnyView *view);

struct WuiTypeId waterui_system_icon_id(void);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_web_view(struct WuiWebView *value);

struct WuiWebView *waterui_force_as_webview(struct WuiAnyView *view);

struct WuiTypeId waterui_webview_id(void);

/**
 * Gets the native handle pointer from a WebView.
 *
 * Returns the opaque pointer to the native WebView wrapper (Swift/Kotlin).
 * This pointer can be used by native backends to access the underlying
 * WKWebView or Android WebView.
 *
 * # Safety
 *
 * - The caller must ensure that `webview` is a valid pointer to a `WuiWebView`.
 * - The WebView must have been created via the FFI WebViewController (i.e., the handle
 *   must be an `FfiWebViewHandle`). This is guaranteed when the native backend properly
 *   installed the WebViewController via `waterui_env_install_webview_controller`.
 */
void *waterui_webview_native_handle(struct WuiWebView *webview);

/**
 * Installs a WebViewController into the environment from a native factory function.
 *
 * Native backends call this during initialization to register their WebView factory.
 * The factory creates blank WebViews that can be navigated with `go_to()`.
 *
 * # Safety
 *
 * The caller must ensure that:
 * - `env` is a valid pointer to a `WuiEnv`
 * - `create_fn` is a valid function pointer that returns a properly initialized `WuiWebViewHandle`
 */
void waterui_env_install_webview_controller(struct WuiEnv *env, WuiCreateWebViewFn create_fn);

struct WuiMap waterui_force_as_map(struct WuiAnyView *view);

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
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_Region *waterui_clone_computed_region(const WuiComputed_Region *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
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
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_Vec_Annotation *waterui_clone_computed_annotations(const WuiComputed_Vec_Annotation *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_Vec_Annotation *waterui_new_watcher_annotations(void *data,
                                                                  void (*call)(void*,
                                                                               struct WuiArray_WuiAnnotation,
                                                                               struct WuiWatcherMetadata*),
                                                                  void (*drop)(void*));

struct WuiViewEffect waterui_force_as_view_effect(struct WuiAnyView *view);

struct WuiTypeId waterui_view_effect_id(void);

/**
 * Initialize a ViewEffect with native layers.
 *
 * This function creates wgpu resources for the effect rendering pipeline.
 *
 * # Arguments
 *
 * * `effect` - Pointer to the WuiViewEffect FFI struct (consumed)
 * * `output_layer` - Platform-specific layer for effect output:
 *   - Apple: `CAMetalLayer*`
 *   - Android: `ANativeWindow*`
 * * `input_width` - Width of the captured view in pixels
 * * `input_height` - Height of the captured view in pixels
 *
 * # Returns
 *
 * Opaque pointer to the initialized state, or null on failure.
 *
 * # Safety
 *
 * - `effect` must be a valid pointer obtained from `waterui_force_as_view_effect`
 * - `output_layer` must be a valid platform-specific layer pointer
 * - The layer must remain valid for the lifetime of the returned state
 */
struct WuiViewEffectState *waterui_view_effect_init(struct WuiViewEffect *effect,
                                                    void *output_layer,
                                                    uint32_t input_width,
                                                    uint32_t input_height);

/**
 * Provide input texture from child view.
 *
 * Call this before each scheduled `waterui_view_effect_render` to provide
 * the captured child view's texture.
 *
 * # Arguments
 *
 * * `state` - Pointer to initialized state
 * * `input_type` - Type of input being provided
 * * `input_handle` - Platform-specific handle:
 *   - `WgpuTexture`: Pointer to `wgpu::Texture`
 *   - `IOSurface`: `IOSurfaceRef` (Apple)
 *   - `AHardwareBuffer`: `AHardwareBuffer*` (Android)
 *   - `PixelData`: Pointer to pixel data
 * * `width` - Input width in pixels
 * * `height` - Input height in pixels
 *
 * # Safety
 *
 * - `state` must be a valid pointer from `waterui_view_effect_init`
 * - `input_handle` must be valid for the specified `input_type`
 */
bool waterui_view_effect_set_input(struct WuiViewEffectState *state,
                                   enum WuiInputType input_type,
                                   void *input_handle,
                                   uint32_t width,
                                   uint32_t height);

/**
 * Render the effect.
 *
 * This function applies the effect to the captured input and renders to the output.
 *
 * # Arguments
 *
 * * `state` - Pointer to initialized state
 *
 * # Returns
 *
 * Render result containing success + redraw intent.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_view_effect_init`.
 */
struct WuiViewEffectRenderResult waterui_view_effect_render(struct WuiViewEffectState *state);

/**
 * Get a pointer to the capture texture for the child view to render into.
 *
 * The native backend should render the child view to this texture, then call
 * `waterui_view_effect_render` to apply the effect.
 *
 * # Arguments
 *
 * * `state` - Pointer to initialized state
 *
 * # Returns
 *
 * Pointer to the capture wgpu::Texture.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_view_effect_init`.
 */
const void *waterui_view_effect_get_capture_texture(struct WuiViewEffectState *state);

/**
 * Clean up ViewEffect resources.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_view_effect_init`,
 * and must not be used after this call.
 */
void waterui_view_effect_drop(struct WuiViewEffectState *state);

/**
 * Check if the child view content is a GpuSurface.
 *
 * Returns `true` if the child is a GpuSurface, enabling the zero-copy optimization
 * where we can directly sample the GpuSurface's texture.
 *
 * # Safety
 *
 * `effect` must be a valid pointer.
 */
bool waterui_view_effect_child_is_gpu_surface(const struct WuiViewEffect *effect);

/**
 * Set input from an AHardwareBuffer (Android-specific zero-copy path).
 *
 * This function is called from JNI with a HardwareBuffer object.
 * The JNI layer extracts the AHardwareBuffer pointer and passes it here.
 *
 * # Arguments
 *
 * * `state` - Pointer to initialized ViewEffect state
 * * `ahb_ptr` - Pointer to AHardwareBuffer (from AHardwareBuffer_fromHardwareBuffer)
 * * `width` - Width in pixels
 * * `height` - Height in pixels
 *
 * # Safety
 *
 * - `state` must be a valid pointer from `waterui_view_effect_init`
 * - `ahb_ptr` must be a valid AHardwareBuffer pointer
 */
bool waterui_view_effect_set_input_ahardwarebuffer(struct WuiViewEffectState *state,
                                                   void *ahb_ptr,
                                                   WuiExternalDropFn drop_fn,
                                                   void *drop_data,
                                                   uint32_t width,
                                                   uint32_t height);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Returns the view's TypeId (guaranteed unique within a single binary).
 */
struct WuiTypeId waterui_metadata_applied_filter_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
struct WuiAppliedFilter waterui_force_as_metadata_applied_filter(struct WuiAnyView *view);

/**
 * Initialize an AppliedFilter with native layers.
 *
 * This function creates wgpu resources for the filter rendering pipeline.
 *
 * # Arguments
 *
 * * `filter_ffi` - Pointer to the WuiAppliedFilter FFI struct (consumed)
 * * `output_layer` - Platform-specific layer for filter output:
 *   - Apple: `CAMetalLayer*`
 *   - Android: `ANativeWindow*`
 * * `input_width` - Width of the captured view in pixels
 * * `input_height` - Height of the captured view in pixels
 *
 * # Returns
 *
 * Opaque pointer to the initialized state, or null on failure.
 *
 * # Safety
 *
 * - `filter_ffi` must be a valid pointer obtained from `waterui_force_as_metadata_applied_filter`
 * - `output_layer` must be a valid platform-specific layer pointer
 * - The layer must remain valid for the lifetime of the returned state
 */
struct WuiAppliedFilterState *waterui_applied_filter_init(struct WuiAppliedFilter *filter_ffi,
                                                          void *output_layer,
                                                          uint32_t input_width,
                                                          uint32_t input_height);

/**
 * Await filter setup to completion on the synchronous FFI path.
 *
 * # Arguments
 *
 * * `state` - Pointer to initialized state from `waterui_applied_filter_init`
 * Returns `true` when setup completes successfully.
 *
 * # Safety
 *
 * - `state` must be a valid pointer from `waterui_applied_filter_init`
 */
bool waterui_applied_filter_setup(struct WuiAppliedFilterState *state);

/**
 * Render the filter.
 *
 * This function applies the filter to the captured input and renders to the output.
 * Pass current width/height - resources are recreated if size changed.
 *
 * # Arguments
 *
 * * `state` - Pointer to initialized state
 * * `width` - Current width in pixels
 * * `height` - Current height in pixels
 *
 * # Returns
 *
 * A `WuiAppliedFilterRenderResult` with:
 * - `success`: whether rendering succeeded
 * - `needs_redraw`: whether another frame is needed (for animations)
 *
 * # Safety
 *
 * - `state` must be a valid pointer from `waterui_applied_filter_init`
 * - `waterui_applied_filter_setup` must have returned `true`
 */
struct WuiAppliedFilterRenderResult waterui_applied_filter_render(struct WuiAppliedFilterState *state,
                                                                  uint32_t width,
                                                                  uint32_t height);

/**
 * Snapshot reactive filter targets on the caller thread.
 *
 * This must be called before scheduling render work on background queues so
 * filter parameter reads stay on the UI/reactive thread.
 *
 * # Safety
 *
 * - `state` must be a valid pointer from `waterui_applied_filter_init`
 * - Caller must ensure no concurrent `waterui_applied_filter_render` is running
 */
bool waterui_applied_filter_sync_targets(struct WuiAppliedFilterState *state);

/**
 * Resolve the current output size from snapped filter state.
 *
 * Call this after `waterui_applied_filter_sync_targets` and before scheduling render work.
 *
 * # Safety
 *
 * - `state` must be a valid pointer from `waterui_applied_filter_init`
 */
struct WuiAppliedFilterOutputSize waterui_applied_filter_resolve_output_size(struct WuiAppliedFilterState *state,
                                                                             uint32_t input_width,
                                                                             uint32_t input_height);

/**
 * Poll whether the filter requires a new frame.
 *
 * This synchronizes reactive targets and returns the filter's redraw hint.
 * Native backends use this to keep on-demand loops responsive without
 * continuously rendering when nothing changed.
 *
 * # Safety
 *
 * - `state` must be a valid pointer from `waterui_applied_filter_init`.
 */
bool waterui_applied_filter_poll_redraw(struct WuiAppliedFilterState *state);

/**
 * Provide input texture from child view.
 *
 * Call this before each scheduled `waterui_applied_filter_render` to provide
 * the captured child view's texture.
 *
 * # Arguments
 *
 * * `state` - Pointer to initialized state
 * * `input_type` - Type of input being provided
 * * `input_handle` - Platform-specific handle:
 *   - `WgpuTexture`: Pointer to `wgpu::Texture`
 *   - `MetalTexture`: `MTLTexture*` (Apple)
 *   - `AHardwareBuffer`: `AHardwareBuffer*` (Android)
 *   - `PixelData`: Pointer to pixel data
 * * `width` - Input width in pixels
 * * `height` - Input height in pixels
 *
 * # Safety
 *
 * - `state` must be a valid pointer from `waterui_applied_filter_init`
 * - `input_handle` must be valid for the specified `input_type`
 */
bool waterui_applied_filter_set_input(struct WuiAppliedFilterState *state,
                                      enum WuiInputType input_type,
                                      void *input_handle,
                                      uint32_t width,
                                      uint32_t height);

/**
 * Set input from an AHardwareBuffer (Android-specific zero-copy path).
 *
 * This requires native to pass a drop callback that releases an acquired reference to the
 * AHardwareBuffer when wgpu is done using it (after GPU work completes).
 */
bool waterui_applied_filter_set_input_ahardwarebuffer(struct WuiAppliedFilterState *state,
                                                      void *ahb_ptr,
                                                      WuiExternalDropFn drop_fn,
                                                      void *drop_data,
                                                      uint32_t width,
                                                      uint32_t height);

/**
 * Prepare the capture texture for rendering.
 *
 * Ensures the capture texture matches the requested dimensions and returns
 * a pointer to the underlying wgpu texture for zero-copy rendering paths.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_applied_filter_init`.
 */
const void *waterui_applied_filter_prepare_capture(struct WuiAppliedFilterState *state,
                                                   uint32_t width,
                                                   uint32_t height);

/**
 * Get a pointer to the capture texture.
 *
 * The native backend should render the child view to this texture.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_applied_filter_init`.
 */
const void *waterui_applied_filter_get_capture_texture(struct WuiAppliedFilterState *state);

/**
 * Get a pointer to the Metal texture backing the capture texture (Apple only).
 *
 * This exposes the underlying MTLTexture so native code can render directly
 * into the wgpu capture texture without extra copies.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_applied_filter_init`.
 */
void *waterui_applied_filter_get_capture_metal_texture(struct WuiAppliedFilterState *state);

/**
 * Clean up AppliedFilter resources.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_applied_filter_init`,
 * and must not be used after this call.
 */
void waterui_applied_filter_drop(struct WuiAppliedFilterState *state);

/**
 * Force-casts `AnyView` to `FilteredView<Blur>`.
 *
 * # Safety
 * Caller must guarantee `view` points to a `FilteredView<Blur>`.
 */
struct WuiFilteredBlur waterui_force_as_filtered_blur(struct WuiAnyView *view);

/**
 * Expands filtered blur into the generic fallback `Metadata<AppliedFilter>` node.
 *
 * This consumes `content` and `radius`, rebuilding the fallback subtree exactly
 * as `FilteredView<Blur>::body()`.
 *
 * # Safety
 * `content` must be a valid `WuiAnyView*`, `radius` must be a valid `WuiComputed<f32>*`.
 */
struct WuiAnyView *waterui_filtered_blur_expand(struct WuiAnyView *content,
                                                WuiComputed_f32 *radius);

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
 * - `render_fn` is a valid function pointer to the native view renderer
 */
void waterui_env_install_view_renderer(struct WuiEnv *env, ViewRenderFn render_fn);

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
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_CursorStyle *waterui_clone_computed_cursor_style(const WuiComputed_CursorStyle *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
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
 * * `draggable` must be a valid pointer to a WuiDraggable.
 */
struct WuiDragData waterui_draggable_get_data(const struct WuiDraggable *draggable);

/**
 * Drops a draggable.
 *
 * # Safety
 *
 * * `draggable` must be a valid pointer to a WuiDraggable.
 */
void waterui_drop_draggable(struct WuiDraggable *draggable);

/**
 * Calls the drop handler with the given data.
 *
 * # Safety
 *
 * * `handler` must be a valid pointer to a WuiDropDestination.
 * * `env` must be a valid pointer to a WuiEnv.
 * * `data_tag` must be a valid WuiDragDataTag value.
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
 * * `dest` must be a valid pointer to a WuiDropDestination.
 * * `env` must be a valid pointer to a WuiEnv.
 */
void waterui_call_drop_enter_handler(const struct WuiDropDestination *dest,
                                     const struct WuiEnv *env);

/**
 * Calls the exit handler if set.
 *
 * # Safety
 *
 * * `dest` must be a valid pointer to a WuiDropDestination.
 * * `env` must be a valid pointer to a WuiEnv.
 */
void waterui_call_drop_exit_handler(const struct WuiDropDestination *dest,
                                    const struct WuiEnv *env);

/**
 * Drops a drop destination handler.
 *
 * # Safety
 *
 * * `dest` must be a valid pointer to a WuiDropDestination.
 */
void waterui_drop_drop_destination(struct WuiDropDestination *dest);

/**
 * Calls a LifeCycleHook handler with the given environment.
 *
 * # Safety
 *
 * * `handler` must be a valid pointer to a WuiLifeCycleHookHandler.
 * * `env` must be a valid pointer to a WuiEnv.
 * * This consumes the handler - it can only be called once.
 */
void waterui_call_lifecycle_hook(struct WuiLifeCycleHookHandler *handler, const struct WuiEnv *env);

/**
 * Drops a LifeCycleHook handler without calling it.
 *
 * # Safety
 *
 * * `handler` must be a valid pointer to a WuiLifeCycleHookHandler.
 */
void waterui_drop_lifecycle_hook(struct WuiLifeCycleHookHandler *handler);

/**
 * Calls an OnEvent handler with the given environment.
 * This handler can be called multiple times (repeatable).
 *
 * # Safety
 *
 * * `handler` must be a valid pointer to a WuiOnEventHandler.
 * * `env` must be a valid pointer to a WuiEnv.
 */
void waterui_call_on_event(struct WuiOnEventHandler *handler, const struct WuiEnv *env);

/**
 * Drops an OnEvent handler.
 *
 * # Safety
 *
 * * `handler` must be a valid pointer to a WuiOnEventHandler.
 */
void waterui_drop_on_event(struct WuiOnEventHandler *handler);

/**
 * Drops a WuiGesture, recursively freeing any composite variants.
 *
 * # Safety
 *
 * The gesture pointer must be valid and properly initialized.
 */
void waterui_drop_gesture(struct WuiGesture *gesture);

struct WuiResolvedGradient waterui_force_as_resolved_gradient(struct WuiAnyView *view);

struct WuiTypeId waterui_resolved_gradient_id(void);

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
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiId waterui_read_computed_id(const WuiComputed_Id *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_id(const WuiComputed_Id *computed,
                                                  struct WuiWatcher_Id *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_id(WuiComputed_Id *computed);

/**
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_Id *waterui_clone_computed_id(const WuiComputed_Id *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_Id *waterui_new_watcher_id(void *data,
                                             void (*call)(void*,
                                                          struct WuiId,
                                                          struct WuiWatcherMetadata*),
                                             void (*drop)(void*));

/**
 * Installs a locale into the environment using a predefined locale enum.
 *
 * This installs a `Locale` snapshot into the environment and publishes it
 * to the shared regional runtime context.
 *
 * # Safety
 * - `env` must be a valid pointer from `waterui_init()` or `waterui_env_new()`.
 */
void waterui_env_install_locale(struct WuiEnv *env, enum WuiLocale locale);

/**
 * Installs a locale into the environment using a BCP 47 locale string.
 *
 * This is more flexible than `waterui_env_install_locale()` as it accepts
 * any valid BCP 47 locale identifier (e.g., "en-US", "zh-Hans-CN", "ja-JP").
 *
 * If the locale string is invalid, falls back to English ("en").
 *
 * This installs a `Locale` snapshot into the environment and publishes it
 * to the shared regional runtime context.
 *
 * # Safety
 * - `env` must be a valid pointer from `waterui_init()` or `waterui_env_new()`.
 * - `locale_str` must be a valid null-terminated C string.
 */
void waterui_env_install_locale_string(struct WuiEnv *env, const char *locale_str);

/**
 * Gets the current locale from the environment.
 *
 * Returns the locale as a WuiLocale enum. If the locale doesn't match
 * any predefined enum value, returns `WuiLocale::EnUs` as default.
 *
 * # Safety
 * - `env` must be a valid pointer.
 */
enum WuiLocale waterui_env_get_locale(const struct WuiEnv *env);

/**
 * Gets the current locale from the environment as a canonical BCP 47 string.
 *
 * This is a lossless alternative to `waterui_env_get_locale()`.
 *
 * # Safety
 * - `env` must be a valid pointer or null.
 */
struct WuiStr waterui_env_get_locale_tag(const struct WuiEnv *env);

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
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_Str *waterui_clone_computed_str(const WuiComputed_Str *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
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
 * Sets a `Binding<StyledStr>` using plain text.
 *
 * This helper is intended for native text input controls that only emit plain
 * text updates while the reactive binding type remains `StyledStr`.
 *
 * # Safety
 * `binding` must be a valid pointer to `WuiBinding<StyledStr>`.
 */
void waterui_set_binding_styled_str_plain(WuiBinding_StyledStr *binding, struct WuiStr value);

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiAnyView *waterui_read_binding_any_view(const WuiBinding_AnyView *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_any_view(WuiBinding_AnyView *binding, struct WuiAnyView *value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_any_view(const WuiBinding_AnyView *binding,
                                                       struct WuiWatcher_AnyView *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_any_view(WuiBinding_AnyView *binding);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiAnyView *waterui_read_computed_any_view(const WuiComputed_AnyView *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_any_view(const WuiComputed_AnyView *computed,
                                                        struct WuiWatcher_AnyView *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_any_view(WuiComputed_AnyView *computed);

/**
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_AnyView *waterui_clone_computed_any_view(const WuiComputed_AnyView *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_AnyView *waterui_new_watcher_any_view(void *data,
                                                        void (*call)(void*,
                                                                     struct WuiAnyView*,
                                                                     struct WuiWatcherMetadata*),
                                                        void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
int32_t waterui_read_binding_int(const WuiBinding_i32 *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_int(WuiBinding_i32 *binding, int32_t value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_int(const WuiBinding_i32 *binding,
                                                  struct WuiWatcher_i32 *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_int(WuiBinding_i32 *binding);

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
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_i32 *waterui_clone_computed_i32(const WuiComputed_i32 *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
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
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_bool *waterui_clone_computed_bool(const WuiComputed_bool *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_bool *waterui_new_watcher_bool(void *data,
                                                 void (*call)(void*,
                                                              bool,
                                                              struct WuiWatcherMetadata*),
                                                 void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
float waterui_read_binding_float(const WuiBinding_f32 *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_float(WuiBinding_f32 *binding, float value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_float(const WuiBinding_f32 *binding,
                                                    struct WuiWatcher_f32 *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_float(WuiBinding_f32 *binding);

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
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_f32 *waterui_clone_computed_f32(const WuiComputed_f32 *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_f32 *waterui_new_watcher_f32(void *data,
                                               void (*call)(void*, float, struct WuiWatcherMetadata*),
                                               void (*drop)(void*));

/**
 * Reads the current value from a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
double waterui_read_binding_double(const WuiBinding_f64 *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_double(WuiBinding_f64 *binding, double value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_double(const WuiBinding_f64 *binding,
                                                     struct WuiWatcher_f64 *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_double(WuiBinding_f64 *binding);

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
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_f64 *waterui_clone_computed_f64(const WuiComputed_f64 *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
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
struct WuiDate waterui_read_binding_date(const WuiBinding_Date *binding);

/**
 * Sets the value of a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
void waterui_set_binding_date(WuiBinding_Date *binding, struct WuiDate value);

/**
 * Watches for changes in a binding
 * # Safety
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_binding_date(const WuiBinding_Date *binding,
                                                   struct WuiWatcher_Date *watcher);

/**
 * Drops a binding
 * # Safety
 * The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_binding_date(WuiBinding_Date *binding);

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiDate waterui_read_computed_date(const WuiComputed_Date *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_date(const WuiComputed_Date *computed,
                                                    struct WuiWatcher_Date *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_date(WuiComputed_Date *computed);

/**
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_Date *waterui_clone_computed_date(const WuiComputed_Date *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_Date *waterui_new_watcher_date(void *data,
                                                 void (*call)(void*,
                                                              struct WuiDate,
                                                              struct WuiWatcherMetadata*),
                                                 void (*drop)(void*));

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
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiDateTime waterui_read_computed_date_time(const WuiComputed_DateTime *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_date_time(const WuiComputed_DateTime *computed,
                                                         struct WuiWatcher_DateTime *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_date_time(WuiComputed_DateTime *computed);

/**
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_DateTime *waterui_clone_computed_date_time(const WuiComputed_DateTime *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
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
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_Vec_Date *waterui_clone_computed_date_vec(const WuiComputed_Vec_Date *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_Vec_Date *waterui_new_watcher_date_vec(void *data,
                                                         void (*call)(void*,
                                                                      struct WuiArray_WuiDate,
                                                                      struct WuiWatcherMetadata*),
                                                         void (*drop)(void*));

/**
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiArray_WuiPickerItem waterui_read_computed_picker_items(const WuiComputed_Vec_PickerItem_Id *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_picker_items(const WuiComputed_Vec_PickerItem_Id *computed,
                                                            struct WuiWatcher_Vec_PickerItem_Id *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_picker_items(WuiComputed_Vec_PickerItem_Id *computed);

/**
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_Vec_PickerItem_Id *waterui_clone_computed_picker_items(const WuiComputed_Vec_PickerItem_Id *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_Vec_PickerItem_Id *waterui_new_watcher_picker_items(void *data,
                                                                      void (*call)(void*,
                                                                                   struct WuiArray_WuiPickerItem,
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

struct WuiResolvedShape waterui_force_as_resolved_shape(struct WuiAnyView *view);

struct WuiTypeId waterui_resolved_shape_id(void);

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
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_ColorScheme *waterui_clone_computed_color_scheme(const WuiComputed_ColorScheme *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
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
 * Creates a constant color scheme signal.
 */
WuiComputed_ColorScheme *waterui_computed_color_scheme_constant(enum WuiColorScheme scheme);

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
 * Calls a ColorScheme watcher with the given value.
 * Used by native code to notify Rust when color scheme changes.
 * # Safety
 * The watcher pointer must be valid.
 */
void waterui_call_watcher_color_scheme(const struct WuiWatcher_ColorScheme *watcher,
                                       enum WuiColorScheme value);

/**
 * Drops a ColorScheme watcher.
 * # Safety
 * The watcher pointer must be valid.
 */
void waterui_drop_watcher_color_scheme(struct WuiWatcher_ColorScheme *watcher);

/**
 * Calls a ResolvedColor watcher with the given value.
 * Used by native code to notify Rust when a color value changes.
 * # Safety
 * The watcher pointer must be valid.
 */
void waterui_call_watcher_resolved_color(const struct WuiWatcher_ResolvedColor *watcher,
                                         struct WuiResolvedColor value);

/**
 * Drops a ResolvedColor watcher.
 * # Safety
 * The watcher pointer must be valid.
 */
void waterui_drop_watcher_resolved_color(struct WuiWatcher_ResolvedColor *watcher);

/**
 * Calls a ResolvedFont watcher with the given value.
 * Used by native code to notify Rust when a font value changes.
 * # Safety
 * The watcher pointer must be valid.
 */
void waterui_call_watcher_resolved_font(const struct WuiWatcher_ResolvedFont *watcher,
                                        struct WuiResolvedFont value);

/**
 * Drops a ResolvedFont watcher.
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
 * Gets the ID of a view at the specified index.
 *
 * # Safety
 * The caller must ensure that `anyviews` is a valid pointer and `index` is within bounds.
 */
struct WuiId waterui_anyviews_get_id(const struct WuiAnyViews *anyviews, uintptr_t index);

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
 * Watches for changes in a views collection.
 *
 * The callback receives the current list of view IDs (in order) whenever the collection changes.
 *
 * # Safety
 * - `anyviews` must be a valid pointer.
 * - `data`, `call`, and `drop` must form a valid callback triplet.
 */
struct WuiWatcherGuard *waterui_anyviews_watch(const struct WuiAnyViews *anyviews,
                                               void *data,
                                               void (*call)(void*,
                                                            struct WuiArray_WuiId,
                                                            struct WuiWatcherMetadata*),
                                               void (*drop)(void*));

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
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiAnyViews *waterui_read_computed_views(const WuiComputed_AnyViews_AnyView *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher pointer will be consumed and freed when the returned guard is dropped.
 */
struct WuiWatcherGuard *waterui_watch_computed_views(const WuiComputed_AnyViews_AnyView *computed,
                                                     struct WuiWatcher_AnyViews_AnyView *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_views(WuiComputed_AnyViews_AnyView *computed);

/**
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_AnyViews_AnyView *waterui_clone_computed_views(const WuiComputed_AnyViews_AnyView *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_AnyViews_AnyView *waterui_new_watcher_views(void *data,
                                                              void (*call)(void*,
                                                                           struct WuiAnyViews*,
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
 * Creates a watcher for WindowState from native callbacks.
 *
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_WindowState *waterui_new_watcher_window_state(void *data,
                                                                void (*call)(void*,
                                                                             enum WuiWindowState,
                                                                             struct WuiWatcherMetadata*),
                                                                void (*drop)(void*));

/**
 * Installs a WindowManager into the environment from a native function pointer.
 *
 * Native backends call this during initialization to register their window
 * management implementation. When `Window` views are rendered, the provided
 * callback will be invoked to create and display native windows.
 *
 * Note: Native code should use its global environment to render window content,
 * as the environment cannot be safely passed through the callback.
 *
 * # Safety
 *
 * The caller must ensure that:
 * - `env` is a valid pointer to a `WuiEnv`
 * - `show_fn` is a valid function pointer that can handle `WuiWindow` and create native windows
 */
void waterui_env_install_window_manager(struct WuiEnv *env, WindowShowFn show_fn);

WuiEnv* waterui_init(void);

struct WuiApp waterui_app(WuiEnv *env);

#ifdef __cplusplus
}
#endif
