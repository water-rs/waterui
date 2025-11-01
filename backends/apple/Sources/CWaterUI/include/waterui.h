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

typedef enum WuiEvent {
  WuiEvent_Appear,
  WuiEvent_Disappear,
  WuiEvent_Unknown,
} WuiEvent;

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

/**
 * Gesture event kind.
 */
typedef enum WuiGestureEventKind {
  WuiGestureEventKind_Tap,
  WuiGestureEventKind_LongPress,
  WuiGestureEventKind_Drag,
  WuiGestureEventKind_Magnification,
} WuiGestureEventKind;

/**
 * Describes the kind of gesture attached to a metadata entry.
 */
typedef enum WuiGestureKind {
  WuiGestureKind_Tap,
  WuiGestureKind_LongPress,
  WuiGestureKind_Drag,
  WuiGestureKind_Magnification,
  WuiGestureKind_Rotation,
  WuiGestureKind_Then,
} WuiGestureKind;

typedef enum WuiGesturePhase {
  WuiGesturePhase_Started,
  WuiGesturePhase_Updated,
  WuiGesturePhase_Ended,
  WuiGesturePhase_Cancelled,
} WuiGesturePhase;

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
typedef struct Computed_Font Computed_Font;

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
 * An `Environment` stores a map of types to values.
 *
 * Each type can have at most one value in the environment. The environment
 * is used to pass contextual information from parent views to child views.
 *
 * # Examples
 *
 * ```
 * use waterui_core::Environment;
 *
 * let mut env = Environment::new();
 * env.insert(String::from("hello"));
 *
 * // Get the value back
 * assert_eq!(env.get::<String>(), Some(&String::from("hello")));
 *
 * // Remove the value
 * env.remove::<String>();
 * assert_eq!(env.get::<String>(), None);
 * ```
 */
typedef struct Environment Environment;

typedef struct WuiAction WuiAction;

typedef struct WuiAnyView WuiAnyView;

typedef struct WuiAnyViews WuiAnyViews;

typedef struct WuiColor WuiColor;

typedef struct WuiDynamic WuiDynamic;

typedef struct WuiOnEvent WuiOnEvent;

typedef struct WuiEnv WuiEnv;

typedef struct WuiFont WuiFont;

typedef struct WuiLayout WuiLayout;

typedef struct WuiRendererView WuiRendererView;

typedef struct WuiTabContent WuiTabContent;

typedef struct WuiWatcherGuard WuiWatcherGuard;

typedef struct WuiWatcherMetadata WuiWatcherMetadata;

typedef struct WuiResolvedColor {
  float red;
  float green;
  float blue;
  float opacity;
} WuiResolvedColor;

