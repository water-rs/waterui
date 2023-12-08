#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

enum WaterUIAlignment {
  WaterUIAlignment_Default,
  WaterUIAlignment_Leading,
  WaterUIAlignment_Center,
  WaterUIAlignment_Trailing,
};
typedef uint8_t WaterUIAlignment;

enum WaterUIStackMode {
  WaterUIStackMode_Vertical,
  WaterUIStackMode_Horizonal,
};
typedef uint8_t WaterUIStackMode;

typedef struct WaterUIEventObject {
  uintptr_t inner[2];
} WaterUIEventObject;

typedef struct WaterUIViewObject {
  uintptr_t inner[2];
} WaterUIViewObject;

typedef struct WaterUIBuf {
  const uint8_t *head;
  uintptr_t len;
} WaterUIBuf;

typedef struct WaterUIText {
  struct WaterUIBuf buf;
  bool selectable;
} WaterUIText;

typedef struct WaterUIButton {
  struct WaterUIBuf label;
  struct WaterUIEventObject action;
} WaterUIButton;

typedef struct WaterUITapGesture {
  struct WaterUIViewObject view;
  struct WaterUIEventObject event;
} WaterUITapGesture;

typedef enum WaterUISize_Tag {
  WaterUISize_Default,
  WaterUISize_Px,
  WaterUISize_Percent,
} WaterUISize_Tag;

typedef struct WaterUISize {
  WaterUISize_Tag tag;
  union {
    struct {
      uintptr_t px;
    };
    struct {
      double percent;
    };
  };
} WaterUISize;

typedef struct WaterUIEdge {
  struct WaterUISize top;
  struct WaterUISize right;
  struct WaterUISize bottom;
  struct WaterUISize left;
} WaterUIEdge;

typedef struct WaterUIFrame {
  struct WaterUISize width;
  struct WaterUISize min_width;
  struct WaterUISize max_width;
  struct WaterUISize height;
  struct WaterUISize min_height;
  struct WaterUISize max_height;
  struct WaterUIEdge margin;
  WaterUIAlignment alignment;
} WaterUIFrame;

typedef struct WaterUIFrameModifier {
  struct WaterUIFrame frame;
  struct WaterUIViewObject view;
} WaterUIFrameModifier;

typedef struct WaterUIViews {
  const struct WaterUIViewObject *head;
  uintptr_t len;
} WaterUIViews;

typedef struct WaterUIStack {
  WaterUIStackMode mode;
  struct WaterUIViews contents;
} WaterUIStack;

typedef struct WaterUISubscriberObject {
  const void *state;
  void (*subscriber)(const void*);
} WaterUISubscriberObject;

typedef struct WaterUISubscriberBuilderObject {
  const void *state;
  struct WaterUISubscriberObject (*subscriber)(const void*);
} WaterUISubscriberBuilderObject;

/**
 * # Safety
 * `EventObject` must be valid
 */
void waterui_call_event_object(struct WaterUIEventObject object);

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
int8_t waterui_view_to_tap_gesture(struct WaterUIViewObject view, struct WaterUITapGesture *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_frame_modifier(struct WaterUIViewObject view,
                                      struct WaterUIFrameModifier *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_stack(struct WaterUIViewObject view, struct WaterUIStack *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
struct WaterUIViewObject waterui_call_view(struct WaterUIViewObject view);

/**
 * # Safety
 * `EventObject` must be valid
 */
void waterui_add_subscriber(struct WaterUIViewObject view,
                            struct WaterUISubscriberBuilderObject subscriber);

extern uintptr_t waterui_create_window(struct WaterUIBuf title, struct WaterUIViewObject content);

extern void waterui_window_closeable(uintptr_t id, bool is);

extern void waterui_close_window(uintptr_t id);

extern struct WaterUIViewObject waterui_main(void);
