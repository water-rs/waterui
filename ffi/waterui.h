#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

typedef enum WuiAlignment {
  WuiAlignment_Default,
  WuiAlignment_Leading,
  WuiAlignment_Center,
  WuiAlignment_Trailing,
} WuiAlignment;

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

typedef enum WuiStackMode {
  WuiStackMode_Vertical,
  WuiStackMode_Horizonal,
  WuiStackMode_Layered,
} WuiStackMode;

/**
 * A type-erased wrapper for a `View`.
 *
 * This allows storing and passing around different view types uniformly.
 */
typedef struct AnyView AnyView;

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
 * A type-erased container for metadata that can be associated with computation results.
 *
 * `Metadata` allows attaching arbitrary typed information to computation results
 * and passing it through the computation pipeline.
 */
typedef struct Metadata Metadata;

/**
 * A string type that can be either a static reference or a ref-counted owned string.
 *
 * `Str` combines the benefits of both `&'static str` and `String` with efficient
 * cloning and passing, automatically using the most appropriate representation
 * based on the source.
 */
typedef struct Str Str;

typedef struct WuiAction WuiAction;

typedef struct WuiWatcherGuard WuiWatcherGuard;

typedef struct WuiWatcherMetadata WuiWatcherMetadata;

typedef struct waterui_env waterui_env;

/**
 * A C-compatible representation of Rust's `core::any::TypeId`.
 *
 * This struct is used for passing TypeId across FFI boundaries.
 */
typedef struct WuiTypeId {
  uint64_t inner[2];
} WuiTypeId;

typedef struct WuiStr {
  void *ptr;
  intptr_t len;
} WuiStr;

/**
 * A C-compatible array structure that wraps a pointer and length.
 *
 * This type is used as an FFI-compatible representation of Rust collections.
 */
typedef struct WuiArray_____AnyView {
  struct AnyView **head;
  uintptr_t len;
} WuiArray_____AnyView;

typedef struct WuiStack {
  struct WuiArray_____AnyView contents;
  enum WuiStackMode mode;
} WuiStack;

typedef struct WuiGridRow {
  struct WuiArray_____AnyView columns;
} WuiGridRow;

/**
 * A C-compatible array structure that wraps a pointer and length.
 *
 * This type is used as an FFI-compatible representation of Rust collections.
 */
typedef struct WuiArray_WuiGridRow {
  struct WuiGridRow *head;
  uintptr_t len;
} WuiArray_WuiGridRow;

typedef struct WuiGrid {
  enum WuiAlignment alignment;
  double h_space;
  double v_space;
  struct WuiArray_WuiGridRow rows;
} WuiGrid;

typedef struct WuiScrollView {
  struct AnyView *content;
  enum WuiAxis axis;
} WuiScrollView;

typedef struct WuiOverlay {
  struct AnyView *content;
} WuiOverlay;

/**
 * C representation of a WaterUI button for FFI purposes.
 */
typedef struct WuiButton {
  /**
   * Pointer to the button's label view
   */
  struct AnyView *label;
  /**
   * Pointer to the button's action handler
   */
  struct WuiAction *action;
} WuiButton;

typedef struct WuiLink {
  struct AnyView *label;
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
  struct AnyView *label;
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
  struct AnyView *label;
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
  struct AnyView *label;
  /**
   * Pointer to the minimum value label view
   */
  struct AnyView *min_value_label;
  /**
   * Pointer to the maximum value label view
   */
  struct AnyView *max_value_label;
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
  struct AnyView *label;
  /**
   * The valid range of values
   */
  struct WuiRange_i32 range;
} WuiStepper;

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
  struct AnyView *placeholder;
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

typedef struct WuiColor {
  enum WuiColorSpace color_space;
  float red;
  float yellow;
  float blue;
  float opacity;
} WuiColor;

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

typedef struct WuiWatcher_WuiColor {
  void *data;
  void (*call)(const void*, struct WuiColor, const struct Metadata*);
  void (*drop)(void*);
} WuiWatcher_WuiColor;

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
void waterui_env_drop(struct waterui_env *value);

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
void waterui_call_action(struct WuiAction *action, const struct waterui_env *env);

enum WuiAnimation waterui_get_animation(const struct WuiWatcherMetadata *metadata);

struct WuiTypeId waterui_divider_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiStr waterui_label_id(struct AnyView *view);

