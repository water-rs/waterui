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
 * Image media type.
 */
#define IMAGE 0

/**
 * Video media type.
 */
#define VIDEO 1

/**
 * Live Photo / Motion Photo media type.
 */
#define LIVE_PHOTO 2

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
 * FFI event enum.
 */
typedef enum WuiEvent {
  WuiEvent_Appear,
  WuiEvent_Disappear,
} WuiEvent;

typedef enum WuiAnimation {
  WuiAnimation_Default,
  WuiAnimation_None,
} WuiAnimation;

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

/**
 * FFI representation of photo events.
 */
typedef enum WuiPhotoEventType {
  WuiPhotoEventType_Loaded = 0,
  WuiPhotoEventType_Error = 1,
} WuiPhotoEventType;

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
} WuiVideoEventType;

/**
 * FFI representation of a simple media filter type.
 * Complex nested filters (All, Not, Any) are not supported via FFI.
 */
typedef enum WuiMediaFilterType {
  /**
   * Filter for live photos only.
   */
  WuiMediaFilterType_LivePhoto = 0,
  /**
   * Filter for videos only.
   */
  WuiMediaFilterType_Video = 1,
  /**
   * Filter for images only.
   */
  WuiMediaFilterType_Image = 2,
  /**
   * Filter for all media types.
   */
  WuiMediaFilterType_All = 3,
} WuiMediaFilterType;

typedef enum WuiProgressStyle {
  WuiProgressStyle_Linear,
  WuiProgressStyle_Circular,
} WuiProgressStyle;

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
typedef struct Computed_Font Computed_Font;

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
typedef struct Computed_LivePhotoSource Computed_LivePhotoSource;

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
typedef struct Computed_Vec_PickerItem_Id Computed_Vec_PickerItem_Id;

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

typedef struct WuiAction WuiAction;

typedef struct WuiAnyView WuiAnyView;

typedef struct WuiAnyViews WuiAnyViews;

typedef struct WuiColor WuiColor;

typedef struct WuiDynamic WuiDynamic;

typedef struct WuiEnv WuiEnv;

typedef struct WuiFont WuiFont;

/**
 * Opaque state held by the native backend after initialization.
 *
 * This struct owns all wgpu resources and the user's renderer.
 * It is created by `waterui_gpu_surface_init` and destroyed by
 * `waterui_gpu_surface_drop`.
 */
typedef struct WuiGpuSurfaceState WuiGpuSurfaceState;

typedef struct WuiLayout WuiLayout;

/**
 * Wrapper for OnEvent to avoid orphan rule issues.
 */
typedef struct WuiOnEventHandler WuiOnEventHandler;

typedef struct WuiTabContent WuiTabContent;

typedef struct WuiWatcherGuard WuiWatcherGuard;

typedef struct WuiWatcherMetadata WuiWatcherMetadata;

typedef struct WuiWatcher_AnyView WuiWatcher_AnyView;

typedef struct WuiWatcher_AnyViews_AnyView WuiWatcher_AnyViews_AnyView;

typedef struct WuiWatcher_Color WuiWatcher_Color;

typedef struct WuiWatcher_ColorScheme WuiWatcher_ColorScheme;

typedef struct WuiWatcher_Font WuiWatcher_Font;

typedef struct WuiWatcher_Id WuiWatcher_Id;

typedef struct WuiWatcher_LivePhotoSource WuiWatcher_LivePhotoSource;

typedef struct WuiWatcher_ResolvedColor WuiWatcher_ResolvedColor;

typedef struct WuiWatcher_ResolvedFont WuiWatcher_ResolvedFont;

typedef struct WuiWatcher_Secure WuiWatcher_Secure;

typedef struct WuiWatcher_Str WuiWatcher_Str;

typedef struct WuiWatcher_StyledStr WuiWatcher_StyledStr;

typedef struct WuiWatcher_Vec_PickerItem_Id WuiWatcher_Vec_PickerItem_Id;

typedef struct WuiWatcher_Vec_TableColumn WuiWatcher_Vec_TableColumn;

typedef struct WuiWatcher_Video WuiWatcher_Video;

typedef struct WuiWatcher_bool WuiWatcher_bool;

typedef struct WuiWatcher_f32 WuiWatcher_f32;

