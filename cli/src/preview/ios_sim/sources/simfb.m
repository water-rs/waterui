#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>
#import <IOSurface/IOSurfaceRef.h>
#include <mach/mach_time.h>
#include <os/lock.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define EXPORT __attribute__((visibility("default")))

#pragma pack(push, 1)
typedef struct {
    uint32_t w;
    uint32_t h;
} SimSize;

typedef struct {
    uint32_t mode_id;
    SimSize size;
    uint32_t scale;
    uint32_t unk16;
    uint32_t pixel_format;
    uint32_t colorspace;
    uint32_t refresh_centihz;
    uint8_t preferred;
    uint8_t unk33;
    uint8_t current;
} SimDisplayMode;
#pragma pack(pop)

_Static_assert(sizeof(SimDisplayMode) == 35, "SimDisplayMode must match CoreSimulator layout");

typedef struct {
    double w;
    double h;
} DSize;

@interface WaterUISFBConn : NSObject {
@public
    CFStringRef label;
    NSArray *displays;
    dispatch_queue_t updQ;
    id updBlock;
}
@end
@implementation WaterUISFBConn
@end

@interface WaterUISFBDisp : NSObject {
@public
    CFStringRef name;
    CFStringRef uid;
    CFStringRef device_name;
    uint32_t display_id;
    uint32_t type;
    uint32_t dot_pitch;
    SimSize device;
    SimSize canvas;
    uint64_t flags;
    SimDisplayMode mode;
}
@end
@implementation WaterUISFBDisp
@end

typedef void (^SFBSwapCallback)(
    CFErrorRef err,
    id swapchain,
    uint64_t present_time,
    uint64_t end_time,
    void *ctx
);

@interface WaterUISFBSwap : NSObject {
@public
    WaterUISFBDisp *display;
    uint32_t swap_id;
    uint32_t presentation_mode;
    SimSize size;
    BOOL pending;
    uint64_t seq;
    uint64_t begin_time;
    NSMutableArray *surfaces;
    NSMutableArray<NSValue *> *rects;
    void *cb_ctx;
    dispatch_queue_t cb_q;
    SFBSwapCallback cb;
}
@end
@implementation WaterUISFBSwap
@end

static atomic_uint_fast64_t gSeq;
static WaterUISFBDisp *gMainDisplay;

// Persistent offscreen backing store capturing full and partial presents without tearing.
static os_unfair_lock gFrameLock = OS_UNFAIR_LOCK_INIT;
static uint8_t *gFrameBytes;
static size_t gFrameCapacity;
static uint32_t gFrameWidth;
static uint32_t gFrameHeight;
static uint32_t gFramePixelFormat;
static uint32_t gFrameBytesPerElement;
static uint64_t gFrameSeq;
static char gColorSpaceName[128];

static uint32_t env_u32(const char *key, uint32_t fallback) {
    const char *val = getenv(key);
    if (!val || !val[0]) {
        return fallback;
    }
    double parsed = atof(val);
    return parsed > 0.0 ? (uint32_t)(parsed + 0.5) : fallback;
}

static WaterUISFBDisp *main_display(void) {
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        WaterUISFBDisp *d = [WaterUISFBDisp new];
        d->name = CFSTR("LCD");
        d->uid = CFSTR("PurpleMain");
        d->device_name = CFSTR("LCD");
        d->display_id = 1;
        d->type = 0;
        d->flags = 1;
        d->dot_pitch = env_u32("SIMULATOR_MAINSCREEN_PITCH", 326);
        uint32_t w = env_u32("SIMULATOR_MAINSCREEN_WIDTH", 780);
        uint32_t h = env_u32("SIMULATOR_MAINSCREEN_HEIGHT", 960);
        uint32_t scale = env_u32("SIMULATOR_MAINSCREEN_SCALE", 2);
        uint32_t refresh = env_u32("WATERUI_SIMFB_REFRESH_CENTIHZ", 6000);
        d->device = (SimSize){ w, h };
        d->canvas = (SimSize){ w, h };
        d->mode = (SimDisplayMode){
            .mode_id = 0,
            .size = { w, h },
            .scale = scale,
            .unk16 = 0,
            .pixel_format = 'RGhA',
            .colorspace = 1,
            .refresh_centihz = refresh,
            .preferred = 1,
            .unk33 = 0,
            .current = 1,
        };
        gMainDisplay = d;
    });
    return gMainDisplay;
}

