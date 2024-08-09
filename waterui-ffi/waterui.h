#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

typedef enum waterui_stack_mode {
  waterui_stack_mode_VERTICAL,
  waterui_stack_mode_HORIZONTAL,
  waterui_stack_mode_LAYERED,
} waterui_stack_mode;

typedef enum waterui_style_progress {
  waterui_style_progress_DEFAULT,
  waterui_style_progress_CIRCULAR,
  waterui_style_progress_LINEAR,
} waterui_style_progress;

typedef struct waterui_action waterui_action;

typedef struct waterui_anyview waterui_anyview;

typedef struct waterui_binding_bool waterui_binding_bool;

typedef struct waterui_binding_int waterui_binding_int;

typedef struct waterui_binding_picker_item_id waterui_binding_picker_item_id;

typedef struct waterui_binding_str waterui_binding_str;

typedef struct waterui_computed_bool waterui_computed_bool;

typedef struct waterui_computed_double waterui_computed_double;

typedef struct waterui_computed_int waterui_computed_int;

typedef struct waterui_computed_picker_items waterui_computed_picker_items;

typedef struct waterui_computed_str waterui_computed_str;

typedef struct waterui_env waterui_env;

typedef struct waterui_watcher_guard waterui_watcher_guard;

typedef struct waterui_type_id {
  uint64_t inner[2];
} waterui_type_id;

typedef struct waterui_button {
  struct waterui_anyview *label;
  struct waterui_action *action;
} waterui_button;

typedef struct waterui_text {
  struct waterui_computed_str *content;
} waterui_text;

typedef struct waterui_picker_item {
  struct waterui_text label;
  uintptr_t tag;
} waterui_picker_item;

typedef struct waterui_array_waterui_picker_item {
  struct waterui_picker_item *head;
  uintptr_t len;
} waterui_array_waterui_picker_item;

typedef struct waterui_fn_waterui_array_waterui_picker_item {
  void *data;
  void (*call)(const void*, struct waterui_array_waterui_picker_item);
  void (*drop)(void*);
} waterui_fn_waterui_array_waterui_picker_item;

typedef struct waterui_picker {
  struct waterui_computed_picker_items *items;
  struct waterui_binding_picker_item_id *selection;
} waterui_picker;

typedef struct waterui_progress {
  struct waterui_anyview *label;
  struct waterui_computed_double *value;
  enum waterui_style_progress style;
} waterui_progress;

typedef struct waterui_array_____waterui_anyview {
  struct waterui_anyview **head;
  uintptr_t len;
} waterui_array_____waterui_anyview;

typedef struct waterui_stack {
  struct waterui_array_____waterui_anyview contents;
  enum waterui_stack_mode mode;
} waterui_stack;

typedef struct waterui_stepper {
  const struct waterui_binding_int *value;
  struct waterui_computed_int *step;
} waterui_stepper;

typedef struct waterui_text_field {
  struct waterui_anyview *label;
  struct waterui_binding_str *value;
  struct waterui_text prompt;
} waterui_text_field;

typedef struct waterui_toggle {
  struct waterui_anyview *label;
  const struct waterui_binding_bool *toggle;
} waterui_toggle;

typedef struct waterui_with_env {
  struct waterui_anyview *view;
  struct waterui_env *env;
} waterui_with_env;

typedef struct waterui_str {
  uint8_t *head;
  uintptr_t len;
} waterui_str;

typedef struct waterui_fn_waterui_str {
  void *data;
  void (*call)(const void*, struct waterui_str);
  void (*drop)(void*);
} waterui_fn_waterui_str;

typedef struct waterui_fn_i32 {
  void *data;
  void (*call)(const void*, int32_t);
  void (*drop)(void*);
} waterui_fn_i32;

typedef struct waterui_fn_bool {
  void *data;
  void (*call)(const void*, bool);
  void (*drop)(void*);
} waterui_fn_bool;

typedef struct waterui_fn_f64 {
  void *data;
  void (*call)(const void*, double);
  void (*drop)(void*);
} waterui_fn_f64;

typedef struct waterui_fnonce {
  void *data;
  void (*call)(void*);
} waterui_fnonce;

void waterui_drop_watcher_guard(struct waterui_watcher_guard *value);

struct waterui_type_id waterui_view_id(const struct waterui_anyview *view);

struct waterui_type_id waterui_view_empty_id(void);

struct waterui_anyview *waterui_view_body(struct waterui_anyview *view, struct waterui_env *env);