struct WuiTypeId waterui_force_as_label(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiStack waterui_force_as_stack(struct AnyView *view);

struct WuiTypeId waterui_stack_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiGrid waterui_force_as_grid(struct AnyView *view);

struct WuiTypeId waterui_grid_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiScrollView waterui_force_as_scroll(struct AnyView *view);

struct WuiTypeId waterui_scroll_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiOverlay waterui_force_as_overlay(struct AnyView *view);

struct WuiTypeId waterui_overlay_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiButton waterui_force_as_button(struct AnyView *view);

struct WuiTypeId waterui_button_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiLink waterui_force_as_link(struct AnyView *view);

struct WuiTypeId waterui_link_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiText waterui_force_as_text(struct AnyView *view);

struct WuiTypeId waterui_text_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiTextField waterui_force_as_text_field(struct AnyView *view);

struct WuiTypeId waterui_text_field_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiToggle waterui_force_as_toggle(struct AnyView *view);

struct WuiTypeId waterui_toggle_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiSlider waterui_force_as_slider(struct AnyView *view);

struct WuiTypeId waterui_slider_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiStepper waterui_force_as_stepper(struct AnyView *view);

struct WuiTypeId waterui_stepper_id(void);

struct WuiTypeId waterui_navigation_view_id(void);

struct WuiTypeId waterui_navigation_link_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiPhoto waterui_force_as_photo(struct AnyView *view);

struct WuiTypeId waterui_photo_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiVideoPlayer waterui_force_as_video_player(struct AnyView *view);

struct WuiTypeId waterui_video_player_id(void);

/**
 * # Safety
 * This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
 * The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
 */
struct WuiLivePhoto waterui_force_as_live_photo(struct AnyView *view);

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

/**
 * Creates a new empty Str instance.
 *
 * # Returns
 *
 * A new empty Str instance.
 */
struct WuiStr waterui_str_new(void);

/**
 * Creates a new Str instance from a C string.
 *
 * # Parameters
 *
 * * `c_str` - A null-terminated C string pointer
 *
 * # Returns
 *
 * A new Str instance containing the content of the C string.
 *
 * # Safety
 *
 * The caller must ensure that:
 * * `c_str` is a valid pointer to a null-terminated C string
 * * `c_str` points to a valid UTF-8 encoded string
 * * The memory referenced by `c_str` remains valid for the duration of this call
 *
 * Undefined behavior (UB) will occur if any of these conditions are violated.
 */
struct WuiStr waterui_str_from_cstr(const char *c_str);

/**
 * Drops the FFI value.
 *
 * # Safety
 *
 * If `value` is NULL, this function does nothing. If `value` is not a valid pointer
 * to a properly initialized value of the expected type, undefined behavior will occur.
 * The pointer must not be used after this function is called.
 */
void waterui_str_drop(struct WuiStr value);

/**
 * Creates a clone of the given Str instance.
 *
 * # Parameters
 *
 * * `str` - A pointer to a valid Str instance
 *
 * # Returns
 *
 * A new Str instance that is a clone of the input Str.
 *
 * # Safety
 *
 * The caller must ensure that `str` is a valid pointer to a Str instance.
 * If `str` is null or invalid, undefined behavior will occur.
 */
struct WuiStr waterui_str_clone(const struct Str *str);

/**
 * Returns the length of the Str in bytes.
 *
 * # Parameters
 *
 * * `str` - A pointer to a valid Str instance
 *
 * # Returns
 *
 * The length of the string in bytes.
 *
 * # Safety
 *
 * The caller must ensure that `str` is a valid pointer to a Str instance.
 * If `str` is null or points to invalid memory, undefined behavior will occur.
 */
unsigned int waterui_str_len(const struct Str *str);

/**
 * Checks if the Str is empty.
 *
 * # Parameters
 *
 * * `str` - A pointer to a valid Str instance
 *
 * # Returns
 *
 * 1 if the string is empty, 0 otherwise.
 *
 * # Safety
 *
 * The caller must ensure that `str` is a valid pointer to a Str instance.
 * If `str` is null or points to invalid memory, undefined behavior will occur.
 */
int waterui_str_is_empty(const struct Str *str);

/**
 * Converts a Str to a C string.
 *
 * # Parameters
 *
 * * `str` - A pointer to a valid Str instance
 *
 * # Returns
 *
 * A pointer to a new null-terminated C string or NULL if conversion fails.
 * The caller is responsible for freeing this memory using the appropriate C function.
 *
 * # Safety
 *
 * The caller must ensure that:
 * * `str` is a valid pointer to a Str instance
 * * The returned C string must be freed by the caller to avoid memory leaks
 *
 * If `str` is null or points to invalid memory, undefined behavior will occur.
 */
char *waterui_str_to_cstr(const struct Str *str);

/**
 * Appends a C string to the end of a Str.
 *
 * # Parameters
 *
 * * `str` - A pointer to a valid Str instance that will be modified
 * * `c_str` - A null-terminated C string to append
 *
 * # Safety
 *
 * The caller must ensure that:
 * * `str` is a valid pointer to a Str instance
 * * `c_str` is a valid pointer to a null-terminated C string
 * * `c_str` points to a valid UTF-8 encoded string
 * * The memory referenced by both pointers remains valid for the duration of this call
 *
 * Undefined behavior will occur if any of these conditions are violated.
 */
void waterui_str_append(struct Str *str, const char *c_str);

/**
 * Concatenates two Str instances.
 *
 * # Parameters
 *
 * * `str1` - A pointer to the first valid Str instance
 * * `str2` - A pointer to the second valid Str instance
 *
 * # Returns
 *
 * A new Str instance that is the concatenation of str1 and str2.
 *
 * # Safety
 *
 * The caller must ensure that both `str1` and `str2` are valid pointers to Str instances.
 * If either pointer is null or points to invalid memory, undefined behavior will occur.
 */
struct WuiStr waterui_str_concat(const struct Str *str1, const struct Str *str2);

/**
 * Compares two Str instances.
 *
 * # Parameters
 *
 * * `str1` - A pointer to the first valid Str instance
 * * `str2` - A pointer to the second valid Str instance
 *
 * # Returns
 *
 * A negative value if str1 < str2, 0 if str1 == str2, and a positive value if str1 > str2.
 *
 * # Safety
 *
 * The caller must ensure that both `str1` and `str2` are valid pointers to Str instances.
 * If either pointer is null or points to invalid memory, undefined behavior will occur.
 */
int waterui_str_compare(const struct Str *str1, const struct Str *str2);

/**
 * Gets the reference count of a Str instance.
 *
 * # Parameters
 *
 * * `str` - A pointer to a valid Str instance
 *
 * # Returns
 *
 * The reference count of the Str, or -1 if the pointer is null, or 0 if the Str doesn't
 * support reference counting.
 *
 * # Safety
 *
 * The caller must ensure that `str` is either null or a valid pointer to a Str instance.
 * If `str` is invalid but not null, undefined behavior will occur.
 */
int waterui_str_ref_count(const struct Str *str);

/**
 * Creates a substring from a Str instance.
 *
 * # Parameters
 *
 * * `str` - A pointer to a valid Str instance
 * * `start` - The starting byte index (inclusive)
 * * `end` - The ending byte index (exclusive)
 *
 * # Returns
 *
 * A new Str instance containing the substring.
 *
 * # Safety
 *
 * The caller must ensure that:
 * * `str` is a valid pointer to a Str instance
 * * `start` and `end` form a valid range within the string's length
 * * The range forms a valid UTF-8 boundary
 *
 * This function uses `get_unchecked` internally, so providing an invalid range will result
 * in undefined behavior.
 */
struct WuiStr waterui_str_substring(const struct Str *str, unsigned int start, unsigned int end);

/**
 * Checks if a Str contains a substring.
 *
 * # Parameters
 *
 * * `str` - A pointer to a valid Str instance to search in
 * * `substring` - A pointer to a valid Str instance to search for
 *
 * # Returns
 *
 * 1 if the substring is found, 0 otherwise.
 *
 * # Safety
 *
 * The caller must ensure that both `str` and `substring` are valid pointers to Str instances.
 * If either pointer is null or points to invalid memory, undefined behavior will occur.
 */
int waterui_str_contains(const struct Str *str, const struct Str *substring);

/**
 * Creates a Str from a byte array.
 *
 * # Parameters
 *
 * * `bytes` - A pointer to a byte array
 * * `len` - The length of the byte array
 *
 * # Returns
 *
 * A new Str instance containing the bytes interpreted as UTF-8.
 *
 * # Safety
 *
 * The caller must ensure that:
 * * `bytes` is a valid pointer to a byte array of at least `len` bytes
 * * The bytes must form a valid UTF-8 string
 * * The memory referenced by `bytes` remains valid for the duration of this call
 *
 * This function uses `from_utf8_unchecked` internally, so providing invalid UTF-8 will result
 * in undefined behavior.
 */
struct WuiStr waterui_str_from_bytes(const char *bytes, unsigned int len);
