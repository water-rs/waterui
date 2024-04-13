#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

typedef struct WaterUIString WaterUIString;

typedef struct WaterUIData {
  uint8_t *head;
  uintptr_t len;
} WaterUIData;

typedef struct WaterUIComputedData {
  uintptr_t inner[2];
} WaterUIComputedData;

typedef struct WaterUISubscriber {
  void *data;
  void (*call)(const void*);
  void (*free)(void*);
} WaterUISubscriber;

typedef struct WaterUIUtf8Data {
  struct WaterUIData inner;
} WaterUIUtf8Data;

typedef struct WaterUIComputedUtf8Data {
  uintptr_t inner[2];
} WaterUIComputedUtf8Data;

typedef struct WaterUIBindingUtf8Data {
  const struct WaterUIString *pointer;
} WaterUIBindingUtf8Data;

typedef intptr_t WaterUIInt;

typedef struct WaterUIBindingInt {
  const WaterUIInt *pointer;
} WaterUIBindingInt;

typedef struct WaterUIComputedInt {
  uintptr_t inner[2];
} WaterUIComputedInt;

typedef struct WaterUITypeId {
  uint64_t inner[2];
} WaterUITypeId;

typedef struct WaterUIAnyView {
  uintptr_t inner[2];
} WaterUIAnyView;

typedef struct WaterUIText {
  struct WaterUIComputedUtf8Data content;
} WaterUIText;

struct WaterUIData waterui_read_computed_data(struct WaterUIComputedData computed);

uintptr_t waterui_subscribe_computed_data(struct WaterUIComputedData computed,
                                          struct WaterUISubscriber subscriber);

void waterui_unsubscribe_computed_data(struct WaterUIComputedData computed, uintptr_t id);

struct WaterUIUtf8Data waterui_read_computed_str(struct WaterUIComputedUtf8Data computed);

uintptr_t waterui_subscribe_computed_str(struct WaterUIComputedUtf8Data computed,
                                         struct WaterUISubscriber subscriber);

void waterui_unsubscribe_computed_str(struct WaterUIComputedUtf8Data computed, uintptr_t id);

struct WaterUIUtf8Data waterui_read_binding_str(struct WaterUIBindingUtf8Data binding);

void waterui_write_binding_str(struct WaterUIBindingUtf8Data binding, struct WaterUIUtf8Data value);

uintptr_t waterui_subscribe_binding_str(struct WaterUIBindingUtf8Data binding,
                                        struct WaterUISubscriber subscriber);

void waterui_unsubscribe_binding_str(struct WaterUIBindingUtf8Data binding, uintptr_t id);

WaterUIInt waterui_read_binding_int(struct WaterUIBindingInt binding);

void waterui_write_binding_int(struct WaterUIBindingInt binding, WaterUIInt value);

uintptr_t waterui_subscribe_binding_int(struct WaterUIBindingInt binding,
                                        struct WaterUISubscriber subscriber);

void waterui_unsubscribe_binding_int(struct WaterUIBindingInt binding, uintptr_t id);

WaterUIInt waterui_read_computed_int(struct WaterUIComputedInt computed);

uintptr_t waterui_subscribe_computed_int(struct WaterUIComputedInt computed,
                                         struct WaterUISubscriber subscriber);

void waterui_unsubscribe_computed_int(struct WaterUIComputedInt computed, uintptr_t id);

struct WaterUITypeId waterui_view_id(struct WaterUIAnyView view);

struct WaterUIAnyView waterui_call_view(struct WaterUIAnyView view);

struct WaterUIText waterui_view_force_as_text(struct WaterUIAnyView view);

void waterui_view_free_text(struct WaterUIText text);

struct WaterUITypeId waterui_view_text_id(void);

struct WaterUIAnyView waterui_view_force_as_anyview(struct WaterUIAnyView view);

void waterui_view_free_anyview(struct WaterUIAnyView text);

struct WaterUITypeId waterui_view_anyview_id(void);