typedef struct WuiWatcher_f64 WuiWatcher_f64;

typedef struct WuiWatcher_i32 WuiWatcher_i32;

/**
 * Type ID as a 128-bit value for O(1) comparison.
 *
 * - Normal build: Uses `std::any::TypeId` (guaranteed unique by Rust)
 * - Hot reload: Uses 128-bit FNV-1a hash of `type_name()` (stable across dylib reloads)
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

typedef struct WuiGesture {
  WuiGesture_Tag tag;
  union {
    WuiGesture_Tap_Body tap;
    WuiGesture_LongPress_Body long_press;
    WuiGesture_Drag_Body drag;
    WuiGesture_Magnification_Body magnification;
    WuiGesture_Rotation_Body rotation;
    WuiGesture_Then_Body then;
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

typedef struct Computed_Color WuiComputed_Color;

typedef struct Computed_Str WuiComputed_Str;

/**
 * FFI-safe representation of a background.
 */
typedef enum WuiBackground_Tag {
  /**
   * A solid color background.
   */
  WuiBackground_Color,
  /**
   * An image background.
   */
  WuiBackground_Image,
} WuiBackground_Tag;

typedef struct WuiBackground_Color_Body {
  WuiComputed_Color *color;
} WuiBackground_Color_Body;

typedef struct WuiBackground_Image_Body {
  WuiComputed_Str *image;
} WuiBackground_Image_Body;

typedef struct WuiBackground {
  WuiBackground_Tag tag;
  union {
    WuiBackground_Color_Body color;
    WuiBackground_Image_Body image;
  };
} WuiBackground;

typedef struct WuiMetadata_WuiBackground {
  struct WuiAnyView *content;
  struct WuiBackground value;
} WuiMetadata_WuiBackground;

/**
 * Type alias for Metadata<Background> FFI struct
 */
typedef struct WuiMetadata_WuiBackground WuiMetadataBackground;

/**
 * FFI-safe representation of a foreground color.
 */
typedef struct WuiForegroundColor {
  /**
   * Pointer to the computed color.
   */
  WuiComputed_Color *color;
} WuiForegroundColor;

typedef struct WuiMetadata_WuiForegroundColor {
  struct WuiAnyView *content;
  struct WuiForegroundColor value;
} WuiMetadata_WuiForegroundColor;

/**
 * Type alias for Metadata<ForegroundColor> FFI struct
 */
typedef struct WuiMetadata_WuiForegroundColor WuiMetadataForeground;

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

typedef struct WuiResolvedColor {
  float red;
  float green;
  float blue;
  float opacity;
  float headroom;
} WuiResolvedColor;

typedef struct Computed_ResolvedColor WuiComputed_ResolvedColor;

typedef struct Binding_Color WuiBinding_Color;

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
  struct WuiSize (*measure)(void *context, struct WuiProposalSize proposal);
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

typedef struct Computed_StyledStr WuiComputed_StyledStr;

typedef struct Binding_Font WuiBinding_Font;

typedef struct Computed_Font WuiComputed_Font;

typedef struct WuiText {
  WuiComputed_StyledStr *content;
} WuiText;

typedef struct WuiResolvedFont {
  float size;
  enum WuiFontWeight weight;
} WuiResolvedFont;

typedef struct Computed_ResolvedFont WuiComputed_ResolvedFont;

typedef struct Binding_Str WuiBinding_Str;

typedef struct WuiTextField {
  struct WuiAnyView *label;
  WuiBinding_Str *value;
  struct WuiText prompt;
  enum WuiKeyboardType keyboard;
} WuiTextField;

typedef struct WuiToggle {
  struct WuiAnyView *label;
  WuiBinding_bool *toggle;
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
} WuiColorPicker;

typedef struct Computed_Vec_PickerItem_Id WuiComputed_Vec_PickerItem_Id;

typedef struct Binding_Id WuiBinding_Id;

typedef struct WuiPicker {
  WuiComputed_Vec_PickerItem_Id *items;
  WuiBinding_Id *selection;
} WuiPicker;

typedef struct Binding_Secure WuiBinding_Secure;

typedef struct WuiSecureField {
  struct WuiAnyView *label;
  WuiBinding_Secure *value;
} WuiSecureField;

