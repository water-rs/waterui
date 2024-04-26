#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

typedef enum ProgressStyle {
  ProgressStyle_Default,
  ProgressStyle_Circular,
  ProgressStyle_Linear,
} ProgressStyle;

typedef enum StackMode {
  StackMode_Auto,
  StackMode_Vertical,
  StackMode_Horizonal,
  StackMode_Layered,
} StackMode;

typedef enum ToggleStyle {
  ToggleStyle_Default,
  ToggleStyle_CheckBox,
  ToggleStyle_Switch,
} ToggleStyle;

typedef Computed<Cow<str>> waterui_computed_Cow_str;

typedef waterui_computed_Cow_str waterui_computed_str;

typedef waterui_computed_bool waterui_computed_bool;

typedef struct Text {
  waterui_computed_str *content;
  waterui_computed_bool *selection;
} Text;

typedef AnyView waterui_anyview;

typedef struct waterui_type_id {
  uint64_t inner[2];
} waterui_type_id;

typedef struct waterui_button {
  waterui_anyview *label;
} waterui_button;

typedef struct waterui_array_waterui_anyview {
  waterui_anyview *head;
  uintptr_t len;
} waterui_array_waterui_anyview;

typedef struct Stack {
  struct waterui_array_waterui_anyview views;
  enum StackMode mode;
} Stack;

typedef struct waterui_binding_Cow_str {
  uint8_t _priv[0];
} waterui_binding_Cow_str;

typedef struct waterui_binding_Cow_str waterui_binding_str;

typedef struct TextField {
  waterui_anyview *label;
  const waterui_binding_str *value;
  waterui_computed_str *prompt;
} TextField;

typedef struct waterui_binding_bool {
  uint8_t _priv[0];
} waterui_binding_bool;

typedef enum ToggleStyle waterui_style_toggle;

typedef struct waterui_toggle {
  waterui_anyview *label;
  const struct waterui_binding_bool *toggle;
  waterui_style_toggle style;
} waterui_toggle;

typedef Computed<Int> waterui_computed_Int;

typedef waterui_computed_Int waterui_computed_int;

typedef enum ProgressStyle waterui_style_progress;

typedef struct Progress {
  waterui_anyview *label;
  waterui_computed_int *progress;
  waterui_style_progress style;
} Progress;

typedef struct waterui_binding_Int {
  uint8_t _priv[0];
} waterui_binding_Int;

typedef struct waterui_binding_Int waterui_binding_int;

typedef struct Stepper {
  const waterui_binding_int *value;
  waterui_computed_int *step;
} Stepper;

typedef struct waterui_picker_item {
  waterui_anyview *label;
  uintptr_t value;
} waterui_picker_item;

typedef struct waterui_array_waterui_picker_item {
  struct waterui_picker_item *head;
  uintptr_t len;
} waterui_array_waterui_picker_item;

typedef struct waterui_picker {
  struct waterui_array_waterui_picker_item items;
  const waterui_binding_int *selection;
} waterui_picker;

typedef struct Edge {
  double top;
  double right;
  double bottom;
  double left;
} Edge;

typedef struct Padding {
  struct Edge _inner;
} Padding;

typedef struct WithValue_Padding {
  waterui_anyview *content;
  struct Padding value;
} WithValue_Padding;

typedef Environment waterui_env;

typedef struct waterui_str {
  uint8_t *head;
  uintptr_t len;
} waterui_str;

typedef struct waterui_binding_String {
  uint8_t _priv[0];
} waterui_binding_String;

typedef struct waterui_closure {
  void *data;
  void (*call)(const void*);
  void (*free)(void*);
} waterui_closure;

typedef struct waterui_array_u8 {
  uint8_t *head;
  uintptr_t len;
} waterui_array_u8;

typedef struct waterui_array_u8 waterui_data;

typedef Computed<Vec<uint8_t>> waterui_computed_Vec_u8;

typedef waterui_computed_Vec_u8 waterui_computed_data;

typedef Int waterui_int;

typedef struct App {
  waterui_anyview *content;
  waterui_env *env;
} App;

typedef struct AppClosure {
  void *data;
  void (*call)(const void*, struct App);
  void (*free)(void*);
} AppClosure;

struct Text waterui_view_force_as_text(waterui_anyview *view);

