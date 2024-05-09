#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

typedef enum waterui_stack_mode {
  waterui_stack_mode_AUTO,
  waterui_stack_mode_VERTICAL,
  waterui_stack_mode_HORIZONTAL,
  waterui_stack_mode_LAYERED,
} waterui_stack_mode;

typedef enum waterui_style_progress {
  waterui_style_progress_DEFAULT,
  waterui_style_progress_CIRCULAR,
  waterui_style_progress_LINEAR,
} waterui_style_progress;

typedef enum waterui_style_toggle {
  waterui_style_toggle_Default,
  waterui_style_toggle_CheckBox,
  waterui_style_toggle_Switch,
} waterui_style_toggle;

typedef struct waterui_anyview waterui_anyview;

typedef struct waterui_binding_bool waterui_binding_bool;

typedef struct waterui_binding_int waterui_binding_int;

typedef struct waterui_binding_str waterui_binding_str;

typedef struct waterui_computed_bool waterui_computed_bool;

typedef struct waterui_computed_int waterui_computed_int;

typedef struct waterui_computed_str waterui_computed_str;

typedef struct waterui_each waterui_each;

typedef struct waterui_env waterui_env;

typedef struct waterui_type_id {
  uint64_t inner[2];
} waterui_type_id;

typedef struct waterui_button {
  struct waterui_anyview *label;
} waterui_button;

typedef struct waterui_progress {
  struct waterui_anyview *label;
  struct waterui_computed_int *progress;
  enum waterui_style_progress style;
} waterui_progress;

typedef struct waterui_array_waterui_anyview {
  struct waterui_anyview *head;
  uintptr_t len;
} waterui_array_waterui_anyview;

typedef struct waterui_stack {
  struct waterui_array_waterui_anyview contents;
  enum waterui_stack_mode mode;
} waterui_stack;

typedef struct waterui_stepper {
  const struct waterui_binding_int *value;
  struct waterui_computed_int *step;
} waterui_stepper;

typedef struct waterui_text {
  struct waterui_computed_str *content;
  struct waterui_computed_bool *selection;
} waterui_text;

typedef struct waterui_toggle {
  struct waterui_anyview *label;
  const struct waterui_binding_bool *toggle;
  enum waterui_style_toggle style;
} waterui_toggle;

typedef struct waterui_padding {
  double top;
  double right;
  double bottom;
  double left;
} waterui_padding;

typedef struct waterui_metadata_waterui_padding {
  struct waterui_anyview *content;
  struct waterui_padding value;
} waterui_metadata_waterui_padding;

typedef struct waterui_str {
  uint8_t *head;
  uintptr_t len;
} waterui_str;

typedef struct waterui_closure {
  void *data;
  void (*call)(const void*);
  void (*free)(void*);
} waterui_closure;

typedef struct waterui_app {
  struct waterui_anyview *content;
  struct waterui_env *env;
} waterui_app;

typedef struct waterui_app_closure {
  void *data;
  void (*call)(const void*, struct waterui_app);
  void (*free)(void*);
} waterui_app_closure;

struct waterui_type_id waterui_view_id(const struct waterui_anyview *view);

struct waterui_anyview *waterui_call_view(struct waterui_anyview *view,
                                          const struct waterui_env *env);

struct waterui_type_id waterui_view_empty_id(void);

struct waterui_button waterui_view_force_as_button(struct waterui_anyview *view);

struct waterui_type_id waterui_view_button_id(void);

struct waterui_each *waterui_view_force_as_each(struct waterui_anyview *view);

struct waterui_type_id waterui_view_each_id(void);

uintptr_t waterui_each_id(struct waterui_each *each, uintptr_t index);

struct waterui_anyview *waterui_each_pull(struct waterui_each *each, uintptr_t index);

uintptr_t waterui_each_len(const struct waterui_each *each);

struct waterui_progress waterui_view_force_as_progress(struct waterui_anyview *view);

struct waterui_type_id waterui_view_progress_id(void);

struct waterui_stack waterui_view_force_as_stack(struct waterui_anyview *view);

struct waterui_type_id waterui_view_stack_id(void);

struct waterui_stepper waterui_view_force_as_stepper(struct waterui_anyview *view);

struct waterui_type_id waterui_view_stepper_id(void);

struct waterui_text waterui_view_force_as_text(struct waterui_anyview *view);

struct waterui_type_id waterui_view_text_id(void);

struct waterui_toggle waterui_view_force_as_toggle(struct waterui_anyview *view);

struct waterui_type_id waterui_view_toggle_id(void);

struct waterui_metadata_waterui_padding waterui_metadata_force_as_padding(struct waterui_anyview *view);

struct waterui_type_id waterui_metadata_padding_id(void);

struct waterui_str waterui_read_binding_str(const struct waterui_binding_str *binding);

void waterui_write_binding_str(const struct waterui_binding_str *binding, struct waterui_str value);

intptr_t waterui_subscribe_binding_str(const struct waterui_binding_str *binding,
                                       struct waterui_closure subscriber);

void waterui_unsubscribe_binding_str(const struct waterui_binding_str *binding, uintptr_t id);

void waterui_drop_binding_str(struct waterui_binding_str *binding);

int32_t waterui_read_binding_int(const struct waterui_binding_int *binding);

void waterui_write_binding_int(const struct waterui_binding_int *binding, int32_t value);

intptr_t waterui_subscribe_binding_int(const struct waterui_binding_int *binding,
                                       struct waterui_closure subscriber);

void waterui_unsubscribe_binding_int(const struct waterui_binding_int *binding, uintptr_t id);

void waterui_drop_binding_int(struct waterui_binding_int *binding);

bool waterui_read_binding_bool(const struct waterui_binding_bool *binding);

void waterui_write_binding_bool(const struct waterui_binding_bool *binding, bool value);

intptr_t waterui_subscribe_binding_bool(const struct waterui_binding_bool *binding,
                                        struct waterui_closure subscriber);

void waterui_unsubscribe_binding_bool(const struct waterui_binding_bool *binding, uintptr_t id);

void waterui_drop_binding_bool(struct waterui_binding_bool *binding);

struct waterui_str waterui_read_computed_str(const struct waterui_computed_str *computed);

intptr_t waterui_subscribe_computed_str(const struct waterui_computed_str *computed,
                                        struct waterui_closure subscriber);

void waterui_unsubscribe_computed_str(const struct waterui_computed_str *computed, uintptr_t id);

void waterui_drop_computed_str(struct waterui_computed_str *computed);

int32_t waterui_read_computed_int(const struct waterui_computed_int *computed);

intptr_t waterui_subscribe_computed_int(const struct waterui_computed_int *computed,
                                        struct waterui_closure subscriber);

void waterui_unsubscribe_computed_int(const struct waterui_computed_int *computed, uintptr_t id);

void waterui_drop_computed_int(struct waterui_computed_int *computed);

bool waterui_read_computed_bool(const struct waterui_computed_bool *computed);

intptr_t waterui_subscribe_computed_bool(const struct waterui_computed_bool *computed,
                                         struct waterui_closure subscriber);

void waterui_unsubscribe_computed_bool(const struct waterui_computed_bool *computed, uintptr_t id);

void waterui_drop_computed_bool(struct waterui_computed_bool *computed);
