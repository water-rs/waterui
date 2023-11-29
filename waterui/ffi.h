#include <stdio.h>

typedef struct Buf
{
    unsigned short *head;
    size_t len;
} Buf;

typedef struct Text
{
    Buf buf;
} Text;

typedef struct Size
{
    enum
    {
        DefaultSize,
        PxSize,
        PercentSize,
        MaximumSize,
        MinimumSize
    } tag;
    union
    {
        size_t px;
        double percent;
        size_t maximum;
        size_t minimum;
    } value;
} Size;

typedef struct Edge
{
    Size top;
    Size right;
    Size bottom;
    Size left;
} Edge;

typedef struct Frame
{

    Size width;
    Size height;
    Edge margin;
} Frame;

typedef struct Node
{
    Frame frame;
    enum
    {
        TextNode
    } tag;
    union
    {
        Text text;
    } value;
} Node;

ssize_t size_to_px(Size size, size_t parent);

size_t __water_create_window(Node view);
void __water_init_app();