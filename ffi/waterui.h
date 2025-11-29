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



typedef enum WuiAnimation {
  WuiAnimation_Default,
  WuiAnimation_None,
} WuiAnimation;

typedef enum WuiAxis {
  WuiAxis_Horizontal,
  WuiAxis_Vertical,
  WuiAxis_All,
} WuiAxis;

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
  WuiKeyboardType_Secure,
  WuiKeyboardType_Email,
  WuiKeyboardType_URL,
  WuiKeyboardType_Number,
  WuiKeyboardType_PhoneNumber,
} WuiKeyboardType;

typedef enum WuiProgressStyle {
  WuiProgressStyle_Linear,
  WuiProgressStyle_Circular,
} WuiProgressStyle;

/**
 * Pixel formats supported by the renderer bridge FFI.
 */
typedef enum WuiRendererBufferFormat {
  /**
   * 8-bit per channel RGBA pixels in native byte order.
   */
  WuiRendererBufferFormat_Rgba8888 = 0,
} WuiRendererBufferFormat;

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
 * Safe area insets in points, relative to the container bounds
 */
typedef struct SafeAreaInsets SafeAreaInsets;

typedef struct WuiAction WuiAction;

typedef struct WuiAnyView WuiAnyView;

typedef struct WuiAnyViews WuiAnyViews;

typedef struct WuiColor WuiColor;

typedef struct WuiDynamic WuiDynamic;

typedef struct WuiEnv WuiEnv;

typedef struct WuiFont WuiFont;

typedef struct WuiLayout WuiLayout;

typedef struct WuiRendererView WuiRendererView;

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

typedef struct WuiWatcher_Str WuiWatcher_Str;

typedef struct WuiWatcher_StyledStr WuiWatcher_StyledStr;

typedef struct WuiWatcher_Vec_PickerItem_Id WuiWatcher_Vec_PickerItem_Id;

typedef struct WuiWatcher_Vec_TableColumn WuiWatcher_Vec_TableColumn;

typedef struct WuiWatcher_Video WuiWatcher_Video;

typedef struct WuiWatcher_bool WuiWatcher_bool;

typedef struct WuiWatcher_f32 WuiWatcher_f32;

typedef struct WuiWatcher_f64 WuiWatcher_f64;

typedef struct WuiWatcher_i32 WuiWatcher_i32;

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
 * `WuiArray` can represent mutiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
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
 * `WuiArray` can represent mutiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
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

typedef struct WuiProposalSize {
  float width;
  float height;
} WuiProposalSize;

typedef struct WuiArraySlice_WuiProposalSize {
  struct WuiProposalSize *head;
  uintptr_t len;
} WuiArraySlice_WuiProposalSize;

typedef struct WuiArrayVTable_WuiProposalSize {
  void (*drop)(void*);
  struct WuiArraySlice_WuiProposalSize (*slice)(const void*);
} WuiArrayVTable_WuiProposalSize;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent mutiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiProposalSize {
  NonNull data;
  struct WuiArrayVTable_WuiProposalSize vtable;
} WuiArray_WuiProposalSize;

typedef struct WuiChildMetadata {
  struct WuiProposalSize proposal;
  uint8_t priority;
  bool stretch;
} WuiChildMetadata;

typedef struct WuiArraySlice_WuiChildMetadata {
  struct WuiChildMetadata *head;
  uintptr_t len;
} WuiArraySlice_WuiChildMetadata;

typedef struct WuiArrayVTable_WuiChildMetadata {
  void (*drop)(void*);
  struct WuiArraySlice_WuiChildMetadata (*slice)(const void*);
} WuiArrayVTable_WuiChildMetadata;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent mutiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiChildMetadata {
  NonNull data;
  struct WuiArrayVTable_WuiChildMetadata vtable;
} WuiArray_WuiChildMetadata;

/**
 * FFI representation of safe area insets
 */
typedef struct WuiSafeAreaInsets {
  float top;
  float bottom;
  float leading;
  float trailing;
} WuiSafeAreaInsets;

/**
 * FFI representation of safe area edges (bitflags)
 */
typedef struct WuiSafeAreaEdges {
  uint8_t bits;
} WuiSafeAreaEdges;

/**
 * FFI representation of layout context
 */
typedef struct WuiLayoutContext {
  struct WuiSafeAreaInsets safe_area;
  struct WuiSafeAreaEdges ignores_safe_area;
} WuiLayoutContext;

typedef struct WuiSize {
  float width;
  float height;
} WuiSize;

