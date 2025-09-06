#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

typedef enum Alignment {
  ALIGNMENT_DEFAULT,
  ALIGNMENT_LEADING,
  ALIGNMENT_CENTER,
  ALIGNMENT_TRAILING,
} Alignment;

typedef enum ColorSpace {
  COLOR_SPACE_S_RGB,
  COLOR_SPACE_P3,
} ColorSpace;

typedef enum StackMode {
  STACK_MODE_VERTICAL,
  STACK_MODE_HORIZONAL,
  STACK_MODE_LAYERED,
} StackMode;

typedef enum waterui_animation {
  WATERUI_ANIMATION_DEFAULT,
  WATERUI_ANIMATION_NONE,
} waterui_animation;

typedef enum waterui_axis {
  WATERUI_AXIS_HORIZONTAL,
  WATERUI_AXIS_VERTICAL,
  WATERUI_AXIS_ALL,
} waterui_axis;

typedef enum waterui_style_progress {
  WATERUI_STYLE_PROGRESS_DEFAULT,
  WATERUI_STYLE_PROGRESS_CIRCULAR,
  WATERUI_STYLE_PROGRESS_LINEAR,
} waterui_style_progress;

typedef struct waterui_action waterui_action;

typedef struct waterui_anyview waterui_anyview;

typedef struct waterui_anyview_iter waterui_anyview_iter;

typedef struct waterui_binding_bool waterui_binding_bool;

typedef struct waterui_binding_color waterui_binding_color;

typedef struct waterui_binding_double waterui_binding_double;

typedef struct waterui_binding_id waterui_binding_id;

typedef struct waterui_binding_int waterui_binding_int;

typedef struct waterui_binding_str waterui_binding_str;

typedef struct waterui_computed_bool waterui_computed_bool;

typedef struct waterui_computed_color waterui_computed_color;

typedef struct waterui_computed_data waterui_computed_data;

typedef struct waterui_computed_double waterui_computed_double;

typedef struct waterui_computed_frame waterui_computed_frame;

typedef struct waterui_computed_int waterui_computed_int;

typedef struct waterui_computed_picker_items waterui_computed_picker_items;

typedef struct waterui_computed_str waterui_computed_str;

typedef struct waterui_dynamic_view waterui_dynamic_view;

typedef struct waterui_env waterui_env;

typedef struct waterui_lazy_view_list waterui_lazy_view_list;

typedef struct waterui_navigation_view_builder waterui_navigation_view_builder;

typedef struct waterui_watcher_guard waterui_watcher_guard;

typedef struct waterui_watcher_metadata waterui_watcher_metadata;

typedef struct waterui_type_id {
  uint64_t inner[2];
} waterui_type_id;

typedef struct waterui_button {
  struct waterui_anyview *label;
  struct waterui_action *action;
} waterui_button;

typedef struct Color {
  enum ColorSpace space;
  double red;
  double green;
  double blue;
  double opacity;
} Color;

typedef struct Color waterui_color;

typedef struct waterui_watcher_waterui_color {
  void *data;
  void (*call)(const void*, waterui_color, const struct waterui_watcher_metadata*);
  void (*drop)(void*);
} waterui_watcher_waterui_color;

typedef struct waterui_background_color {
  struct waterui_computed_color *color;
} waterui_background_color;

typedef struct waterui_metadata_waterui_background_color {
  struct waterui_anyview *content;
  struct waterui_background_color value;
} waterui_metadata_waterui_background_color;

typedef struct waterui_foreground_color {
  struct waterui_computed_color *color;
} waterui_foreground_color;

typedef struct waterui_metadata_waterui_foreground_color {
  struct waterui_anyview *content;
  struct waterui_foreground_color value;
} waterui_metadata_waterui_foreground_color;

typedef struct waterui_nothing {
  uint8_t _nothing;
} waterui_nothing;

typedef struct waterui_divider {
  struct waterui_nothing _0;
} waterui_divider;

typedef struct waterui_fn_____waterui_anyview {
  void *data;
  void (*call)(const void*, struct waterui_anyview*);
  void (*drop)(void*);
} waterui_fn_____waterui_anyview;

typedef struct waterui_icon {
  struct waterui_computed_str *name;
  struct waterui_computed_double *size;
} waterui_icon;

typedef struct waterui_array_u8 {
  uint8_t *head;
  uintptr_t len;
} waterui_array_u8;

typedef struct waterui_array_u8 waterui_data;