struct waterui_type_id waterui_view_text_id(void);

struct waterui_button waterui_view_force_as_button(waterui_anyview *view);

struct waterui_type_id waterui_view_button_id(void);

struct Stack waterui_view_force_as_stack(waterui_anyview *view);

struct waterui_type_id waterui_view_stack_id(void);

struct TextField waterui_view_force_as_field(waterui_anyview *view);

struct waterui_type_id waterui_view_field_id(void);

struct waterui_toggle waterui_view_force_as_toggle(waterui_anyview *view);

struct waterui_type_id waterui_view_toggle_id(void);

struct Progress waterui_view_force_as_progress(waterui_anyview *view);

struct waterui_type_id waterui_view_progress_id(void);

struct Stepper waterui_view_force_as_stepper(waterui_anyview *view);

struct waterui_type_id waterui_view_stepper_id(void);

struct waterui_picker waterui_view_force_as_picker(waterui_anyview *view);

struct waterui_type_id waterui_view_picker_id(void);

struct WithValue_Padding waterui_modifier_force_as_padding(waterui_anyview *view);

struct waterui_type_id waterui_modifier_padding_id(void);

struct waterui_type_id waterui_view_id(const waterui_anyview *view);

waterui_anyview *waterui_call_view(waterui_anyview *view, waterui_env *env);

struct waterui_type_id waterui_view_empty_id(void);

struct waterui_str waterui_read_binding_str(const struct waterui_binding_String *binding);

void waterui_write_binding_str(const struct waterui_binding_String *binding,
                               struct waterui_str value);

Int waterui_subscribe_binding_str(const struct waterui_binding_String *binding,
                                  struct waterui_closure subscriber);

void waterui_unsubscribe_binding_str(const struct waterui_binding_String *binding, uintptr_t id);

void waterui_drop_binding_str(struct waterui_binding_String *binding);

Int waterui_read_binding_int(const struct waterui_binding_Int *binding);

void waterui_write_binding_int(const struct waterui_binding_Int *binding, Int value);

Int waterui_subscribe_binding_int(const struct waterui_binding_Int *binding,
                                  struct waterui_closure subscriber);

void waterui_unsubscribe_binding_int(const struct waterui_binding_Int *binding, uintptr_t id);

void waterui_drop_binding_int(struct waterui_binding_Int *binding);

bool waterui_read_binding_bool(const struct waterui_binding_bool *binding);

void waterui_write_binding_bool(const struct waterui_binding_bool *binding, bool value);

Int waterui_subscribe_binding_bool(const struct waterui_binding_bool *binding,
                                   struct waterui_closure subscriber);

void waterui_unsubscribe_binding_bool(const struct waterui_binding_bool *binding, uintptr_t id);

void waterui_drop_binding_bool(struct waterui_binding_bool *binding);

waterui_data waterui_read_computed_data(const waterui_computed_data *computed);

Int waterui_subscribe_computed_data(const waterui_computed_data *computed,
                                    struct waterui_closure subscriber);

void waterui_unsubscribe_computed_data(const waterui_computed_data *computed, uintptr_t id);

void waterui_drop_computed_data(waterui_computed_data *computed);

struct waterui_str waterui_read_computed_str(const waterui_computed_str *computed);

Int waterui_subscribe_computed_str(const waterui_computed_str *computed,
                                   struct waterui_closure subscriber);

void waterui_unsubscribe_computed_str(const waterui_computed_str *computed, uintptr_t id);

void waterui_drop_computed_str(waterui_computed_str *computed);

waterui_int waterui_read_computed_int(const waterui_computed_int *computed);

Int waterui_subscribe_computed_int(const waterui_computed_int *computed,
                                   struct waterui_closure subscriber);

void waterui_unsubscribe_computed_int(const waterui_computed_int *computed, uintptr_t id);

void waterui_drop_computed_int(waterui_computed_int *computed);

bool waterui_read_computed_bool(const waterui_computed_bool *computed);

Int waterui_subscribe_computed_bool(const waterui_computed_bool *computed,
                                    struct waterui_closure subscriber);

void waterui_unsubscribe_computed_bool(const waterui_computed_bool *computed, uintptr_t id);

void waterui_drop_computed_bool(waterui_computed_bool *computed);