typedef struct WuiScrollView {
  enum WuiAxis axis;
  struct WuiAnyView *content;
} WuiScrollView;

typedef struct WuiPoint {
  float x;
  float y;
} WuiPoint;

typedef struct WuiRect {
  struct WuiPoint origin;
  struct WuiSize size;
} WuiRect;

/**
 * FFI representation of child placement (rect + context)
 */
typedef struct WuiChildPlacement {
  struct WuiRect rect;
  struct WuiLayoutContext context;
} WuiChildPlacement;

typedef struct WuiArraySlice_WuiChildPlacement {
  struct WuiChildPlacement *head;
  uintptr_t len;
} WuiArraySlice_WuiChildPlacement;

typedef struct WuiArrayVTable_WuiChildPlacement {
  void (*drop)(void*);
  struct WuiArraySlice_WuiChildPlacement (*slice)(const void*);
} WuiArrayVTable_WuiChildPlacement;

/**
 * A generic array structure for FFI, representing a contiguous sequence of elements.
 * `WuiArray` can represent mutiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiChildPlacement {
  NonNull data;
  struct WuiArrayVTable_WuiChildPlacement vtable;
} WuiArray_WuiChildPlacement;

typedef struct WuiButton {
  struct WuiAnyView *label;
  struct WuiAction *action;
} WuiButton;

typedef struct WuiLazy {
  struct WuiAnyViews *contents;
} WuiLazy;

typedef struct Computed_Str WuiComputed_Str;

typedef struct WuiLink {
  struct WuiAnyView *label;
  WuiComputed_Str *url;
} WuiLink;

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
 * `WuiArray` can represent mutiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
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

typedef struct Binding_bool WuiBinding_bool;

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

typedef struct WuiStr Url;

typedef struct WuiPhoto {
  Url source;
  struct WuiAnyView *placeholder;
} WuiPhoto;

typedef struct Computed_Video WuiComputed_Video;

typedef struct Binding_Volume WuiBinding_Volume;

typedef struct WuiVideoPlayer {
  WuiComputed_Video *video;
  WuiBinding_Volume *volume;
} WuiVideoPlayer;

typedef struct Computed_LivePhotoSource WuiComputed_LivePhotoSource;

typedef struct WuiLivePhoto {
  WuiComputed_LivePhotoSource *source;
} WuiLivePhoto;

typedef struct WuiLivePhotoSource {
  Url image;
  Url video;
} WuiLivePhotoSource;

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
 * `WuiArray` can represent mutiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
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
 * `WuiArray` can represent mutiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiPickerItem {
  NonNull data;
  struct WuiArrayVTable_WuiPickerItem vtable;
} WuiArray_WuiPickerItem;

typedef struct WuiVideo {
  Url url;
} WuiVideo;

typedef struct Computed_ColorScheme WuiComputed_ColorScheme;

typedef struct Computed_AnyViews_AnyView WuiComputed_AnyViews_AnyView;



void waterui_configure_hot_reload_endpoint(const char *_host, uint16_t _port);

void waterui_configure_hot_reload_directory(const char *_path);

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
 * Gets the id of a view
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to a properly
 * initialized `WuiAnyView` instance and that it remains valid for the
 * duration of this function call.
 */
struct WuiStr waterui_view_id(const struct WuiAnyView *view);

struct WuiAnyView *waterui_empty_anyview(void);

struct WuiStr waterui_anyview_id(void);

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

struct WuiStr waterui_color_id(void);

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

struct WuiStr waterui_plain_id(void);

struct WuiStr waterui_empty_id(void);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_layout(struct WuiLayout *value);

struct WuiStr waterui_spacer_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiFixedContainer waterui_force_as_fixed_container(struct WuiAnyView *view);

struct WuiStr waterui_fixed_container_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiContainer waterui_force_as_layout_container(struct WuiAnyView *view);

struct WuiStr waterui_layout_container_id(void);

/**
 * Proposes sizes for children based on parent constraints and child metadata.
 *
 * # Safety
 *
 * The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
 * The caller must ensure the layout object remains valid for the duration of this call.
 */
struct WuiArray_WuiProposalSize waterui_layout_propose(struct WuiLayout *layout,
                                                       struct WuiProposalSize parent,
                                                       struct WuiArray_WuiChildMetadata children,
                                                       struct WuiLayoutContext context);

/**
 * Calculates the size required by the layout based on parent constraints and child metadata.
 *
 * # Safety
 *
 * The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
 * The caller must ensure the layout object remains valid for the duration of this call.
 */