typedef struct waterui_watcher_waterui_data {
  void *data;
  void (*call)(const void*, waterui_data, const struct waterui_watcher_metadata*);
  void (*drop)(void*);
} waterui_watcher_waterui_data;

typedef struct waterui_image {
  struct waterui_computed_data *data;
} waterui_image;

typedef struct waterui_scroll {
  struct waterui_anyview *content;
  enum waterui_axis axis;
} waterui_scroll;

typedef struct waterui_spacer {
  struct waterui_nothing _0;
} waterui_spacer;

typedef struct waterui_array_____waterui_anyview {
  struct waterui_anyview **head;
  uintptr_t len;
} waterui_array_____waterui_anyview;

typedef enum StackMode waterui_stack_mode;

typedef struct waterui_stack {
  struct waterui_array_____waterui_anyview contents;
  waterui_stack_mode mode;
} waterui_stack;

typedef struct waterui_fnonce_____waterui_anyview {
  void *data;
  void (*call)(void*, struct waterui_anyview*);
} waterui_fnonce_____waterui_anyview;

typedef struct waterui_list {
  struct waterui_lazy_view_list *contents;
} waterui_list;

typedef struct waterui_metadata_____waterui_env {
  struct waterui_anyview *content;
  struct waterui_env *value;
} waterui_metadata_____waterui_env;

typedef struct waterui_metadata_____waterui_computed_frame {
  struct waterui_anyview *content;
  struct waterui_computed_frame *value;
} waterui_metadata_____waterui_computed_frame;

typedef struct Edge {
  double top;
  double right;
  double bottom;
  double left;
} Edge;

typedef struct Frame {
  double width;
  double min_width;
  double max_width;
  double height;
  double min_height;
  double max_height;
  struct Edge margin;
  enum Alignment alignment;
} Frame;

typedef struct Frame waterui_frame;

typedef struct waterui_watcher_waterui_frame {
  void *data;
  void (*call)(const void*, waterui_frame, const struct waterui_watcher_metadata*);
  void (*drop)(void*);
} waterui_watcher_waterui_frame;

typedef struct Edge waterui_edge;

typedef struct waterui_metadata_waterui_edge {
  struct waterui_anyview *content;
  waterui_edge value;
} waterui_metadata_waterui_edge;

typedef struct waterui_text {
  struct waterui_computed_str *content;
} waterui_text;

typedef struct waterui_bar {
  struct waterui_text title;
  struct waterui_computed_bool *hidden;
} waterui_bar;

typedef struct waterui_navigation_view {
  struct waterui_bar bar;
  struct waterui_anyview *content;
} waterui_navigation_view;

typedef struct waterui_navigation_link {
  struct waterui_anyview *label;
  struct waterui_navigation_view_builder *content;
} waterui_navigation_link;

typedef struct waterui_picker_item {
  struct waterui_text label;
  int32_t tag;
} waterui_picker_item;

typedef struct waterui_array_waterui_picker_item {
  struct waterui_picker_item *head;
  uintptr_t len;
} waterui_array_waterui_picker_item;

typedef struct waterui_watcher_waterui_array_waterui_picker_item {
  void *data;
  void (*call)(const void*,
               struct waterui_array_waterui_picker_item,
               const struct waterui_watcher_metadata*);
  void (*drop)(void*);
} waterui_watcher_waterui_array_waterui_picker_item;

typedef struct waterui_picker {
  struct waterui_computed_picker_items *items;
  struct waterui_binding_id *selection;
} waterui_picker;

typedef struct waterui_color_picker {
  struct waterui_anyview *label;
  struct waterui_binding_color *value;
} waterui_color_picker;

typedef struct waterui_progress {
  struct waterui_anyview *label;
  struct waterui_computed_double *value;
  enum waterui_style_progress style;
} waterui_progress;

typedef struct waterui_rectangle {
  struct waterui_nothing _0;
} waterui_rectangle;

typedef struct waterui_rounded_rectangle {
  struct waterui_computed_double *radius;
} waterui_rounded_rectangle;

typedef struct waterui_circle {
  struct waterui_nothing _0;
} waterui_circle;

typedef struct waterui_range_inclusive_f64 {
  double start;
  double end;
} waterui_range_inclusive_f64;

typedef struct waterui_slider {
  struct waterui_anyview *label;
  struct waterui_anyview *min_value_label;
  struct waterui_anyview *max_value_label;
  struct waterui_range_inclusive_f64 range;
  struct waterui_binding_double *value;
} waterui_slider;

