#include <stdio.h>
#include <stdbool.h>

typedef struct WaterUIBuf
{
    char *head;
    size_t len;
} WaterUIBuf;

typedef struct WaterUISize
{
    enum
    {
        DefaultSize,
        PxSize,
        PercentSize,
    } tag;
    union
    {
        size_t px;
        double percent;
    } value;
} WaterUISize;

typedef struct WaterUIEdge
{
    WaterUISize top;
    WaterUISize right;
    WaterUISize bottom;
    WaterUISize left;
} WaterUIEdge;

typedef struct WaterUIFrame
{

    WaterUISize width;
    WaterUISize min_width;
    WaterUISize max_width;

    WaterUISize height;
    WaterUISize min_height;
    WaterUISize max_height;

    WaterUIEdge margin;
} WaterUIFrame;

typedef struct WaterUIText
{
    WaterUIBuf buf;
} WaterUIText;

typedef struct WaterUIWidget WaterUIWidget;

typedef struct WaterUIWidgets
{
    WaterUIWidget *head;
    size_t len;
} WaterUIWidgets;

typedef struct WaterUIStack
{
    WaterUIWidgets contents;
} WaterUIStack;

typedef struct WaterUIWidget
{
    WaterUIFrame frame;
    enum
    {
        WaterUIEmptyTag,
        WaterUITextTag,
        WaterUIStackTag
    } tag;
    union
    {
        WaterUIText text;
        WaterUIStack stack;
    } value;
} WaterUIWidget;

size_t waterui_create_window(WaterUIWidget view);
void waterui_window_closeable(size_t id, bool is);
void waterui_close_window(size_t id);
WaterUIWidget waterui_main();