typedef struct Computed_bool WuiComputed_bool;

typedef struct WuiBar {
  struct WuiText title;
  WuiComputed_Color *color;
  WuiComputed_bool *hidden;
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
} WuiNavigationStack;

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
 * FFI representation of a photo event.
 */
typedef struct WuiPhotoEvent {
  enum WuiPhotoEventType event_type;
  struct WuiStr error_message;
} WuiPhotoEvent;

/**
 * A C-compatible function wrapper that can be called multiple times.
 *
 * This structure wraps a Rust `Fn` closure to allow it to be passed across
 * the FFI boundary while maintaining proper memory management.
 */
typedef struct WuiFn_WuiPhotoEvent {
  void *data;
  void (*call)(const void*, struct WuiPhotoEvent);
  void (*drop)(void*);
} WuiFn_WuiPhotoEvent;

typedef struct WuiPhoto {
  struct WuiStr source;
  struct WuiFn_WuiPhotoEvent on_event;
} WuiPhoto;

typedef struct Binding_Volume WuiBinding_Volume;

/**
 * FFI representation of a video event.
 */
typedef struct WuiVideoEvent {
  enum WuiVideoEventType event_type;
  struct WuiStr error_message;
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
   * The volume of the video.
   */
  WuiBinding_Volume *volume;
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
   * The volume of the video player.
   */
  WuiBinding_Volume *volume;
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

typedef struct Computed_LivePhotoSource WuiComputed_LivePhotoSource;

typedef struct WuiLivePhoto {
  WuiComputed_LivePhotoSource *source;
} WuiLivePhoto;

/**
 * FFI representation of a Video source for Computed signals.
 * This is used by Android to observe video source changes reactively.
 */
typedef struct WuiComputedVideo {
  /**
   * The URL of the video source.
   */
  struct WuiStr url;
} WuiComputedVideo;

typedef struct Computed_Video WuiComputed_Video;

/**
 * FFI representation of a selected media item.
 */
typedef struct WuiSelected {
  /**
   * The unique identifier of the selected media item.
   */
  uint32_t id;
} WuiSelected;

/**
 * A callback for receiving selected media ID when user picks media.
 *
 * This is a C-compatible closure that native code calls when picker completes.
 */
typedef struct MediaPickerPresentCallback {
  /**
   * Opaque pointer to the callback data.
   */
  void *data;
  /**
   * Function to call with the selected media. This consumes the callback.
   */
  void (*call)(void*, struct WuiSelected);
} MediaPickerPresentCallback;

/**
 * Type alias for the native media picker present function.
 */
typedef void (*MediaPickerPresentFn)(enum WuiMediaFilterType, struct MediaPickerPresentCallback);

/**
 * FFI representation of the result from loading media.
 *
 * For Live Photos / Motion Photos, both `url_ptr` (image) and `video_url_ptr` (video)
 * are populated. For regular images/videos, only `url_ptr` is used.
 */
typedef struct MediaLoadResult {
  /**
   * Pointer to UTF-8 encoded URL string (image URL for Live Photos).
   */
  const uint8_t *url_ptr;
  /**
   * Length of the URL string in bytes.
   */
  uintptr_t url_len;
  /**
   * Pointer to UTF-8 encoded video URL (only for Live Photos).
   */
  const uint8_t *video_url_ptr;
  /**
   * Length of the video URL string in bytes.
   */
  uintptr_t video_url_len;
  /**
   * Media type: 0 = Image, 1 = Video, 2 = LivePhoto.
   */
  uint8_t media_type;
} MediaLoadResult;

/**
 * A callback for receiving loaded media from native code.
 *
 * This is a C-compatible closure that native code calls with the result.
 */
typedef struct MediaLoadCallback {
  /**
   * Opaque pointer to the callback data.
   */
  void *data;
  /**
   * Function to call with the result. This consumes the callback.
   */
  void (*call)(void*, struct MediaLoadResult);
} MediaLoadCallback;

/**
 * Type alias for the native media load function.
 */
typedef void (*MediaLoadFn)(uint32_t, struct MediaLoadCallback);

typedef struct WuiListItem {
  struct WuiAnyView *content;
} WuiListItem;

typedef struct WuiList {
  struct WuiAnyViews *contents;
} WuiList;

typedef struct WuiTableColumn {
  struct WuiText label;
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

typedef struct Computed_f64 WuiComputed_f64;

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
   * Opaque pointer to the boxed GpuRenderer trait object.
   * This is consumed during init and should not be used after.
   */
  void *renderer;
} WuiGpuSurface;

typedef struct WuiId {
  int32_t inner;
} WuiId;

typedef struct Computed_Id WuiComputed_Id;

typedef struct Binding_AnyView WuiBinding_AnyView;

typedef struct Computed_AnyView WuiComputed_AnyView;

typedef struct Binding_f32 WuiBinding_f32;

typedef struct Computed_f32 WuiComputed_f32;

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

typedef struct WuiLivePhotoSource {
  struct WuiStr image;
  struct WuiStr video;
} WuiLivePhotoSource;

typedef struct Computed_ColorScheme WuiComputed_ColorScheme;

typedef struct Computed_AnyViews_AnyView WuiComputed_AnyViews_AnyView;

typedef struct Binding_Rect WuiBinding_Rect;

typedef struct Binding_WindowState WuiBinding_WindowState;

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
} WuiWindow;

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
 * - Normal build: Returns the view's `TypeId` (guaranteed unique)
 * - Hot reload: Returns 128-bit hash of `type_name()` (stable across dylibs)
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
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
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
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
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
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
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
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
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
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_metadata_background_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataBackground waterui_force_as_metadata_background(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_metadata_foreground_id(void);

/**
 * Force-casts an AnyView to this metadata type
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to an `AnyView`
 * that contains a `Metadata<$ty>`.
 */
WuiMetadataForeground waterui_force_as_metadata_foreground(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
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
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
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
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
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
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
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

enum WuiAnimation waterui_get_animation(const struct WuiWatcherMetadata *metadata);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_color(struct WuiColor *value);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiColor *waterui_force_as_color(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_color_id(void);

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
 * The watcher must be a valid callback function.
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
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiStr waterui_force_as_plain(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
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

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiFixedContainer waterui_force_as_fixed_container(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_fixed_container_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiContainer waterui_force_as_layout_container(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_layout_container_id(void);

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

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiScrollView waterui_force_as_scroll_view(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_scroll_view_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiButton waterui_force_as_button(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
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
 * The watcher must be a valid callback function.
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

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiText waterui_force_as_text(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
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

WuiComputed_ResolvedFont *waterui_resolve_font(const struct WuiFont *font,
                                               const struct WuiEnv *env);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiTextField waterui_force_as_text_field(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_text_field_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiToggle waterui_force_as_toggle(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_toggle_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiSlider waterui_force_as_slider(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_slider_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiStepper waterui_force_as_stepper(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_stepper_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiColorPicker waterui_force_as_color_picker(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_color_picker_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiPicker waterui_force_as_picker(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_picker_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiSecureField waterui_force_as_secure_field(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_secure_field_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiNavigationView waterui_force_as_navigation_view(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_navigation_view_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiNavigationStack waterui_force_as_navigation_stack(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_navigation_stack_id(void);

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

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiTabs waterui_force_as_tabs(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_tabs_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiPhoto waterui_force_as_photo(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_photo_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiVideo waterui_force_as_video(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_video_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiVideoPlayer waterui_force_as_video_player(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_video_player_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiLivePhoto waterui_force_as_live_photo(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_live_photo_id(void);

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
 * Installs a MediaPickerManager into the environment from native function pointers.
 *
 * Native backends call this during initialization to register their media picker
 * implementation. This unified manager handles both presenting the picker and loading media.
 *
 * # Safety
 *
 * The caller must ensure that:
 * - `env` is a valid pointer to a `WuiEnv`
 * - `present_fn` is a valid function pointer to the native media picker presentation
 * - `load_fn` is a valid function pointer to the native media loader implementation
 */
void waterui_env_install_media_picker_manager(struct WuiEnv *env,
                                              MediaPickerPresentFn present_fn,
                                              MediaLoadFn load_fn);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_dynamic(struct WuiDynamic *value);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiDynamic *waterui_force_as_dynamic(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_dynamic_id(void);

void waterui_dynamic_connect(struct WuiDynamic *dynamic, struct WuiWatcher_AnyView *watcher);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiListItem waterui_force_as_list_item(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_list_item_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiList waterui_force_as_list(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_list_id(void);

/**
 * Calls the delete callback for a list item.
 *
 * # Safety
 * The caller must ensure that `item` and `env` are valid pointers.
 */
void waterui_list_item_call_delete(struct WuiListItem *item,
                                   const struct WuiEnv *env,
                                   uintptr_t index);

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

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiTable waterui_force_as_table(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_table_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiTableColumn waterui_force_as_table_column(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_table_column_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiProgress waterui_force_as_progress(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_progress_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiGpuSurface waterui_force_as_gpu_surface(struct WuiAnyView *view);

/**
 * Returns the type ID as a 128-bit value for O(1) comparison.
 * Uses TypeId in normal builds, type_name hash in hot reload builds.
 */
struct WuiTypeId waterui_gpu_surface_id(void);

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
                                                    uint32_t height);

/**
 * Render a single frame.
 *
 * This function should be called from a display-sync callback (CADisplayLink on Apple,
 * Choreographer on Android) to render at the display's refresh rate.
 *
 * # Arguments
 *
 * * `state` - Pointer to the initialized state from `waterui_gpu_surface_init`
 * * `width` - Current surface width in pixels (from layout)
 * * `height` - Current surface height in pixels (from layout)
 *
 * # Returns
 *
 * `true` if rendering succeeded, `false` on error.
 *
 * # Safety
 *
 * `state` must be a valid pointer from `waterui_gpu_surface_init`.
 */
bool waterui_gpu_surface_render(struct WuiGpuSurfaceState *state, uint32_t width, uint32_t height);

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
 * Calls an OnEvent handler with the given environment.
 *
 * # Safety
 *
 * * `handler` must be a valid pointer to a WuiOnEventHandler.
 * * `env` must be a valid pointer to a WuiEnv.
 * * This consumes the handler - it can only be called once.
 */
void waterui_call_on_event(struct WuiOnEventHandler *handler, const struct WuiEnv *env);

/**
 * Drops an OnEvent handler without calling it.
 *
 * # Safety
 *
 * * `handler` must be a valid pointer to a WuiOnEventHandler.
 */
void waterui_drop_on_event(struct WuiOnEventHandler *handler);

/**
 * Drops a WuiGesture, recursively freeing any Then variants.
 *
 * # Safety
 *
 * The gesture pointer must be valid and properly initialized.
 */
void waterui_drop_gesture(struct WuiGesture *gesture);

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
 * The watcher must be a valid callback function.
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
 * The watcher must be a valid callback function.
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
 * The watcher must be a valid callback function.
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
 * The watcher must be a valid callback function.
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
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
int32_t waterui_read_computed_i32(const WuiComputed_i32 *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
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
 * The watcher must be a valid callback function.
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
 * The watcher must be a valid callback function.
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
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
float waterui_read_computed_f32(const WuiComputed_f32 *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
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
 * The watcher must be a valid callback function.
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
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
double waterui_read_computed_f64(const WuiComputed_f64 *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
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
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiArray_WuiPickerItem waterui_read_computed_picker_items(const WuiComputed_Vec_PickerItem_Id *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
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
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiLivePhotoSource waterui_read_computed_live_photo_source(const WuiComputed_LivePhotoSource *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiWatcherGuard *waterui_watch_computed_live_photo_source(const WuiComputed_LivePhotoSource *computed,
                                                                 struct WuiWatcher_LivePhotoSource *watcher);

/**
 * Drops a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
void waterui_drop_computed_live_photo_source(WuiComputed_LivePhotoSource *computed);

/**
 * Clones a computed
 * # Safety
 * The caller must ensure that `computed` is a valid pointer.
 */
WuiComputed_LivePhotoSource *waterui_clone_computed_live_photo_source(const WuiComputed_LivePhotoSource *computed);

/**
 * Creates a watcher from native callbacks.
 * # Safety
 * All function pointers must be valid.
 */
struct WuiWatcher_LivePhotoSource *waterui_new_watcher_live_photo_source(void *data,
                                                                         void (*call)(void*,
                                                                                      struct WuiLivePhotoSource,
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
 * The watcher must be a valid callback function.
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
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
enum WuiColorScheme waterui_read_computed_color_scheme(const WuiComputed_ColorScheme *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
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
 */
void waterui_theme_install_color_scheme(struct WuiEnv *env, WuiComputed_ColorScheme *signal);

/**
 * Returns the current color scheme signal from the environment.
 */
WuiComputed_ColorScheme *waterui_theme_color_scheme(const struct WuiEnv *env);

/**
 * Installs a color signal for a specific slot.
 *
 * Takes ownership of the signal pointer.
 */
void waterui_theme_install_color(struct WuiEnv *env,
                                 enum WuiColorSlot slot,
                                 WuiComputed_ResolvedColor *signal);

/**
 * Returns the color signal for a specific slot.
 *
 * Returns a new reference to the signal. Caller must drop it when done.
 */
WuiComputed_ResolvedColor *waterui_theme_color(const struct WuiEnv *env, enum WuiColorSlot slot);

/**
 * Installs a font signal for a specific slot.
 *
 * Takes ownership of the signal pointer.
 */
void waterui_theme_install_font(struct WuiEnv *env,
                                enum WuiFontSlot slot,
                                WuiComputed_ResolvedFont *signal);

/**
 * Returns the font signal for a specific slot.
 *
 * Returns a new reference to the signal. Caller must drop it when done.
 */
WuiComputed_ResolvedFont *waterui_theme_font(const struct WuiEnv *env, enum WuiFontSlot slot);

/**
 * Legacy function to install all theme values at once.
 *
 * **Deprecated**: Use the new slot-based API instead:
 * - `waterui_theme_install_color_scheme()`
 * - `waterui_theme_install_color()`
 * - `waterui_theme_install_font()`
 */
void waterui_env_install_theme(struct WuiEnv *env,
                               WuiComputed_ResolvedColor *background,
                               WuiComputed_ResolvedColor *surface,
                               WuiComputed_ResolvedColor *surface_variant,
                               WuiComputed_ResolvedColor *border,
                               WuiComputed_ResolvedColor *foreground,
                               WuiComputed_ResolvedColor *muted_foreground,
                               WuiComputed_ResolvedColor *accent,
                               WuiComputed_ResolvedColor *accent_foreground,
                               WuiComputed_ResolvedFont *body,
                               WuiComputed_ResolvedFont *title,
                               WuiComputed_ResolvedFont *headline,
                               WuiComputed_ResolvedFont *subheadline,
                               WuiComputed_ResolvedFont *caption);

WuiComputed_ResolvedColor *waterui_theme_color_background(const struct WuiEnv *env);

WuiComputed_ResolvedColor *waterui_theme_color_surface(const struct WuiEnv *env);

WuiComputed_ResolvedColor *waterui_theme_color_surface_variant(const struct WuiEnv *env);

WuiComputed_ResolvedColor *waterui_theme_color_border(const struct WuiEnv *env);

WuiComputed_ResolvedColor *waterui_theme_color_foreground(const struct WuiEnv *env);

WuiComputed_ResolvedColor *waterui_theme_color_muted_foreground(const struct WuiEnv *env);

WuiComputed_ResolvedColor *waterui_theme_color_accent(const struct WuiEnv *env);

WuiComputed_ResolvedColor *waterui_theme_color_accent_foreground(const struct WuiEnv *env);

WuiComputed_ResolvedFont *waterui_theme_font_body(const struct WuiEnv *env);

WuiComputed_ResolvedFont *waterui_theme_font_title(const struct WuiEnv *env);

WuiComputed_ResolvedFont *waterui_theme_font_headline(const struct WuiEnv *env);

WuiComputed_ResolvedFont *waterui_theme_font_subheadline(const struct WuiEnv *env);

WuiComputed_ResolvedFont *waterui_theme_font_caption(const struct WuiEnv *env);

WuiComputed_ResolvedFont *waterui_theme_font_footnote(const struct WuiEnv *env);

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
 * Reads the current value from a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiAnyViews *waterui_read_computed_views(const WuiComputed_AnyViews_AnyView *computed);

/**
 * Watches for changes in a computed
 * # Safety
 * The computed pointer must be valid and point to a properly initialized computed object.
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

WuiEnv* waterui_init(void);

struct WuiApp waterui_app(WuiEnv *env);

#ifdef __cplusplus
}
#endif
