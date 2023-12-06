#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

enum WaterUIAlignment
{
  WaterUIAlignment_Default,
  WaterUIAlignment_Leading,
  WaterUIAlignment_Center,
  WaterUIAlignment_Trailing,
};
typedef uint8_t WaterUIAlignment;

enum WaterUIStackMode
{
  WaterUIStackMode_Vertical,
  WaterUIStackMode_Horizonal,
};
typedef uint8_t WaterUIStackMode;

typedef struct WaterUIBuf
{
  const char *head;
  uintptr_t len;
} WaterUIBuf;

typedef enum WaterUISize_Tag
{
  WaterUISize_Default,
  WaterUISize_Px,
  WaterUISize_Percent,
} WaterUISize_Tag;

typedef struct WaterUISize
{
  WaterUISize_Tag tag;
  union
  {
    struct
    {
      uintptr_t px;
    };
    struct
    {
      double percent;
    };
  };
} WaterUISize;

typedef struct WaterUIEdge
{
  struct WaterUISize top;
  struct WaterUISize right;
  struct WaterUISize bottom;
  struct WaterUISize left;
} WaterUIEdge;

typedef struct WaterUIFrame
{
  struct WaterUISize width;
  struct WaterUISize min_width;
  struct WaterUISize max_width;
  struct WaterUISize height;
  struct WaterUISize min_height;
  struct WaterUISize max_height;
  struct WaterUIEdge margin;
  WaterUIAlignment alignment;
} WaterUIFrame;

typedef struct WaterUIColor
{
  uint8_t red;
  uint8_t green;
  uint8_t blue;
  double opacity;
} WaterUIColor;

typedef enum WaterUIBackground_Tag
{
  WaterUIBackground_Default,
  WaterUIBackground_Color,
} WaterUIBackground_Tag;

typedef struct WaterUIBackground
{
  WaterUIBackground_Tag tag;
  union
  {
    struct
    {
      struct WaterUIColor color;
    };
  };
} WaterUIBackground;

typedef struct WaterUIText
{
  struct WaterUIBuf buf;
} WaterUIText;

typedef struct WaterUIButton
{
  struct WaterUIBuf label;
} WaterUIButton;

typedef struct WaterUIImage
{
  struct WaterUIBuf data;
} WaterUIImage;

typedef struct WaterUIWidgets
{
  const struct WaterUIWidget *head;
  uintptr_t len;
} WaterUIWidgets;

typedef struct WaterUIStack
{
  WaterUIStackMode mode;
  struct WaterUIWidgets contents;
} WaterUIStack;

typedef enum WaterUIWidgetInner_Tag
{
  WaterUIWidgetInner_Empty,
  WaterUIWidgetInner_Text,
  WaterUIWidgetInner_Button,
  WaterUIWidgetInner_Image,
  WaterUIWidgetInner_Stack,
} WaterUIWidgetInner_Tag;

typedef struct WaterUIWidgetInner
{
  WaterUIWidgetInner_Tag tag;
  union
  {
    struct
    {
      struct WaterUIText text;
    };
    struct
    {
      struct WaterUIButton button;
    };
    struct
    {
      struct WaterUIImage image;
    };
    struct
    {
      struct WaterUIStack stack;
    };
  };
} WaterUIWidgetInner;

typedef struct WaterUIWidget
{
  struct WaterUIFrame frame;
  struct WaterUIBackground background;
  struct WaterUIWidgetInner inner;
} WaterUIWidget;

extern uintptr_t waterui_create_window(struct WaterUIBuf title, struct WaterUIWidget widget);

extern void waterui_window_closeable(uintptr_t id, bool is);

extern void waterui_close_window(uintptr_t id);

extern struct WaterUIWidget waterui_main(void);