static void record_surface_frame(IOSurfaceRef surf, CGRect dirty, uint64_t seq) {
    IOSurfaceLock(surf, kIOSurfaceLockReadOnly, NULL);
    size_t w = IOSurfaceGetWidth(surf);
    size_t h = IOSurfaceGetHeight(surf);
    size_t bpr = IOSurfaceGetBytesPerRow(surf);
    size_t bpe = IOSurfaceGetBytesPerElement(surf);
    uint32_t fmt = IOSurfaceGetPixelFormat(surf);
    const uint8_t *src = (const uint8_t *)IOSurfaceGetBaseAddress(surf);

    if (src && w > 0 && h > 0 && bpe > 0) {
        size_t dst_bpr = w * bpe;
        size_t needed = dst_bpr * h;

        char cs_name[128] = "";
        CFTypeRef cs = IOSurfaceCopyValue(surf, CFSTR("IOSurfaceColorSpace"));
        if (cs) {
            if (CFGetTypeID(cs) == CFStringGetTypeID()) {
                CFStringGetCString((CFStringRef)cs, cs_name, sizeof(cs_name), kCFStringEncodingUTF8);
            }
            CFRelease(cs);
        }

        os_unfair_lock_lock(&gFrameLock);
        bool full = (gFrameWidth != w) || (gFrameHeight != h) || (gFrameBytesPerElement != bpe) ||
            (gFramePixelFormat != fmt) || (dirty.size.width <= 0.0) || (dirty.size.height <= 0.0) ||
            (dirty.origin.x <= 0.0 && dirty.origin.y <= 0.0 && dirty.size.width >= (double)w &&
             dirty.size.height >= (double)h);

        if (needed > gFrameCapacity) {
            uint8_t *grown = (uint8_t *)realloc(gFrameBytes, needed);
            if (grown) {
                gFrameBytes = grown;
                gFrameCapacity = needed;
            }
        }
        if (gFrameBytes && needed <= gFrameCapacity) {
            if (full) {
                for (size_t y = 0; y < h; y++) {
                    memcpy(gFrameBytes + y * dst_bpr, src + y * bpr, dst_bpr);
                }
            } else {
                size_t x0 = dirty.origin.x > 0.0 ? (size_t)dirty.origin.x : 0;
                size_t y0 = dirty.origin.y > 0.0 ? (size_t)dirty.origin.y : 0;
                size_t x1 = (size_t)CGRectGetMaxX(dirty);
                size_t y1 = (size_t)CGRectGetMaxY(dirty);
                if (x1 > w) {
                    x1 = w;
                }
                if (y1 > h) {
                    y1 = h;
                }
                if (x1 > x0 && y1 > y0) {
                    size_t span = (x1 - x0) * bpe;
                    for (size_t y = y0; y < y1; y++) {
                        memcpy(
                            gFrameBytes + y * dst_bpr + x0 * bpe,
                            src + y * bpr + x0 * bpe,
                            span
                        );
                    }
                }
            }
            gFrameWidth = (uint32_t)w;
            gFrameHeight = (uint32_t)h;
            gFramePixelFormat = fmt;
            gFrameBytesPerElement = (uint32_t)bpe;
            gFrameSeq = seq;
            if (cs_name[0] != '\0') {
                strlcpy(gColorSpaceName, cs_name, sizeof(gColorSpaceName));
            }
        }
        os_unfair_lock_unlock(&gFrameLock);
    }

    IOSurfaceUnlock(surf, kIOSurfaceLockReadOnly, NULL);
}