struct WuiSize waterui_layout_size(struct WuiLayout *layout,
                                   struct WuiProposalSize parent,
                                   struct WuiArray_WuiChildMetadata children,
                                   struct WuiLayoutContext context);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiScrollView waterui_force_as_scroll_view(struct WuiAnyView *view);

struct WuiStr waterui_scroll_view_id(void);

/**
 * Places child views within the specified bounds based on layout constraints and child metadata.
 *
 * # Safety
 *
 * The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
 * The caller must ensure the layout object remains valid for the duration of this call.
 */
struct WuiArray_WuiChildPlacement waterui_layout_place(struct WuiLayout *layout,
                                                       struct WuiRect bound,
                                                       struct WuiProposalSize proposal,
                                                       struct WuiArray_WuiChildMetadata children,
                                                       struct WuiLayoutContext context);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiButton waterui_force_as_button(struct WuiAnyView *view);

struct WuiStr waterui_button_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiLazy waterui_force_as_lazy(struct WuiAnyView *view);

struct WuiStr waterui_lazy_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiLink waterui_force_as_link(struct WuiAnyView *view);

struct WuiStr waterui_link_id(void);

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

struct WuiStr waterui_text_id(void);

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

struct WuiStr waterui_text_field_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiToggle waterui_force_as_toggle(struct WuiAnyView *view);

struct WuiStr waterui_toggle_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiSlider waterui_force_as_slider(struct WuiAnyView *view);

struct WuiStr waterui_slider_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiStepper waterui_force_as_stepper(struct WuiAnyView *view);

struct WuiStr waterui_stepper_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiColorPicker waterui_force_as_color_picker(struct WuiAnyView *view);

struct WuiStr waterui_color_picker_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiPicker waterui_force_as_picker(struct WuiAnyView *view);

struct WuiStr waterui_picker_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiNavigationView waterui_force_as_navigation_view(struct WuiAnyView *view);

struct WuiStr waterui_navigation_view_id(void);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_tab_content(struct WuiTabContent *value);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiPhoto waterui_force_as_photo(struct WuiAnyView *view);

struct WuiStr waterui_photo_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiVideoPlayer waterui_force_as_video_player(struct WuiAnyView *view);

struct WuiStr waterui_video_player_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiLivePhoto waterui_force_as_live_photo(struct WuiAnyView *view);

struct WuiStr waterui_live_photo_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiLivePhotoSource waterui_force_as_live_photo_source(struct WuiAnyView *view);

struct WuiStr waterui_live_photo_source_id(void);

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

struct WuiStr waterui_dynamic_id(void);

void waterui_dynamic_connect(struct WuiDynamic *dynamic, struct WuiWatcher_AnyView *watcher);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiListItem waterui_force_as_list_item(struct WuiAnyView *view);

struct WuiStr waterui_list_item_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiList waterui_force_as_list(struct WuiAnyView *view);

struct WuiStr waterui_list_id(void);

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

struct WuiStr waterui_table_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiTableColumn waterui_force_as_table_column(struct WuiAnyView *view);

struct WuiStr waterui_table_column_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiProgress waterui_force_as_progress(struct WuiAnyView *view);

struct WuiStr waterui_progress_id(void);

/**
 * # Safety
 * The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
 */
void waterui_drop_renderer_view(struct WuiRendererView *value);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiRendererView *waterui_force_as_renderer_view(struct WuiAnyView *view);

struct WuiStr waterui_renderer_view_id(void);

/**
 * Gets the width of the renderer view.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer.
 */
float waterui_renderer_view_width(const struct WuiRendererView *view);

/**
 * Gets the height of the renderer view.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer.
 */
float waterui_renderer_view_height(const struct WuiRendererView *view);

/**
 * Gets the preferred buffer format for the renderer view.
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer.
 */
enum WuiRendererBufferFormat waterui_renderer_view_preferred_format(const struct WuiRendererView *_view);

/**
 * Renders the view to a CPU buffer.
 *
 * # Safety
 * The caller must ensure that `view` and `pixels` are valid pointers, and that the
 * pixel buffer has sufficient capacity for the given dimensions and stride.
 */
bool waterui_renderer_view_render_cpu(struct WuiRendererView *view,
                                      uint8_t *pixels,
                                      uint32_t width,
                                      uint32_t height,
                                      uintptr_t stride,
                                      enum WuiRendererBufferFormat format);

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
struct WuiVideo waterui_read_computed_video(const WuiComputed_Video *computed);

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
                                                                struct WuiVideo,
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

WuiAnyView* waterui_main(void);

#ifdef __cplusplus
}
#endif