typedef struct waterui_stepper {
  const struct waterui_binding_int *value;
  struct waterui_computed_int *step;
} waterui_stepper;

typedef struct waterui_tab {
  struct waterui_anyview *label;
  int32_t tag;
  struct waterui_navigation_view_builder *content;
} waterui_tab;

typedef struct waterui_array_waterui_tab {
  struct waterui_tab *head;
  uintptr_t len;
} waterui_array_waterui_tab;

typedef struct waterui_tabs {
  struct waterui_binding_id *selection;
  struct waterui_array_waterui_tab tabs;
} waterui_tabs;

typedef struct waterui_text_field {
  struct waterui_anyview *label;
  struct waterui_binding_str *value;
  struct waterui_text prompt;
} waterui_text_field;

typedef struct waterui_toggle {
  struct waterui_anyview *label;
  const struct waterui_binding_bool *toggle;
} waterui_toggle;

typedef struct waterui_str {
  const void *ptr;
  uintptr_t len;
} waterui_str;

typedef struct waterui_watcher_waterui_str {
  void *data;
  void (*call)(const void*, struct waterui_str, const struct waterui_watcher_metadata*);
  void (*drop)(void*);
} waterui_watcher_waterui_str;

typedef struct waterui_watcher_f64 {
  void *data;
  void (*call)(const void*, double, const struct waterui_watcher_metadata*);
  void (*drop)(void*);
} waterui_watcher_f64;

typedef struct waterui_watcher_i32 {
  void *data;
  void (*call)(const void*, int32_t, const struct waterui_watcher_metadata*);
  void (*drop)(void*);
} waterui_watcher_i32;

typedef struct waterui_watcher_bool {
  void *data;
  void (*call)(const void*, bool, const struct waterui_watcher_metadata*);
  void (*drop)(void*);
} waterui_watcher_bool;

void waterui_drop_watcher_guard(struct waterui_watcher_guard *value);

void waterui_drop_binding_id(struct waterui_binding_id *value);

struct waterui_type_id waterui_view_id(const struct waterui_anyview *view);

struct waterui_type_id waterui_view_empty_id(void);

struct waterui_anyview *waterui_view_body(struct waterui_anyview *view, struct waterui_env *env);

struct waterui_button waterui_view_force_as_button(struct waterui_anyview *view);

struct waterui_type_id waterui_view_button_id(void);

void waterui_drop_binding_color(struct waterui_binding_color *value);

waterui_color waterui_read_binding_color(const struct waterui_binding_color *binding);

void waterui_set_binding_color(struct waterui_binding_color *binding, waterui_color value);

struct waterui_watcher_guard *waterui_watch_binding_color(const struct waterui_binding_color *binding,
                                                          struct waterui_watcher_waterui_color watcher);

void waterui_drop_computed_color(struct waterui_computed_color *value);

waterui_color waterui_read_computed_color(const struct waterui_computed_color *computed);

struct waterui_watcher_guard *waterui_watch_computed_color(const struct waterui_computed_color *computed,
                                                           struct waterui_watcher_waterui_color watcher);

struct waterui_metadata_waterui_background_color waterui_metadata_force_as_background_color(struct waterui_anyview *view);

struct waterui_type_id waterui_metadata_background_color_id(void);

struct waterui_metadata_waterui_foreground_color waterui_metadata_force_as_foreground_color(struct waterui_anyview *view);

struct waterui_type_id waterui_metadata_foreground_color_id(void);

struct waterui_divider waterui_view_force_as_divider(struct waterui_anyview *view);

struct waterui_type_id waterui_view_divider_id(void);

void waterui_drop_dynamic_view(struct waterui_dynamic_view *value);

struct waterui_dynamic_view *waterui_view_force_as_dynamic(struct waterui_anyview *view);

struct waterui_type_id waterui_view_dynamic_id(void);

void waterui_dynamic_view_connect(struct waterui_dynamic_view *dyanmic,
                                  struct waterui_fn_____waterui_anyview f);

struct waterui_icon waterui_view_force_as_icon(struct waterui_anyview *view);

struct waterui_type_id waterui_view_icon_id(void);

void waterui_drop_computed_data(struct waterui_computed_data *value);

waterui_data waterui_read_computed_data(const struct waterui_computed_data *computed);

struct waterui_watcher_guard *waterui_watch_computed_data(const struct waterui_computed_data *computed,
                                                          struct waterui_watcher_waterui_data watcher);