// ---- WaterUI in-process frame readback exports ----

EXPORT uint64_t waterui_simfb_frame_count(void) {
    os_unfair_lock_lock(&gFrameLock);
    uint64_t seq = gFrameSeq;
    os_unfair_lock_unlock(&gFrameLock);
    return seq;
}

EXPORT bool waterui_simfb_copy_latest(
    uint8_t *out_buf,
    size_t out_capacity,
    uint32_t *out_width,
    uint32_t *out_height,
    uint32_t *out_pixel_format,
    uint32_t *out_bytes_per_element,
    uint64_t *out_seq
) {
    os_unfair_lock_lock(&gFrameLock);
    if (gFrameSeq == 0 || !gFrameBytes || gFrameWidth == 0 || gFrameHeight == 0) {
        os_unfair_lock_unlock(&gFrameLock);
        return false;
    }
    size_t required = (size_t)gFrameWidth * (size_t)gFrameHeight * (size_t)gFrameBytesPerElement;
    if (out_width) {
        *out_width = gFrameWidth;
    }
    if (out_height) {
        *out_height = gFrameHeight;
    }
    if (out_pixel_format) {
        *out_pixel_format = gFramePixelFormat;
    }
    if (out_bytes_per_element) {
        *out_bytes_per_element = gFrameBytesPerElement;
    }
    if (out_seq) {
        *out_seq = gFrameSeq;
    }
    bool ok = true;
    if (out_buf) {
        if (out_capacity >= required) {
            memcpy(out_buf, gFrameBytes, required);
        } else {
            ok = false;
        }
    }
    os_unfair_lock_unlock(&gFrameLock);
    return ok;
}

EXPORT size_t waterui_simfb_copy_colorspace_name(char *out_buf, size_t out_capacity) {
    if (!out_buf || out_capacity == 0) {
        return 0;
    }
    os_unfair_lock_lock(&gFrameLock);
    size_t len = strlcpy(out_buf, gColorSpaceName, out_capacity);
    os_unfair_lock_unlock(&gFrameLock);
    return len;
}

// ---- SFBConnection (all 58 SimFramebufferClient symbols) ----

EXPORT const char *simFramebufferGetStructName(uint32_t t) {
    (void)t;
    return "SimStruct";
}
EXPORT CFTypeID SFBConnectionGetTypeID(void) { return 0; }
EXPORT CFTypeRef SFBConnectionCreate(CFAllocatorRef a, CFStringRef label) {
    (void)a;
    WaterUISFBConn *c = [WaterUISFBConn new];
    c->label = label ? CFRetain(label) : NULL;
    main_display();
    return CFBridgingRetain(c);
}
EXPORT bool SFBConnectionConnect(CFTypeRef c, CFErrorRef *err) {
    (void)err;
    ((__bridge WaterUISFBConn *)c)->displays = @[ main_display() ];
    return true;
}
EXPORT uint64_t SFBConnectionGetID(CFTypeRef c) {
    (void)c;
    return 1;
}
static void set_handler(CFTypeRef c, dispatch_queue_t q, id blk, int which) {
    if (which == 1) {
        WaterUISFBConn *cc = (__bridge WaterUISFBConn *)c;
        cc->updQ = q;
        cc->updBlock = [blk copy];
    }
}
EXPORT void SFBConnectionSetDisplayConnectedHandler(CFTypeRef c, dispatch_queue_t q, id blk) {
    set_handler(c, q, blk, 0);
}
EXPORT void SFBConnectionSetDisplayUpdatedHandler(CFTypeRef c, dispatch_queue_t q, id blk) {
    set_handler(c, q, blk, 1);
}
EXPORT void SFBConnectionSetDisplayDisconnectedHandler(CFTypeRef c, dispatch_queue_t q, id blk) {
    set_handler(c, q, blk, 2);
}
EXPORT CFArrayRef SFBConnectionCopyDisplays(CFTypeRef c, CFErrorRef *err) {
    (void)c;
    (void)err;
    return CFBridgingRetain(@[ main_display() ]);
}
EXPORT CFTypeRef SFBConnectionCopyDisplay(CFTypeRef c, uint32_t display_id) {
    (void)c;
    return display_id == main_display()->display_id ? CFBridgingRetain(main_display()) : NULL;
}
EXPORT CFTypeRef SFBConnectionCopyDisplayByUID(CFTypeRef c, CFStringRef uid) {
    (void)c;
    return (uid && CFEqual(uid, main_display()->uid)) ? CFBridgingRetain(main_display()) : NULL;
}
EXPORT CFTypeRef SFBConnectionCreateDisplay(CFTypeRef c, ...) {
    (void)c;
    return NULL;
}
EXPORT void SFBConnectionUpdateDisplay(CFTypeRef c, ...) { (void)c; }
EXPORT void SFBConnectionRemoveDisplay(CFTypeRef c, ...) { (void)c; }