struct waterui_button waterui_view_force_as_button(struct waterui_anyview *view);

struct waterui_type_id waterui_view_button_id(void);

void waterui_drop_computed_picker_items(struct waterui_computed_picker_items *value);

struct waterui_array_waterui_picker_item waterui_read_computed_picker_item(const struct waterui_computed_picker_items *computed);

struct waterui_watcher_guard *waterui_watch_computed_picker_item(const struct waterui_computed_picker_items *computed,
                                                                 struct waterui_fn_waterui_array_waterui_picker_item watcher);

void waterui_drop_binding_picker_item_id(struct waterui_binding_picker_item_id *value);

struct waterui_picker waterui_view_force_as_picker(struct waterui_anyview *view);

struct waterui_type_id waterui_view_picker_id(void);

struct waterui_progress waterui_view_force_as_progress(struct waterui_anyview *view);

struct waterui_type_id waterui_view_progress_id(void);

struct waterui_stack waterui_view_force_as_stack(struct waterui_anyview *view);

struct waterui_type_id waterui_view_stack_id(void);

struct waterui_stepper waterui_view_force_as_stepper(struct waterui_anyview *view);

struct waterui_type_id waterui_view_stepper_id(void);

struct waterui_text waterui_view_force_as_text(struct waterui_anyview *view);

struct waterui_type_id waterui_view_text_id(void);

struct waterui_text_field waterui_view_force_as_text_field(struct waterui_anyview *view);

struct waterui_type_id waterui_view_text_field_id(void);

struct waterui_toggle waterui_view_force_as_toggle(struct waterui_anyview *view);

struct waterui_type_id waterui_view_toggle_id(void);

struct waterui_with_env waterui_view_force_as_with_env(struct waterui_anyview *view);

struct waterui_type_id waterui_view_with_env_id(void);

void waterui_drop_binding_str(struct waterui_binding_str *value);

void waterui_drop_binding_int(struct waterui_binding_int *value);

void waterui_drop_binding_bool(struct waterui_binding_bool *value);

struct waterui_str waterui_read_binding_str(const struct waterui_binding_str *binding);

void waterui_set_binding_str(const struct waterui_binding_str *binding, struct waterui_str value);

struct waterui_watcher_guard *waterui_watch_binding_str(const struct waterui_binding_str *binding,
                                                        struct waterui_fn_waterui_str watcher);

int32_t waterui_read_binding_int(const struct waterui_binding_int *binding);

void waterui_set_binding_int(const struct waterui_binding_int *binding, int32_t value);

struct waterui_watcher_guard *waterui_watch_binding_int(const struct waterui_binding_int *binding,
                                                        struct waterui_fn_i32 watcher);

bool waterui_read_binding_bool(const struct waterui_binding_bool *binding);

void waterui_set_binding_bool(const struct waterui_binding_bool *binding, bool value);

struct waterui_watcher_guard *waterui_watch_binding_bool(const struct waterui_binding_bool *binding,
                                                         struct waterui_fn_bool watcher);

void waterui_drop_computed_str(struct waterui_computed_str *value);

void waterui_drop_computed_int(struct waterui_computed_int *value);

void waterui_drop_computed_double(struct waterui_computed_double *value);

void waterui_drop_computed_bool(struct waterui_computed_bool *value);

struct waterui_str waterui_read_computed_str(const struct waterui_computed_str *computed);

struct waterui_watcher_guard *waterui_watch_computed_str(const struct waterui_computed_str *computed,
                                                         struct waterui_fn_waterui_str watcher);

int32_t waterui_read_computed_int(const struct waterui_computed_int *computed);

struct waterui_watcher_guard *waterui_watch_computed_int(const struct waterui_computed_int *computed,
                                                         struct waterui_fn_i32 watcher);

bool waterui_read_computed_bool(const struct waterui_computed_bool *computed);

struct waterui_watcher_guard *waterui_watch_computed_bool(const struct waterui_computed_bool *computed,
                                                          struct waterui_fn_bool watcher);

double waterui_read_computed_double(const struct waterui_computed_double *computed);

struct waterui_watcher_guard *waterui_watch_computed_double(const struct waterui_computed_double *computed,
                                                            struct waterui_fn_f64 watcher);

void waterui_drop_env(struct waterui_env *value);

struct waterui_env *waterui_clone_env(const struct waterui_env *env);

void waterui_drop_action(struct waterui_action *value);

void waterui_call_action(const struct waterui_action *action);