struct waterui_image waterui_view_force_as_image(struct waterui_anyview *view);

struct waterui_type_id waterui_view_image_id(void);

struct waterui_scroll waterui_view_force_as_scroll(struct waterui_anyview *view);

struct waterui_type_id waterui_view_scroll_id(void);

struct waterui_spacer waterui_view_force_as_spacer(struct waterui_anyview *view);

struct waterui_type_id waterui_view_spacer_id(void);

struct waterui_stack waterui_view_force_as_stack(struct waterui_anyview *view);

struct waterui_type_id waterui_view_stack_id(void);

void waterui_drop_lazy_view_list(struct waterui_lazy_view_list *value);

void waterui_lazy_view_list_get(const struct waterui_lazy_view_list *list,
                                uintptr_t index,
                                struct waterui_fnonce_____waterui_anyview callback);

int32_t waterui_lazy_list_len(const struct waterui_lazy_view_list *list);

void waterui_drop_anyview_iter(struct waterui_anyview_iter *value);

void waterui_anyview_iter_next(struct waterui_anyview_iter *iter,
                               struct waterui_fnonce_____waterui_anyview callback);

struct waterui_anyview_iter *waterui_lazy_list_iter(const struct waterui_lazy_view_list *list);

struct waterui_anyview_iter *waterui_lazy_list_rev_iter(const struct waterui_lazy_view_list *list);

struct waterui_list waterui_view_force_as_list(struct waterui_anyview *view);

struct waterui_type_id waterui_view_list_id(void);

struct waterui_metadata_____waterui_env waterui_metadata_force_as_env(struct waterui_anyview *view);

struct waterui_type_id waterui_metadata_env_id(void);

struct waterui_metadata_____waterui_computed_frame waterui_metadata_force_as_frame(struct waterui_anyview *view);

struct waterui_type_id waterui_metadata_frame_id(void);

void waterui_drop_computed_frame(struct waterui_computed_frame *value);

waterui_frame waterui_read_computed_frame(const struct waterui_computed_frame *computed);

struct waterui_watcher_guard *waterui_watch_computed_frame(const struct waterui_computed_frame *computed,
                                                           struct waterui_watcher_waterui_frame watcher);

struct waterui_metadata_waterui_edge waterui_metadata_force_as_padding(struct waterui_anyview *view);

struct waterui_type_id waterui_metadata_padding_id(void);

void waterui_drop_navigation_view_builder(struct waterui_navigation_view_builder *value);

struct waterui_navigation_view waterui_navigation_view_builder_call(const struct waterui_navigation_view_builder *content,
                                                                    struct waterui_env *env);

struct waterui_navigation_view waterui_view_force_as_navigation_view(struct waterui_anyview *view);

struct waterui_type_id waterui_view_navigation_view_id(void);

struct waterui_navigation_link waterui_view_force_as_navigation_link(struct waterui_anyview *view);

struct waterui_type_id waterui_view_navigation_link_id(void);

void waterui_drop_computed_picker_items(struct waterui_computed_picker_items *value);

struct waterui_array_waterui_picker_item waterui_read_computed_picker_items(const struct waterui_computed_picker_items *computed);

struct waterui_watcher_guard *waterui_watch_computed_picker_items(const struct waterui_computed_picker_items *computed,
                                                                  struct waterui_watcher_waterui_array_waterui_picker_item watcher);

struct waterui_picker waterui_view_force_as_picker(struct waterui_anyview *view);

struct waterui_type_id waterui_view_picker_id(void);

struct waterui_color_picker waterui_view_force_as_color_picker(struct waterui_anyview *view);

struct waterui_type_id waterui_view_color_picker_id(void);

struct waterui_progress waterui_view_force_as_progress(struct waterui_anyview *view);

struct waterui_type_id waterui_view_progress_id(void);

struct waterui_rectangle waterui_view_force_as_rectangle(struct waterui_anyview *view);

struct waterui_type_id waterui_view_rectangle_id(void);

struct waterui_rounded_rectangle waterui_view_force_as_rounded_rectangle(struct waterui_anyview *view);

struct waterui_type_id waterui_view_rounded_rectangle_id(void);

struct waterui_circle waterui_view_force_as_circle(struct waterui_anyview *view);

struct waterui_type_id waterui_view_circle_id(void);

struct waterui_slider waterui_view_force_as_slider(struct waterui_anyview *view);