// ---- SFBDisplay ----

#define DISP ((__bridge WaterUISFBDisp *)d)
EXPORT CFTypeID SFBDisplayGetTypeID(void) { return 0; }
EXPORT CFStringRef SFBDisplayGetName(CFTypeRef d) { return DISP->name; }
EXPORT CFStringRef SFBDisplayGetDisplayUID(CFTypeRef d) { return DISP->uid; }
EXPORT CFStringRef SFBDisplayGetDeviceName(CFTypeRef d) { return DISP->device_name; }
EXPORT uint64_t SFBDisplayGetConnectionID(CFTypeRef d) {
    (void)d;
    return 1;
}
EXPORT uint32_t SFBDisplayGetPowerState(CFTypeRef d) { return (uint32_t)(DISP->flags & 1u); }
EXPORT uint64_t SFBDisplayGetModeCount(CFTypeRef d) {
    (void)d;
    return 1;
}
EXPORT bool SFBDisplayGetMode(CFTypeRef d, int64_t i, SimDisplayMode *out) {
    if (i != 0) {
        return false;
    }
    *out = DISP->mode;
    return true;
}
EXPORT void SFBDisplayGetPreferredMode(CFTypeRef d, SimDisplayMode *out) { *out = DISP->mode; }
EXPORT void SFBDisplayGetCurrentMode(CFTypeRef d, SimDisplayMode *out) { *out = DISP->mode; }
EXPORT bool SFBDisplaySetCurrentMode(CFTypeRef d, const SimDisplayMode *m, CFErrorRef *err) {
    (void)d;
    (void)m;
    (void)err;
    return true;
}
EXPORT bool SFBDisplayGetExtendedProperties(CFTypeRef d, void *out) {
    (void)d;
    (void)out;
    return false;
}
EXPORT CFArrayRef SFBDisplayCopyExtendedPropertyProtocols(CFTypeRef d) {
    (void)d;
    return CFArrayCreate(NULL, NULL, 0, &kCFTypeArrayCallBacks);
}
EXPORT DSize SFBDisplayGetDeviceSize(CFTypeRef d) {
    return (DSize){ (double)DISP->device.w, (double)DISP->device.h };
}
EXPORT DSize SFBDisplayGetCurrentCanvasSize(CFTypeRef d) {
    return (DSize){ (double)DISP->canvas.w, (double)DISP->canvas.h };
}
EXPORT uint64_t SFBDisplayGetCanvasSizeCount(CFTypeRef d) {
    (void)d;
    return 0;
}
EXPORT DSize SFBDisplayGetCanvasSize(CFTypeRef d, uint64_t i) {
    (void)d;
    (void)i;
    return (DSize){ 0.0, 0.0 };
}
EXPORT bool SFBDisplaySetCanvasSize(CFTypeRef d, CFErrorRef *err, double w, double h) {
    (void)d;
    (void)err;
    (void)w;
    (void)h;
    return true;
}
EXPORT bool SFBDisplaySetCurrentUIOrientation(CFTypeRef d, uint32_t o, CFErrorRef *err) {
    (void)d;
    (void)o;
    (void)err;
    return true;
}
EXPORT uint64_t SFBDisplayGetSupportedSurfaceFlags(CFTypeRef d) {
    (void)d;
    return 0;
}
EXPORT uint64_t SFBDisplayGetFlags(CFTypeRef d) { return DISP->flags; }
EXPORT uint32_t SFBDisplayGetSupportedPresentationModes(CFTypeRef d) {
    (void)d;
    return 0xffffffffu;
}
EXPORT uint32_t SFBDisplayGetID(CFTypeRef d) { return DISP->display_id; }
EXPORT uint32_t SFBDisplayGetType(CFTypeRef d) { return DISP->type; }
EXPORT uint32_t SFBDisplayGetDotPitch(CFTypeRef d) { return DISP->dot_pitch; }
EXPORT uint16_t SFBDisplayGetMaxLayerCount(CFTypeRef d) {
    (void)d;
    return 1;
}
EXPORT uint16_t SFBDisplayGetMaxSwapchainCount(CFTypeRef d) {
    (void)d;
    return 1;
}
EXPORT CFTypeRef SFBDisplayGetMaskPath(CFTypeRef d) {
    (void)d;
    return NULL;
}
EXPORT bool SFBDisplaySetBacklightState(CFTypeRef d, int32_t s, CFErrorRef *err) {
    (void)d;
    (void)s;
    (void)err;
    return true;
}
EXPORT bool SFBDisplaySetBrightnessFactor(CFTypeRef d, CFErrorRef *err, double f) {
    (void)d;
    (void)err;
    (void)f;
    return true;
}
EXPORT CFTypeRef SFBDisplayCreateSwapchain(
    CFTypeRef d,
    uint32_t presentation_mode,
    NSDictionary *opts,
    CFErrorRef *err,
    double w,
    double h
) {
    (void)opts;
    (void)err;
    static uint32_t next_id = 1;
    WaterUISFBSwap *s = [WaterUISFBSwap new];
    s->display = DISP;
    s->swap_id = next_id++;
    s->presentation_mode = presentation_mode;
    s->size = (SimSize){ (uint32_t)w, (uint32_t)h };
    s->surfaces = [NSMutableArray array];
    s->rects = [NSMutableArray array];
    return CFBridgingRetain(s);
}
#undef DISP

