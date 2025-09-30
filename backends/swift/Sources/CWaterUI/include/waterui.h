#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>


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

typedef enum WuiColorSpace {
  WuiColorSpace_Srgb,
  WuiColorSpace_P3,
  WuiColorSpace_Invalid,
} WuiColorSpace;

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
typedef struct Computed_Str Computed_Str;

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

/**
 * A type-erased container for metadata that can be associated with computation results.
 *
 * `Metadata` allows attaching arbitrary typed information to computation results
 * and passing it through the computation pipeline.
 */
typedef struct Metadata Metadata;

typedef struct WuiAction WuiAction;

typedef struct WuiAnyView WuiAnyView;

typedef struct WuiDynamic WuiDynamic;

typedef struct WuiEnv WuiEnv;

typedef struct WuiLayout WuiLayout;

typedef struct WuiTabContent WuiTabContent;

typedef struct WuiWatcherGuard WuiWatcherGuard;

typedef struct WuiWatcherMetadata WuiWatcherMetadata;

/**
 * A C-compatible representation of Rust's `core::any::TypeId`.
 *
 * This struct is used for passing TypeId across FFI boundaries.
 */
typedef struct WuiTypeId {
  uint64_t inner[2];
} WuiTypeId;

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
 */
typedef struct WuiArray_u8 {
  uint8_t *_Nonnull data;
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
 * `WuiArray` can represent mutiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
 * or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
 * For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
 */
typedef struct WuiArray_____WuiAnyView {
  struct WuiAnyView **_Nonnull data;
  struct WuiArrayVTable_____WuiAnyView vtable;
} WuiArray_____WuiAnyView;

typedef struct WuiContainer {
  struct WuiLayout *layout;
  struct WuiArray_____WuiAnyView contents;
} WuiContainer;

typedef struct WuiProposalSize {
  double width;
  double height;
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
 */
typedef struct WuiArray_WuiProposalSize {
  struct WuiProposalSize *_Nonnull data;
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
 */
typedef struct WuiArray_WuiChildMetadata {
  struct WuiChildMetadata *_Nonnull data;
  struct WuiArrayVTable_WuiChildMetadata vtable;
} WuiArray_WuiChildMetadata;

typedef struct WuiScrollView {
  enum WuiAxis axis;
  struct WuiAnyView *content;
} WuiScrollView;

typedef struct WuiPoint {
  double x;
  double y;
} WuiPoint;

typedef struct WuiSize {
  double width;
  double height;
} WuiSize;

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
 */
typedef struct WuiArray_WuiRect {
  struct WuiRect *_Nonnull data;
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

typedef struct WuiLink {
  struct WuiAnyView *label;
  struct Computed_Str *url;
} WuiLink;

/**
 * C representation of Text configuration
 */
typedef struct WuiText {
  /**
   * Pointer to the text content computed value
   */
  struct Computed_Str *content;
  /**
   * Pointer to the font computed value
   */
  struct Computed_Font *font;
} WuiText;

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

typedef struct WuiColor {
  enum WuiColorSpace color_space;
  float red;
  float yellow;
  float blue;
  float opacity;
} WuiColor;

typedef struct WuiWatcher_WuiColor {
  void *data;
  void (*call)(const void*, struct WuiColor, const struct Metadata*);
  void (*drop)(void*);
} WuiWatcher_WuiColor;

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
  void (*call)(const void*, struct WuiAnyView*, const struct Metadata*);
  void (*drop)(void*);
} WuiWatcher_____WuiAnyView;

typedef struct WuiProgress {
  struct WuiAnyView *label;
  struct WuiAnyView *value_label;
  struct Computed_f64 *value;
  enum WuiProgressStyle style;
} WuiProgress;

typedef struct WuiWatcher_WuiStr {
  void *data;
  void (*call)(const void*, struct WuiStr, const struct Metadata*);
  void (*drop)(void*);
} WuiWatcher_WuiStr;

typedef struct WuiWatcher_i32 {
  void *data;
  void (*call)(const void*, int32_t, const struct Metadata*);
  void (*drop)(void*);
} WuiWatcher_i32;

typedef struct WuiWatcher_bool {
  void *data;
  void (*call)(const void*, bool, const struct Metadata*);
  void (*drop)(void*);
} WuiWatcher_bool;

typedef struct WuiWatcher_f64 {
  void *data;
  void (*call)(const void*, double, const struct Metadata*);
  void (*drop)(void*);
} WuiWatcher_f64;

/**
 * C representation of Font
 */
typedef struct WuiFont {
  double size;
  bool italic;
  struct WuiColor strikethrough;
  struct WuiColor underlined;
  bool bold;
} WuiFont;

typedef struct WuiWatcher_WuiFont {
  void *data;
  void (*call)(const void*, struct WuiFont, const struct Metadata*);
  void (*drop)(void*);
} WuiWatcher_WuiFont;

typedef struct WuiVideo {
  Url url;
} WuiVideo;

typedef struct WuiWatcher_WuiVideo {
  void *data;
  void (*call)(const void*, struct WuiVideo, const struct Metadata*);
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
  void (*call)(const void*, struct WuiLivePhotoSource, const struct Metadata*);
  void (*drop)(void*);
} WuiWatcher_WuiLivePhotoSource;

typedef struct WuiId {
  int32_t inner;
} WuiId;

typedef struct WuiWatcher_WuiId {
  void *data;
  void (*call)(const void*, struct WuiId, const struct Metadata*);
  void (*drop)(void*);
} WuiWatcher_WuiId;

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
struct WuiTypeId waterui_view_id(const struct WuiAnyView *view);

struct WuiAnyView *waterui_empty_anyview(void);

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

struct WuiTypeId waterui_divider_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiStr waterui_force_as_label(struct WuiAnyView *view);

struct WuiTypeId waterui_label_id(void);

struct WuiTypeId waterui_empty_id(void);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * The pointer must be a valid pointer to a properly initialized value
 * of the expected type, and must not be used after this function is called.
 */
void waterui_drop_layout(struct WuiLayout *value);

struct WuiTypeId waterui_spacer_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiContainer waterui_force_as_container(struct WuiAnyView *view);

struct WuiTypeId waterui_container_id(void);

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
struct WuiProposalSize waterui_layout_size(struct WuiLayout *layout,
                                           struct WuiProposalSize parent,
                                           struct WuiArray_WuiChildMetadata children);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiScrollView waterui_force_as_scroll_view(struct WuiAnyView *view);

struct WuiTypeId waterui_scroll_view_id(void);

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

struct WuiTypeId waterui_button_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiLink waterui_force_as_link(struct WuiAnyView *view);

struct WuiTypeId waterui_link_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiText waterui_force_as_text(struct WuiAnyView *view);

struct WuiTypeId waterui_text_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiTextField waterui_force_as_text_field(struct WuiAnyView *view);

struct WuiTypeId waterui_text_field_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiToggle waterui_force_as_toggle(struct WuiAnyView *view);

struct WuiTypeId waterui_toggle_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiSlider waterui_force_as_slider(struct WuiAnyView *view);

struct WuiTypeId waterui_slider_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiStepper waterui_force_as_stepper(struct WuiAnyView *view);

struct WuiTypeId waterui_stepper_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiColorPicker waterui_force_as_color_picker(struct WuiAnyView *view);

struct WuiTypeId waterui_color_picker_id(void);

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
struct WuiColor waterui_binding_read_color(const struct Binding_Color *binding);

/**
 * Sets a new value to a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The value must be a valid instance of the FFI type.
 */
void waterui_binding_set_color(struct Binding_Color *binding, struct WuiColor value);

/**
 * Watches for changes in a binding
 *
 * # Safety
 *
 * The binding pointer must be valid and point to a properly initialized binding object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_color(const struct Binding_Color *binding,
                                            struct WuiWatcher_WuiColor watcher);

struct WuiTypeId waterui_navigation_view_id(void);

struct WuiTypeId waterui_navigation_link_id(void);

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

struct WuiTypeId waterui_photo_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiVideoPlayer waterui_force_as_video_player(struct WuiAnyView *view);

struct WuiTypeId waterui_video_player_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiLivePhoto waterui_force_as_live_photo(struct WuiAnyView *view);

struct WuiTypeId waterui_live_photo_id(void);

struct WuiTypeId waterui_live_photo_source_id(void);

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

struct WuiTypeId waterui_dynamic_id(void);

void waterui_dynamic_connect(struct WuiDynamic *dynamic, struct WuiWatcher_____WuiAnyView watcher);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiProgress waterui_force_as_progress(struct WuiAnyView *view);

struct WuiTypeId waterui_progress_id(void);

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
void waterui_drop_computed_font(struct Computed_Font *value);

/**
 * Reads the current value from a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 */
struct WuiFont waterui_read_computed_font(const struct Computed_Font *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_font(const struct Computed_Font *computed,
                                                    struct WuiWatcher_WuiFont watcher);

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
struct WuiColor waterui_read_computed_color(const struct Computed_Color *computed);

/**
 * Watches for changes in a computed
 *
 * # Safety
 *
 * The computed pointer must be valid and point to a properly initialized computed object.
 * The watcher must be a valid callback function.
 */
struct WuiWatcherGuard *waterui_watch_computed_color(const struct Computed_Color *computed,
                                                     struct WuiWatcher_WuiColor watcher);

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
void waterui_drop_watcher_metadata(struct WuiWatcherMetadata *value);

WuiEnv* waterui_init(void);

WuiAnyView* waterui_main(void);