struct waterui_type_id waterui_view_slider_id(void);

struct waterui_stepper waterui_view_force_as_stepper(struct waterui_anyview *view);

struct waterui_type_id waterui_view_stepper_id(void);

struct waterui_tabs waterui_view_force_as_tabs(struct waterui_anyview *view);

struct waterui_type_id waterui_view_tabs_id(void);

struct waterui_text waterui_view_force_as_text(struct waterui_anyview *view);

struct waterui_type_id waterui_view_text_id(void);

struct waterui_text_field waterui_view_force_as_text_field(struct waterui_anyview *view);

struct waterui_type_id waterui_view_text_field_id(void);

struct waterui_toggle waterui_view_force_as_toggle(struct waterui_anyview *view);

struct waterui_type_id waterui_view_toggle_id(void);

enum waterui_animation waterui_get_animation(const struct waterui_watcher_metadata *metadata);

const uint8_t *waterui_str_get_head(struct waterui_str s);

struct waterui_str waterui_new_str(const uint8_t *head, uintptr_t len);

void waterui_free_str(struct waterui_str s);

void waterui_free_array(uint8_t *ptr, uintptr_t size);

void waterui_drop_binding_str(struct waterui_binding_str *value);

struct waterui_str waterui_read_binding_str(const struct waterui_binding_str *binding);

void waterui_set_binding_str(struct waterui_binding_str *binding, struct waterui_str value);

struct waterui_watcher_guard *waterui_watch_binding_str(const struct waterui_binding_str *binding,
                                                        struct waterui_watcher_waterui_str watcher);

void waterui_drop_binding_double(struct waterui_binding_double *value);

double waterui_read_binding_double(const struct waterui_binding_double *binding);

void waterui_set_binding_double(struct waterui_binding_double *binding, double value);

struct waterui_watcher_guard *waterui_watch_binding_double(const struct waterui_binding_double *binding,
                                                           struct waterui_watcher_f64 watcher);

void waterui_drop_binding_int(struct waterui_binding_int *value);

int32_t waterui_read_binding_int(const struct waterui_binding_int *binding);

void waterui_set_binding_int(struct waterui_binding_int *binding, int32_t value);

struct waterui_watcher_guard *waterui_watch_binding_int(const struct waterui_binding_int *binding,
                                                        struct waterui_watcher_i32 watcher);

void waterui_drop_binding_bool(struct waterui_binding_bool *value);

bool waterui_read_binding_bool(const struct waterui_binding_bool *binding);

void waterui_set_binding_bool(struct waterui_binding_bool *binding, bool value);

struct waterui_watcher_guard *waterui_watch_binding_bool(const struct waterui_binding_bool *binding,
                                                         struct waterui_watcher_bool watcher);

void waterui_drop_computed_str(struct waterui_computed_str *value);

struct waterui_str waterui_read_computed_str(const struct waterui_computed_str *computed);

struct waterui_watcher_guard *waterui_watch_computed_str(const struct waterui_computed_str *computed,
                                                         struct waterui_watcher_waterui_str watcher);

void waterui_drop_computed_int(struct waterui_computed_int *value);

int32_t waterui_read_computed_int(const struct waterui_computed_int *computed);

struct waterui_watcher_guard *waterui_watch_computed_int(const struct waterui_computed_int *computed,
                                                         struct waterui_watcher_i32 watcher);

void waterui_drop_computed_bool(struct waterui_computed_bool *value);

bool waterui_read_computed_bool(const struct waterui_computed_bool *computed);

struct waterui_watcher_guard *waterui_watch_computed_bool(const struct waterui_computed_bool *computed,
                                                          struct waterui_watcher_bool watcher);

void waterui_drop_computed_double(struct waterui_computed_double *value);

double waterui_read_computed_double(const struct waterui_computed_double *computed);

struct waterui_watcher_guard *waterui_watch_computed_double(const struct waterui_computed_double *computed,
                                                            struct waterui_watcher_f64 watcher);

void waterui_drop_anyview(struct waterui_anyview *value);

void waterui_drop_env(struct waterui_env *value);

struct waterui_env *waterui_clone_env(const struct waterui_env *env);

void waterui_drop_watcher_metadata(struct waterui_watcher_metadata *value);

void waterui_drop_action(struct waterui_action *value);

void waterui_call_action(struct waterui_action *action, struct waterui_env *env);

waterui_env *waterui_init();
waterui_anyview *waterui_widget_main();