// ---- SFBSwapchain ----

#define SWAP ((__bridge WaterUISFBSwap *)s)
EXPORT CFTypeID SFBSwapchainGetTypeID(void) { return 0; }
EXPORT uint32_t SFBSwapchainGetID(CFTypeRef s) { return SWAP->swap_id; }
EXPORT uint32_t SFBSwapchainGetDisplayID(CFTypeRef s) { return SWAP->display->display_id; }
EXPORT uint64_t SFBSwapchainGetConnectionID(CFTypeRef s) {
    (void)s;
    return 1;
}
EXPORT DSize SFBSwapchainGetFramebufferSize(CFTypeRef s) {
    return (DSize){ (double)SWAP->size.w, (double)SWAP->size.h };
}
EXPORT uint32_t SFBSwapchainGetPixelFormat(CFTypeRef s) { return SWAP->display->mode.pixel_format; }
EXPORT uint32_t SFBSwapchainGetColorspace(CFTypeRef s) { return SWAP->display->mode.colorspace; }
EXPORT uint32_t SFBSwapchainGetHDRMode(CFTypeRef s) {
    (void)s;
    return 0;
}
EXPORT uint32_t SFBSwapchainGetPresentationMode(CFTypeRef s) { return SWAP->presentation_mode; }
EXPORT uint64_t SFBSwapchainAcquireSurfaceFence(CFTypeRef s, IOSurfaceRef surf, uint64_t timeout) {
    (void)s;
    (void)surf;
    (void)timeout;
    return 1;
}
EXPORT uint64_t SFBSwapchainGetMaxSurfacesPerOperation(CFTypeRef s) {
    (void)s;
    return 32;
}
EXPORT bool SFBSwapchainSwapBegin(CFTypeRef s, uint64_t host_time, CFErrorRef *err) {
    (void)err;
    SWAP->pending = YES;
    SWAP->begin_time = host_time;
    SWAP->seq = atomic_fetch_add(&gSeq, 1) + 1;
    [SWAP->surfaces removeAllObjects];
    [SWAP->rects removeAllObjects];
    return true;
}
EXPORT bool SFBSwapchainSwapAddSurface(
    CFTypeRef s,
    IOSurfaceRef surf,
    uint64_t a3,
    uint64_t a4,
    CFErrorRef *err,
    double sx,
    double sy,
    double sw,
    double sh,
    double dx,
    double dy,
    double dw,
    double dh
) {
    (void)a3;
    (void)a4;
    (void)err;
    (void)dx;
    (void)dy;
    (void)dw;
    (void)dh;
    if (!SWAP->pending) {
        return false;
    }
    [SWAP->surfaces addObject:(__bridge id)surf];
    CGRect r = CGRectMake(sx, sy, sw, sh);
    [SWAP->rects addObject:[NSValue valueWithBytes:&r objCType:@encode(CGRect)]];
    return true;
}
EXPORT void SFBSwapchainSwapSetCallback(
    CFTypeRef s,
    void *ctx,
    dispatch_queue_t q,
    SFBSwapCallback blk
) {
    SWAP->cb_ctx = ctx;
    SWAP->cb_q = q;
    SWAP->cb = [blk copy];
}
EXPORT bool SFBSwapchainSwapSubmit(CFTypeRef s, CFErrorRef *err) {
    (void)err;
    WaterUISFBSwap *sc = SWAP;
    if (!sc->pending) {
        return false;
    }
    sc->pending = NO;
    uint64_t seq = sc->seq;
    for (NSUInteger i = 0; i < sc->surfaces.count; i++) {
        IOSurfaceRef surf = (__bridge IOSurfaceRef)sc->surfaces[i];
        CGRect rect = CGRectZero;
        [sc->rects[i] getValue:&rect size:sizeof(rect)];
        record_surface_frame(surf, rect, seq);
    }
    [sc->surfaces removeAllObjects];
    [sc->rects removeAllObjects];
    if (sc->cb) {
        SFBSwapCallback cb = sc->cb;
        void *ctx = sc->cb_ctx;
        sc->cb = nil;
        CFRetain(s);
        uint64_t t = mach_absolute_time();
        dispatch_queue_t q = sc->cb_q ?: dispatch_get_global_queue(QOS_CLASS_USER_INTERACTIVE, 0);
        dispatch_async(q, ^{
            cb(NULL, (__bridge id)s, t, t, ctx);
            CFRelease(s);
        });
    }
    return true;
}
EXPORT bool SFBSwapchainSwapCancel(CFTypeRef s, CFErrorRef *err) {
    (void)err;
    WaterUISFBSwap *sc = SWAP;
    [sc->surfaces removeAllObjects];
    [sc->rects removeAllObjects];
    sc->pending = NO;
    if (sc->cb) {
        SFBSwapCallback cb = sc->cb;
        void *ctx = sc->cb_ctx;
        sc->cb = nil;
        CFRetain(s);
        dispatch_async(dispatch_get_global_queue(0, 0), ^{
            cb(NULL, (__bridge id)s, 0, 0, ctx);
            CFRelease(s);
        });
    }
    return true;
}
#undef SWAP
