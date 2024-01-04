#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

typedef struct WaterUISubscriber
{
  void *state;
  void (*subscriber)(void *);
} WaterUISubscriber;

typedef struct WaterUIViewObject
{
  uintptr_t inner[2];
} WaterUIViewObject;

typedef enum WaterUIStackMode
{
  WaterUIStackMode_Vertical,
  WaterUIStackMode_Horizonal,
} WaterUIStackMode;

typedef struct WaterUIEventObject
{
  uintptr_t inner[2];
} WaterUIEventObject;

typedef struct WaterUIBuf
{
  uint8_t *head;
  uintptr_t len;
  uintptr_t capacity;
} WaterUIBuf;

typedef struct WaterUIText
{
  const void *text;
  const void *selectable;
} WaterUIText;

typedef struct WaterUIButton
{
  struct WaterUIViewObject label;
  struct WaterUIEventObject action;
} WaterUIButton;

typedef struct WaterUIImage
{
  struct WaterUIBuf data;
} WaterUIImage;

typedef struct WaterUITextField
{
  const void *label;
  const void *value;
  const void *prompt;
} WaterUITextField;

typedef struct WaterUIViews
{
  struct WaterUIViewObject *head;
  uintptr_t len;
  uintptr_t capacity;
} WaterUIViews;

typedef struct WaterUIStack
{
  enum WaterUIStackMode mode;
  struct WaterUIViews contents;
} WaterUIStack;

typedef struct WaterUIToggle
{
  struct WaterUIViewObject label;
  const void *toggle;
} WaterUIToggle;

typedef struct WaterUIStepper
{
  struct WaterUIViewObject text;
  const void *value;
  uint64_t step;
} WaterUIStepper;

typedef struct WaterUIModifier
{
  const void *modifier;
  struct WaterUIViewObject view;
} WaterUIModifier;

typedef struct WaterUIApp
{
  struct WaterUIViewObject view;
  const void *env;
} WaterUIApp;

/**
 * # Safety
 * `EventObject` must be valid
 */
void waterui_call_event_object(struct WaterUIEventObject object);

/**
 * # Safety
 * Must be valid `Reactive<String>`.
 */
struct WaterUIBuf waterui_get_reactive_string(const void *reactive);

/**
 * # Safety
 * Must be valid `Reactive`
 */
void waterui_subscribe_reactive_string(const void *reactive, WaterUISubscriber subscriber);

/**
 * # Safety
 * Must be valid `Reactive`
 */
void waterui_subscribe_reactive_view(const void *reactive, WaterUISubscriber subscriber);

/**
 * # Safety
 * Must be valid `Reactive`
 */
void waterui_subscribe_reactive_bool(const void *reactive, WaterUISubscriber subscriber);

/**
 * # Safety
 * Must be valid `Binding`
 */
void waterui_subscribe_binding_string(const void *binding, WaterUISubscriber subscriber);

/**
 * # Safety
 * Must be valid `Binding`
 */
void waterui_subscribe_binding_bool(const void *binding, WaterUISubscriber subscriber);

/**
 * # Safety
 * Must be valid `Binding`
 */
void waterui_subscribe_binding_int(const void *binding, WaterUISubscriber subscriber);

/**
 * # Safety
 * Must be valid `Binding<String>`.
 */
struct WaterUIBuf waterui_get_binding_string(const void *binding);

/**
 * # Safety
 * Must be valid `Binding<i64>`.
 */
int64_t waterui_get_binding_int(const void *binding);

/**
 * # Safety
 * Must be valid `Binding<u64>`.
 */
void waterui_increment_binding_int(const void *binding, int64_t num);

/**
 * # Safety
 * `Binding<String>` must be valid, and `Buf` must be valid UTF-8 string.
 */
void waterui_set_binding_string(const void *binding, struct WaterUIBuf string);

/**
 * # Safety
 * Must be valid `Reactive<BoxView>`.
 */
struct WaterUIViewObject waterui_get_reactive_view(const void *binding);

/**
 * # Safety
 * Must be valid `Binding<bool>`
 */
bool waterui_get_binding_bool(const void *binding);

/**
 * # Safety
 * Must be valid `Reactive<bool>`
 */
bool waterui_get_reactive_bool(const void *reactive);

/**
 * # Safety
 * Must be valid `Binding<bool>`
 */
void waterui_set_binding_bool(const void *binding, bool bool_);

/**
 * # Safety
 * Must be valid `Reactive<BoxView>`.
 */
const void *waterui_view_to_reactive_view(struct WaterUIViewObject view);

struct WaterUIViewObject waterui_unwrap_anyview(struct WaterUIViewObject view);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_empty(struct WaterUIViewObject view);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_text(struct WaterUIViewObject view, struct WaterUIText *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_button(struct WaterUIViewObject view, struct WaterUIButton *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_image(struct WaterUIViewObject view, struct WaterUIImage *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_text_field(struct WaterUIViewObject view, struct WaterUITextField *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_stack(struct WaterUIViewObject view, struct WaterUIStack *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_toggle(struct WaterUIViewObject view, struct WaterUIToggle *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_stepper(struct WaterUIViewObject view, struct WaterUIStepper *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_frame_modifier(struct WaterUIViewObject view, struct WaterUIModifier *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_display_modifier(struct WaterUIViewObject view,
                                        struct WaterUIModifier *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
struct WaterUIViewObject waterui_call_view(struct WaterUIViewObject view, const void *env);

void waterui_env_increment_count(const void *env);

void waterui_env_decrement_count(const void *env);

extern struct WaterUIApp waterui_main(void);