typedef struct WuiWatcher_WuiResolvedColor {
  void *data;
  void (*call)(const void*, struct WuiResolvedColor, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_WuiResolvedColor;

typedef struct WuiWatcher_____WuiColor {
  void *data;
  void (*call)(const void*, struct WuiColor*, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_____WuiColor;

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

typedef struct WuiMetadata_OnEvent {
  struct WuiAnyView *content;
  struct WuiOnEvent *value;
} WuiMetadata_OnEvent;

typedef struct WuiAssociatedValue {
  void *data;
  void (*drop)(void*);
} WuiAssociatedValue;

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
 * `WuiArray` can represent mutiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 * We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
 */
typedef struct WuiArray_WuiRect {
  NonNull data;
  struct WuiArrayVTable_WuiRect vtable;
} WuiArray_WuiRect;

/**
 * C representation of a WaterUI button for FFI purposes.
 */
typedef struct WuiButton {
  /**
   * Pointer to the button's label view
   */
  struct WuiAnyView *label;
  /**
   * Pointer to the button's action handler
   */
  struct WuiAction *action;
} WuiButton;

typedef struct WuiLazy {
  struct WuiAnyViews *contents;
} WuiLazy;

typedef struct WuiLink {
  struct WuiAnyView *label;
  struct Computed_Str *url;
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

typedef struct WuiWatcher_WuiStyledStr {
  void *data;
  void (*call)(const void*, struct WuiStyledStr, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_WuiStyledStr;

typedef struct WuiWatcher_____WuiFont {
  void *data;
  void (*call)(const void*, struct WuiFont*, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_____WuiFont;

/**
 * C representation of Text configuration
 */
typedef struct WuiText {
  /**
   * Pointer to the text content computed value
   */
  struct Computed_StyledStr *content;
} WuiText;

/**
 * C representation of Font
 */
typedef struct WuiResolvedFont {
  float size;
  enum WuiFontWeight weight;
} WuiResolvedFont;

typedef struct WuiWatcher_WuiResolvedFont {
  void *data;
  void (*call)(const void*, struct WuiResolvedFont, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_WuiResolvedFont;

/**
 * C representation of a TextField configuration
 */
typedef struct WuiTextField {
  /**
   * Pointer to the text field's label view
   */
  struct WuiAnyView *label;
  /**
   * Pointer to the text value binding
   */
  struct Binding_Str *value;
  /**
   * Pointer to the prompt text
   */
  struct WuiText prompt;
  /**
   * The keyboard type to use
   */
  enum WuiKeyboardType keyboard;
} WuiTextField;

/**
 * C representation of a Toggle configuration
 */
typedef struct WuiToggle {
  /**
   * Pointer to the toggle's label view
   */
  struct WuiAnyView *label;
  /**
   * Pointer to the toggle state binding
   */
  struct Binding_bool *toggle;
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
 * C representation of a Slider configuration
 */
typedef struct WuiSlider {
  /**
   * Pointer to the slider's label view
   */
  struct WuiAnyView *label;
  /**
   * Pointer to the minimum value label view
   */
  struct WuiAnyView *min_value_label;
  /**
   * Pointer to the maximum value label view
   */
  struct WuiAnyView *max_value_label;
  /**
   * The range of values
   */
  struct WuiRange_f64 range;
  /**
   * Pointer to the value binding
   */
  struct Binding_f64 *value;
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
 * C representation of a Stepper configuration
 */
typedef struct WuiStepper {
  /**
   * Pointer to the value binding
   */
  struct Binding_i32 *value;
  /**
   * Pointer to the step size computed value
   */
  struct Computed_i32 *step;
  /**
   * Pointer to the stepper's label view
   */
  struct WuiAnyView *label;
  /**
   * The valid range of values
   */
  struct WuiRange_i32 range;
} WuiStepper;

typedef struct WuiColorPicker {
  struct WuiAnyView *label;
  struct Binding_Color *value;
} WuiColorPicker;

typedef struct WuiPicker {
  struct Computed_Vec_PickerItem_Id *items;
  struct Binding_Id *selection;
} WuiPicker;

typedef struct WuiStr Url;

/**
 * C representation of Photo configuration
 */
typedef struct WuiPhoto {
  /**
   * The image source URL
   */
  Url source;
  /**
   * Pointer to the placeholder view
   */
  struct WuiAnyView *placeholder;
} WuiPhoto;

/**
 * C representation of VideoPlayer configuration
 */
typedef struct WuiVideoPlayer {
  /**
   * Pointer to the video computed value
   */
  struct Computed_Video *video;
  /**
   * Pointer to the volume binding
   */
  struct Binding_Volume *volume;
} WuiVideoPlayer;

/**
 * C representation of LivePhoto configuration
 */
typedef struct WuiLivePhoto {
  /**
   * Pointer to the live photo source computed value
   */
  struct Computed_LivePhotoSource *source;
} WuiLivePhoto;

typedef struct WuiWatcher_____WuiAnyView {
  void *data;
  void (*call)(const void*, struct WuiAnyView*, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_____WuiAnyView;

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

typedef struct WuiWatcher_WuiArray_WuiTableColumn {
  void *data;
  void (*call)(const void*, struct WuiArray_WuiTableColumn, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_WuiArray_WuiTableColumn;

typedef struct WuiTable {
  struct Computed_Vec_TableColumn *columns;
} WuiTable;

typedef struct WuiProgress {
  struct WuiAnyView *label;
  struct WuiAnyView *value_label;
  struct Computed_f64 *value;
  enum WuiProgressStyle style;
} WuiProgress;

typedef struct WuiTapGesture {
  uint32_t count;
} WuiTapGesture;

typedef struct WuiLongPressGesture {
  uint32_t duration;
} WuiLongPressGesture;

typedef struct WuiDragGesture {
  float min_distance;
} WuiDragGesture;

typedef struct WuiMagnificationGesture {
  float initial_scale;
} WuiMagnificationGesture;

typedef struct WuiRotationGesture {
  float initial_angle;
} WuiRotationGesture;

typedef struct WuiGesture {
  enum WuiGestureKind kind;
  struct WuiTapGesture tap;
  struct WuiLongPressGesture long_press;
  struct WuiDragGesture drag;
  struct WuiMagnificationGesture magnification;
  struct WuiRotationGesture rotation;
  struct WuiGesture *first;
  struct WuiGesture *then;
} WuiGesture;

typedef struct WuiGestureMetadata {
  struct WuiAnyView *view;
  struct WuiGesture *gesture;
  struct WuiAction *action;
} WuiGestureMetadata;

/**
 * FFI-safe two-dimensional point used in gesture payloads.
 */
typedef struct WuiGesturePoint {
  float x;
  float y;
} WuiGesturePoint;

/**
 * FFI-safe gesture event payload sent from the backend.
 */
typedef struct WuiGestureEvent {
  enum WuiGestureEventKind kind;
  enum WuiGesturePhase phase;
  struct WuiGesturePoint location;
  struct WuiGesturePoint translation;
  struct WuiGesturePoint velocity;
  float scale;
  float velocity_scalar;
  uint32_t count;
  float duration;
} WuiGestureEvent;

typedef struct WuiWatcher_WuiStr {
  void *data;
  void (*call)(const void*, struct WuiStr, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_WuiStr;

typedef struct WuiWatcher_i32 {
  void *data;
  void (*call)(const void*, int32_t, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_i32;

typedef struct WuiWatcher_bool {
  void *data;
  void (*call)(const void*, bool, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_bool;

typedef struct WuiWatcher_f32 {
  void *data;
  void (*call)(const void*, float, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_f32;

typedef struct WuiWatcher_f64 {
  void *data;
  void (*call)(const void*, double, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_f64;

typedef struct WuiId {
  int32_t inner;
} WuiId;

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

typedef struct WuiWatcher_WuiArray_WuiPickerItem {
  void *data;
  void (*call)(const void*, struct WuiArray_WuiPickerItem, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_WuiArray_WuiPickerItem;

typedef struct WuiVideo {
  Url url;
} WuiVideo;

typedef struct WuiWatcher_WuiVideo {
  void *data;
  void (*call)(const void*, struct WuiVideo, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_WuiVideo;

/**
 * C representation of LivePhotoSource
 */
typedef struct WuiLivePhotoSource {
  /**
   * The image URL
   */
  Url image;
  /**
   * The video URL
   */
  Url video;
} WuiLivePhotoSource;

typedef struct WuiWatcher_WuiLivePhotoSource {
  void *data;
  void (*call)(const void*, struct WuiLivePhotoSource, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_WuiLivePhotoSource;

typedef struct WuiWatcher_WuiId {
  void *data;
  void (*call)(const void*, struct WuiId, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_WuiId;

typedef struct WuiWatcher_____WuiAnyViews {
  void *data;
  void (*call)(const void*, struct WuiAnyViews*, struct WuiWatcherMetadata*);
  void (*drop)(void*);
} WuiWatcher_____WuiAnyViews;

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * The pointer must be a valid pointer to a properly initialized value
 * of the expected type, and must not be used after this function is called.
 */
void waterui_drop_env(struct WuiEnv *value);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * The pointer must be a valid pointer to a properly initialized value
 * of the expected type, and must not be used after this function is called.
 */
void waterui_drop_any_view(struct WuiAnyView *value);

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
struct WuiAnyView *waterui_view_body(struct WuiAnyView *view, struct Environment *env);

/**
 * Gets the type ID of a view
 *
 * # Safety
 * The caller must ensure that `view` is a valid pointer to a properly
 * initialized `WuiAnyView` instance and that it remains valid for the
 * duration of this function call.
 */
struct WuiStr waterui_view_id(const struct WuiAnyView *view);

struct WuiAnyView *waterui_empty_anyview(void);

struct WuiStr waterui_anyview_id(void);
struct WuiStr waterui_metadata_on_event_id(void);
struct WuiMetadata_OnEvent waterui_metadata_force_as_on_event(struct WuiAnyView *view);
enum WuiEvent waterui_on_event_kind(const struct WuiOnEvent *on_event);
void waterui_on_event_trigger(struct WuiOnEvent *on_event, const struct WuiEnv *env);
void waterui_drop_on_event(struct WuiOnEvent *value);
struct WuiAnyView *waterui_associated_view(struct WuiAssociatedValue value, struct WuiAnyView *view);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * The pointer must be a valid pointer to a properly initialized value
 * of the expected type, and must not be used after this function is called.
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
 * Drops the FFI value.
 *
 * # Safety
 *
 * The pointer must be a valid pointer to a properly initialized value
 * of the expected type, and must not be used after this function is called.
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
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_resolved_color(struct Computed_ResolvedColor *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiResolvedColor waterui_read_computed_resolved_color(const struct Computed_ResolvedColor *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_resolved_color(const struct Computed_ResolvedColor *computed,
                                                              struct WuiWatcher_WuiResolvedColor watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_color_watcher_guard(struct Binding_Color *value);

/**
 * Reads the current value from a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiColor *waterui_binding_read_color(const struct Binding_Color *binding);

/**
 * Sets a new value to a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The value must be a valid instance of the FFI type.
 */
void waterui_binding_set_color(struct Binding_Color *binding, struct WuiColor *value);

/**
 * Watches for changes in a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_binding_watch_color(const struct Binding_Color *binding,
                                                    struct WuiWatcher_____WuiColor watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_color(struct Computed_Color *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiColor *waterui_read_computed_color(const struct Computed_Color *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_color(const struct Computed_Color *computed,
                                                     struct WuiWatcher_____WuiColor watcher);

/**
 * Resolves a color in the given environment.
 *
 * # Safety
 *
 * Both `color` and `env` must be valid, non-null pointers to their respective types.
 */
struct Computed_ResolvedColor *waterui_resolve_color(const struct WuiColor *color,
                                                     const struct WuiEnv *env);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiStr waterui_force_as_label(struct WuiAnyView *view);

struct WuiStr waterui_label_id(void);

struct WuiStr waterui_empty_id(void);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * The pointer must be a valid pointer to a properly initialized value
 * of the expected type, and must not be used after this function is called.
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
struct WuiContainer waterui_force_as_container(struct WuiAnyView *view);

struct WuiStr waterui_container_id(void);

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
                                                       struct WuiArray_WuiChildMetadata children);

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
                                   struct WuiArray_WuiChildMetadata children);

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
struct WuiArray_WuiRect waterui_layout_place(struct WuiLayout *layout,
                                             struct WuiRect bound,
                                             struct WuiProposalSize proposal,
                                             struct WuiArray_WuiChildMetadata children);

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
 * Drops the FFI value.
 *
 * # Safety
 *
 * The pointer must be a valid pointer to a properly initialized value
 * of the expected type, and must not be used after this function is called.
 */
void waterui_drop_font(struct WuiFont *value);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_styled_str(struct Computed_StyledStr *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiStyledStr waterui_read_computed_styled_str(const struct Computed_StyledStr *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_styled_str(const struct Computed_StyledStr *computed,
                                                          struct WuiWatcher_WuiStyledStr watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_font(struct Computed_Font *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiFont *waterui_read_computed_font(const struct Computed_Font *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_font(const struct Computed_Font *computed,
                                                    struct WuiWatcher_____WuiFont watcher);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiText waterui_force_as_text(struct WuiAnyView *view);

struct WuiStr waterui_text_id(void);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_resolved_font(struct Computed_ResolvedFont *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiResolvedFont waterui_read_computed_resolved_font(const struct Computed_ResolvedFont *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_resolved_font(const struct Computed_ResolvedFont *computed,
                                                             struct WuiWatcher_WuiResolvedFont watcher);

struct Computed_ResolvedFont *waterui_resolve_font(const struct WuiFont *font,
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

struct WuiStr waterui_navigation_view_id(void);

struct WuiStr waterui_navigation_link_id(void);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * The pointer must be a valid pointer to a properly initialized value
 * of the expected type, and must not be used after this function is called.
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

struct WuiStr waterui_live_photo_source_id(void);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * The pointer must be a valid pointer to a properly initialized value
 * of the expected type, and must not be used after this function is called.
 */
void waterui_drop_dynamic(struct WuiDynamic *value);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiDynamic *waterui_force_as_dynamic(struct WuiAnyView *view);

struct WuiStr waterui_dynamic_id(void);

void waterui_dynamic_connect(struct WuiDynamic *dynamic, struct WuiWatcher_____WuiAnyView watcher);

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

void waterui_list_item_call_delete(struct WuiListItem *item,
                                   const struct WuiEnv *env,
                                   uintptr_t index);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_table_cols(struct Computed_Vec_TableColumn *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiArray_WuiTableColumn waterui_read_computed_table_cols(const struct Computed_Vec_TableColumn *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_table_cols(const struct Computed_Vec_TableColumn *computed,
                                                          struct WuiWatcher_WuiArray_WuiTableColumn watcher);

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
 * Drops the FFI value.
 *
 * # Safety
 *
 * The pointer must be a valid pointer to a properly initialized value
 * of the expected type, and must not be used after this function is called.
 */
void waterui_drop_renderer_view(struct WuiRendererView *value);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiRendererView *waterui_force_as_renderer_view(struct WuiAnyView *view);

struct WuiStr waterui_renderer_view_id(void);

float waterui_renderer_view_width(const struct WuiRendererView *view);

float waterui_renderer_view_height(const struct WuiRendererView *view);

enum WuiRendererBufferFormat waterui_renderer_view_preferred_format(const struct WuiRendererView *_view);

bool waterui_renderer_view_render_cpu(struct WuiRendererView *view,
                                      uint8_t *pixels,
                                      uint32_t width,
                                      uint32_t height,
                                      uintptr_t stride,
                                      enum WuiRendererBufferFormat format);

/**
 * Releases a gesture descriptor tree allocated by Rust.
 *
 * # Safety
 *
 * The pointer must either be null or point to a gesture obtained through
 * `waterui_force_as_gesture` or any conversion that returns a `WuiGesture` pointer.
 */
void waterui_drop_gesture(struct WuiGesture *value);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiGestureMetadata waterui_force_as_gesture(struct WuiAnyView *view);

struct WuiStr waterui_gesture_id(void);

/**
 * Calls a gesture action with the provided event payload.
 *
 * # Safety
 *
 * * `action` must be a valid pointer to an existing `WuiAction`.
 * * `env` must be a valid pointer to an existing `WuiEnv`.
 */
void waterui_call_gesture_action(struct WuiAction *action,
                                 const struct WuiEnv *env,
                                 struct WuiGestureEvent event);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * The pointer must be a valid pointer to a properly initialized value
 * of the expected type, and must not be used after this function is called.
 */
void waterui_drop_watcher_metadata(struct WuiWatcherMetadata *value);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * The pointer must be a valid pointer to a properly initialized value
 * of the expected type, and must not be used after this function is called.
 */
void waterui_drop_box_watcher_guard(struct WuiWatcherGuard *value);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_str(struct Computed_Str *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiStr waterui_read_computed_str(const struct Computed_Str *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_str(const struct Computed_Str *computed,
                                                   struct WuiWatcher_WuiStr watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_any_view(struct Computed_AnyView *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiAnyView *waterui_read_computed_any_view(const struct Computed_AnyView *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_any_view(const struct Computed_AnyView *computed,
                                                        struct WuiWatcher_____WuiAnyView watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_int(struct Computed_i32 *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
int32_t waterui_read_computed_int(const struct Computed_i32 *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_int(const struct Computed_i32 *computed,
                                                   struct WuiWatcher_i32 watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_bool(struct Computed_bool *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
bool waterui_read_computed_bool(const struct Computed_bool *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_bool(const struct Computed_bool *computed,
                                                    struct WuiWatcher_bool watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_float(struct Computed_f32 *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
float waterui_read_computed_float(const struct Computed_f32 *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_float(const struct Computed_f32 *computed,
                                                     struct WuiWatcher_f32 watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_double(struct Computed_f64 *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
double waterui_read_computed_double(const struct Computed_f64 *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_double(const struct Computed_f64 *computed,
                                                      struct WuiWatcher_f64 watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_picker_items(struct Computed_Vec_PickerItem_Id *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiArray_WuiPickerItem waterui_read_computed_picker_items(const struct Computed_Vec_PickerItem_Id *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_picker_items(const struct Computed_Vec_PickerItem_Id *computed,
                                                            struct WuiWatcher_WuiArray_WuiPickerItem watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_video(struct Computed_Video *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiVideo waterui_read_computed_video(const struct Computed_Video *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_video(const struct Computed_Video *computed,
                                                     struct WuiWatcher_WuiVideo watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_live_photo_sources(struct Computed_LivePhotoSource *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiLivePhotoSource waterui_read_computed_live_photo_source(const struct Computed_LivePhotoSource *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_live_photo_source(const struct Computed_LivePhotoSource *computed,
                                                                 struct WuiWatcher_WuiLivePhotoSource watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_binding_str(struct Binding_Str *value);

/**
 * Reads the current value from a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiStr waterui_read_binding_str(const struct Binding_Str *binding);

/**
 * Sets a new value to a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The value must be a valid instance of the FFI type.
 */
void waterui_set_binding_str(struct Binding_Str *binding, struct WuiStr value);

/**
 * Watches for changes in a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_binding_str(const struct Binding_Str *binding,
                                                  struct WuiWatcher_WuiStr watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_binding_float(struct Binding_f32 *value);

/**
 * Reads the current value from a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
float waterui_read_binding_float(const struct Binding_f32 *binding);

/**
 * Sets a new value to a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The value must be a valid instance of the FFI type.
 */
void waterui_set_binding_float(struct Binding_f32 *binding, float value);

/**
 * Watches for changes in a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_binding_float(const struct Binding_f32 *binding,
                                                    struct WuiWatcher_f32 watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_binding_double(struct Binding_f64 *value);

/**
 * Reads the current value from a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
double waterui_read_binding_double(const struct Binding_f64 *binding);

/**
 * Sets a new value to a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The value must be a valid instance of the FFI type.
 */
void waterui_set_binding_double(struct Binding_f64 *binding, double value);

/**
 * Watches for changes in a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_binding_double(const struct Binding_f64 *binding,
                                                     struct WuiWatcher_f64 watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_binding_int(struct Binding_i32 *value);

/**
 * Reads the current value from a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
int32_t waterui_read_binding_int(const struct Binding_i32 *binding);

/**
 * Sets a new value to a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The value must be a valid instance of the FFI type.
 */
void waterui_set_binding_int(struct Binding_i32 *binding, int32_t value);

/**
 * Watches for changes in a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_binding_int(const struct Binding_i32 *binding,
                                                  struct WuiWatcher_i32 watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_binding_bool(struct Binding_bool *value);

/**
 * Reads the current value from a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
bool waterui_read_binding_bool(const struct Binding_bool *binding);

/**
 * Sets a new value to a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The value must be a valid instance of the FFI type.
 */
void waterui_set_binding_bool(struct Binding_bool *binding, bool value);

/**
 * Watches for changes in a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_binding_bool(const struct Binding_bool *binding,
                                                   struct WuiWatcher_bool watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_binding_id(struct Binding_Id *value);

/**
 * Reads the current value from a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 */
struct WuiId waterui_read_binding_id(const struct Binding_Id *binding);

/**
 * Sets a new value to a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The value must be a valid instance of the FFI type.
 */
void waterui_set_binding_id(struct Binding_Id *binding, struct WuiId value);

/**
 * Watches for changes in a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_binding_id(const struct Binding_Id *binding,
                                                 struct WuiWatcher_WuiId watcher);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * The pointer must be a valid pointer to a properly initialized value
 * of the expected type, and must not be used after this function is called.
 */
void waterui_drop_anyviews(struct WuiAnyViews *value);

struct WuiId waterui_anyviews_get_id(const struct WuiAnyViews *anyviews, uintptr_t index);

struct WuiAnyView *waterui_anyviews_get_view(const struct WuiAnyViews *anyview, uintptr_t index);

uintptr_t waterui_anyviews_len(const struct WuiAnyViews *anyviews);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_drop_computed_views(struct Computed_AnyViews_AnyView *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiAnyViews *waterui_read_computed_views(const struct Computed_AnyViews_AnyView *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_views(const struct Computed_AnyViews_AnyView *computed,
                                                     struct WuiWatcher_____WuiAnyViews watcher);

WuiEnv* waterui_init(void);

WuiAnyView* waterui_main(void);
