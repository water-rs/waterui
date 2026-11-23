#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>
#import <QuartzCore/QuartzCore.h>
#import <UIKit/UIKit.h>
#include <dlfcn.h>
#include <mach/mach.h>
#include <mach/mach_time.h>
#include <notify.h>
#include <objc/message.h>
#include <objc/runtime.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <xpc/xpc.h>

// ---- Interposer registration hooks (resolved via dlsym from libwaterui_sim_interpose.dylib) ----

typedef Boolean (*waterui_bks_handler_t)(void);
typedef void (*waterui_fbs_handler_t)(id workspace);
typedef void (*waterui_set_bks_fn)(waterui_bks_handler_t);
typedef void (*waterui_set_fbs_fn)(waterui_fbs_handler_t);

// ---- SimFramebuffer readback hooks (resolved via dlsym from SimFramebuffer) ----

typedef uint64_t (*waterui_simfb_frame_count_fn)(void);
typedef bool (*waterui_simfb_copy_latest_fn)(
    uint8_t *out_buf,
    size_t out_capacity,
    uint32_t *out_width,
    uint32_t *out_height,
    uint32_t *out_pixel_format,
    uint32_t *out_bytes_per_element,
    uint64_t *out_seq
);
typedef size_t (*waterui_simfb_copy_colorspace_name_fn)(char *out_buf, size_t out_capacity);

// ---- Private simulator symbols resolved dynamically via dlsym(RTLD_DEFAULT, ...) ----

typedef struct __IOHIDEvent *IOHIDEventRef;
typedef double IOHIDFloat;

typedef void (*gs_set_main_screen_info_fn)(double pixelW, double pixelH, float scale, float orient);
typedef kern_return_t (*bootstrap_look_up_fn)(mach_port_t bp, const char *name, mach_port_t *out);
typedef IOHIDEventRef (*iohid_create_digitizer_fn)(
    CFAllocatorRef,
    uint64_t ts,
    uint32_t transducerType,
    uint32_t index,
    uint32_t identity,
    uint32_t eventMask,
    uint32_t buttonMask,
    IOHIDFloat x,
    IOHIDFloat y,
    IOHIDFloat z,
    IOHIDFloat tipPressure,
    IOHIDFloat barrelPressure,
    Boolean range,
    Boolean touch,
    uint32_t options
);
typedef IOHIDEventRef (*iohid_create_finger_fn)(
    CFAllocatorRef,
    uint64_t ts,
    uint32_t index,
    uint32_t identity,
    uint32_t eventMask,
    IOHIDFloat x,
    IOHIDFloat y,
    IOHIDFloat z,
    IOHIDFloat tipPressure,
    IOHIDFloat twist,
    Boolean range,
    Boolean touch,
    uint32_t options
);
typedef void (*iohid_append_fn)(IOHIDEventRef parent, IOHIDEventRef child, uint32_t options);
typedef void (*iohid_set_int_fn)(IOHIDEventRef event, uint32_t field, long value);
typedef void (*iohid_set_float_fn)(IOHIDEventRef event, uint32_t field, IOHIDFloat value);
typedef void (*iohid_set_sender_fn)(IOHIDEventRef event, uint64_t senderID);
typedef void (*bks_set_digitizer_info_fn)(
    IOHIDEventRef event,
    uint32_t contextID,
    Boolean systemGesturePossible,
    Boolean systemGestureStateChange,
    CFStringRef displayUUID,
    double initialTouchTimestamp,
    double maxForce
);

// ---- ObjC runtime declarations for simulator system classes ----

@interface NSObject (WaterUISimPrivate)
- (void)performAsync:(dispatch_block_t)block;
- (void)_registerSource:(id)source;
- (id)delegate;
+ (id)nullEndpointForService:(NSString *)service instance:(NSString *)instance;
+ (NSString *)identifier;
- (void)handshakeWithRemnants:(id)remnants;
- (void)createSceneWithIdentity:(id)identity
                     parameters:(id)params
              transitionContext:(id)tc
                     completion:(void (^)(id))completion;
+ (id)identityForIdentifier:(NSString *)identifier workspaceIdentifier:(NSString *)workspace;
+ (id)parametersForSpecification:(id)spec;
+ (id)specification;
- (void)updateSettingsWithBlock:(void (^)(id))block;
- (void)setDisplay:(id)displayConfiguration;
- (id)settings;
- (id)displayConfiguration;
- (id)sceneWithIdentity:(id)identity;
- (id)clientSettings;
- (id)layers;
- (uint32_t)contextID;
- (void)_enqueueHIDEvent:(IOHIDEventRef)event;
- (uint32_t)_contextId;
+ (instancetype)serverWithOptions:(NSDictionary *)options;
+ (instancetype)remoteContextWithOptions:(NSDictionary *)options;
@property (strong) CALayer *layer;
@property (readonly) uint32_t contextId;
@end

@protocol WaterUIMutableSceneSettings
- (void)setFrame:(CGRect)frame;
- (void)setLevel:(double)level;
- (void)setInterfaceOrientation:(long long)orientation;
- (void)setForeground:(BOOL)foreground;
- (void)setDeactivationReasons:(unsigned long long)reasons;
- (void)setDeviceOrientation:(long long)orientation;
- (void)setUserInterfaceStyle:(long long)style;
- (void)setCanShowAlerts:(BOOL)canShowAlerts;
- (void)setStatusBarHeight:(double)height;
- (void)setDefaultStatusBarHeight:(double)height forOrientation:(long long)orientation;
- (void)setSafeAreaInsetsPortrait:(UIEdgeInsets)insets;
- (void)setSafeAreaInsetsPortraitUpsideDown:(UIEdgeInsets)insets;
- (void)setSafeAreaInsetsLandscapeLeft:(UIEdgeInsets)insets;
- (void)setSafeAreaInsetsLandscapeRight:(UIEdgeInsets)insets;
- (void)setPersistenceIdentifier:(NSString *)identifier;
@end

// ---- State ----

typedef void (*waterui_scene_ready_cb_t)(void *window_ptr, void *user_data);

static CGFloat gLogicalWidth = 375.0;
static CGFloat gLogicalHeight = 667.0;
static CGFloat gScale = 2.0;
static int32_t gColorSchemeOverride = 0; // 0 = system/light, 1 = light, 2 = dark
static waterui_scene_ready_cb_t gSceneReadyCb;
static void *gSceneReadyUserData;

static NSString *gSceneID;
static id gWindowServer;
static id gHostContext;
static CALayer *gHostRoot;
static NSMutableArray<NSNumber *> *gHostedContextIDs;
static id gWorkspace;
static id gScenesClient;
static id gFakeHost;
static id gFakeConnection;
static id gSceneIdentity;
static NSMutableArray *gSinkPeers;

static waterui_simfb_frame_count_fn gSimfbFrameCount;
static waterui_simfb_copy_latest_fn gSimfbCopyLatest;
static waterui_simfb_copy_colorspace_name_fn gSimfbCopyColorSpace;

static void set_ivar_if_present(id obj, const char *name, id value) {
    Ivar iv = class_getInstanceVariable(object_getClass(obj), name);
    if (iv) {
        object_setIvarWithStrongDefault(obj, iv, value);
    }
}

static void set_ivar_required(id obj, const char *name, id value) {
    Ivar iv = class_getInstanceVariable(object_getClass(obj), name);
    if (!iv) {
        fprintf(
            stderr,
            "waterui-ios-sim: missing ivar %s on %s\n",
            name,
            object_getClassName(obj)
        );
        abort();
    }
    object_setIvarWithStrongDefault(obj, iv, value);
}

static void call_completion_nil(id block) {
    if (!block) {
        return;
    }
    ((void (^)(id, id))block)(nil, nil);
}

// ---- Multi-CALayerHost scene compositor ----

static void update_hosted_layers(NSArray<NSNumber *> *ids) {
    if ([ids isEqual:gHostedContextIDs]) {
        return;
    }
    gHostedContextIDs = [ids mutableCopy];

    if (!gHostContext) {
        NSString *displayableKey = @"displayable";
        NSString *const *sym =
            (NSString *const *)dlsym(RTLD_DEFAULT, "kCAContextDisplayable");
        if (sym && *sym) {
            displayableKey = *sym;
        }
        Class caContextCls = NSClassFromString(@"CAContext");
        gHostContext = [caContextCls remoteContextWithOptions:@{ displayableKey: @YES }];

        CGFloat pixelW = round(gLogicalWidth * gScale);
        CGFloat pixelH = round(gLogicalHeight * gScale);
        gHostRoot = [CALayer layer];
        gHostRoot.anchorPoint = CGPointZero;
        gHostRoot.bounds = CGRectMake(0.0, 0.0, pixelW, pixelH);
        gHostRoot.position = CGPointZero;
        CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
        CGFloat clear[4] = { 0.0, 0.0, 0.0, 0.0 };
        CGColorRef bg = CGColorCreate(cs, clear);
        gHostRoot.backgroundColor = bg;
        CGColorRelease(bg);
        CGColorSpaceRelease(cs);
        [gHostContext setLayer:gHostRoot];
    }

    Class layerHostCls = NSClassFromString(@"CALayerHost");
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    gHostRoot.sublayers = nil;
    for (NSNumber *cid in ids) {
        CALayer *hostLayer = [layerHostCls layer];
        ((void (*)(id, SEL, uint32_t))objc_msgSend)(
            hostLayer,
            NSSelectorFromString(@"setContextId:"),
            cid.unsignedIntValue
        );
        hostLayer.anchorPoint = CGPointZero;
        hostLayer.bounds = CGRectMake(0.0, 0.0, gLogicalWidth, gLogicalHeight);
        hostLayer.transform = CATransform3DMakeScale(gScale, gScale, 1.0);
        hostLayer.position = CGPointZero;
        [gHostRoot addSublayer:hostLayer];
    }
    [CATransaction commit];
    [CATransaction flush];
}

static void report_scene_layers(void) {
    id scene = [gScenesClient sceneWithIdentity:gSceneIdentity];
    id cs = [scene clientSettings];
    id layers = [cs respondsToSelector:@selector(layers)] ? [cs layers] : nil;

    typedef struct {
        uint32_t cid;
        double level;
        size_t order;
    } layer_entry_t;

    layer_entry_t entries[32];
    size_t count = 0;
    for (id layer in layers) {
        if (count >= 32) {
            break;
        }
        if ([layer respondsToSelector:@selector(contextID)]) {
            entries[count].cid = [layer contextID];
            SEL levelSel = NSSelectorFromString(@"level");
            entries[count].level = [layer respondsToSelector:levelSel]
                ? ((double (*)(id, SEL))objc_msgSend)(layer, levelSel)
                : 0.0;
            entries[count].order = count;
            count++;
        }
    }

    // Stable insertion sort by ascending window level (bottom -> top).
    for (size_t i = 1; i < count; i++) {
        layer_entry_t key = entries[i];
        size_t j = i;
        while (j > 0 && entries[j - 1].level > key.level) {
            entries[j] = entries[j - 1];
            j--;
        }
        entries[j] = key;
    }

    NSMutableArray<NSNumber *> *ordered = [NSMutableArray arrayWithCapacity:count];
    for (size_t i = 0; i < count; i++) {
        [ordered addObject:@(entries[i].cid)];
    }
    dispatch_async(dispatch_get_main_queue(), ^{
        update_hosted_layers(ordered);
    });
}

// ---- In-process FrontBoard host objects ----

@interface WaterUISimFakeHost : NSObject
@end

@implementation WaterUISimFakeHost
- (void)handshakeWithRemnants:(id)remnants {
    (void)remnants;
}
- (void)sceneID:(id)sid
    didUpdateClientSettingsWithDiff:(id)diff
                  transitionContext:(id)tc
                         completion:(void (^)(id))completion {
    (void)sid;
    (void)diff;
    (void)tc;
    report_scene_layers();
    if (completion) {
        completion(nil);
    }
}
- (void)sceneID:(id)sid didReceiveActions:(NSSet *)actions forExtension:(id)ext {
    (void)sid;
    (void)actions;
    (void)ext;
}
- (void)sceneID:(id)sid invalidateWithContext:(id)ctx clientError:(id)err {
    (void)sid;
    (void)ctx;
    (void)err;
}
- (void)sceneID:(id)sid sendMessage:(id)msg withResponse:(id)resp {
    (void)sid;
    (void)msg;
    call_completion_nil(resp);
}
- (void)sceneID:(id)sid handleInvocation:(id)inv completion:(id)completion {
    (void)sid;
    (void)inv;
    call_completion_nil(completion);
}
- (void)workspaceID:(id)wid sendActions:(id)actions completion:(id)completion {
    (void)wid;
    (void)actions;
    call_completion_nil(completion);
}
- (void)requestSceneWithOptions:(id)opts completion:(id)completion {
    (void)opts;
    call_completion_nil(completion);
}
- (void)createSceneWithOptions:(id)opts completion:(id)completion {
    (void)opts;
    call_completion_nil(completion);
}
- (NSMethodSignature *)methodSignatureForSelector:(SEL)selector {
    NSMethodSignature *sig = [super methodSignatureForSelector:selector];
    return sig ?: [NSMethodSignature signatureWithObjCTypes:"v@:@@@@"];
}
- (void)forwardInvocation:(NSInvocation *)invocation {
    (void)invocation;
}
@end

@interface WaterUISimFakeConnection : NSObject
@property (strong) id localTarget;
@property (strong) id queue;
@property (strong) id interface;
@property (strong) id serviceQuality;
@property (strong) id activationHandler;
@property (strong) id interruptionHandler;
@property (strong) id invalidationHandler;
@property (strong) id batchingHandler;
@end

@implementation WaterUISimFakeConnection
- (void)configure:(void (^)(id))block {
    if (block) {
        block(self);
    }
}
- (void)activate {}
- (void)invalidate {}
- (id)remoteTarget { return gFakeHost; }
- (id)immediateMessagingRemoteTargetWithLaunchingAssertionAttributes:(id)attrs {
    (void)attrs;
    return gFakeHost;
}
- (id)activeMessageBatch { return nil; }
- (id)remoteToken { return nil; }
- (id)remoteProcess { return nil; }
- (NSMethodSignature *)methodSignatureForSelector:(SEL)selector {
    NSMethodSignature *sig = [super methodSignatureForSelector:selector];
    return sig ?: [NSMethodSignature signatureWithObjCTypes:"@@:@@@@"];
}
- (void)forwardInvocation:(NSInvocation *)invocation {
    id nilVal = nil;
    if (invocation.methodSignature.methodReturnLength == sizeof(id)) {
        [invocation setReturnValue:&nilVal];
    }
}
@end

static void workspace_async(dispatch_block_t block) {
    id q = [gWorkspace valueForKey:@"_queue"];
    if ([q respondsToSelector:@selector(performAsync:)]) {
        [q performAsync:block];
    } else {
        dispatch_async(q, block);
    }
}

static void apply_scene_settings(id<WaterUIMutableSceneSettings> settings) {
    [settings setFrame:CGRectMake(0.0, 0.0, gLogicalWidth, gLogicalHeight)];
    [settings setLevel:1.0];
    [settings setInterfaceOrientation:UIInterfaceOrientationPortrait];
    [settings setForeground:YES];
    if ([(id)settings respondsToSelector:@selector(setDeactivationReasons:)]) {
        [settings setDeactivationReasons:0];
        [settings setDeviceOrientation:UIDeviceOrientationPortrait];
        UIUserInterfaceStyle style =
            (gColorSchemeOverride == 2) ? UIUserInterfaceStyleDark : UIUserInterfaceStyleLight;
        [settings setUserInterfaceStyle:style];
        [settings setCanShowAlerts:YES];
        // Preview viewports fill the exact requested frame without device notch/home insets.
        [settings setStatusBarHeight:0.0];
        [settings setDefaultStatusBarHeight:0.0 forOrientation:UIInterfaceOrientationPortrait];
        [settings setSafeAreaInsetsPortrait:UIEdgeInsetsZero];
        [settings setSafeAreaInsetsPortraitUpsideDown:UIEdgeInsetsZero];
        [settings setSafeAreaInsetsLandscapeLeft:UIEdgeInsetsZero];
        [settings setSafeAreaInsetsLandscapeRight:UIEdgeInsetsZero];
        [settings setPersistenceIdentifier:gSceneID];
    }
}

static void create_preview_scene(void) {
    Class specCls = NSClassFromString(@"UIApplicationSceneSpecification");
    Class paramsCls = NSClassFromString(@"FBSMutableSceneParameters");
    Class identityCls = NSClassFromString(@"FBSSceneIdentity");
    Class tcCls = NSClassFromString(@"UIApplicationSceneTransitionContext");

    id spec = [specCls specification];
    id params = [paramsCls parametersForSpecification:spec];
    id mainScreen =
        ((id (*)(Class, SEL))objc_msgSend)([UIScreen class], NSSelectorFromString(@"mainScreen"));
    id displayConfig = mainScreen ? [mainScreen displayConfiguration] : nil;
    if (displayConfig) {
        [params setDisplay:displayConfig];
    }
    [params updateSettingsWithBlock:^(id settings) {
        apply_scene_settings(settings);
    }];

    gSceneIdentity = [identityCls identityForIdentifier:gSceneID
                                    workspaceIdentifier:@"com.apple.frontboard.systemappservices"];
    id tc = [tcCls new];
    [gScenesClient createSceneWithIdentity:gSceneIdentity
                                parameters:params
                         transitionContext:tc
                                completion:^(id err) {
                                    if (err) {
                                        fprintf(
                                            stderr,
                                            "waterui-ios-sim: createScene error: %s\n",
                                            [[err description] UTF8String]
                                        );
                                    }
                                }];
}

static void on_fbs_workspace_initialized(id ws) {
    gWorkspace = ws;
    id q = [ws valueForKey:@"_queue"];
    id cq = [ws valueForKey:@"_callOutQueue"];
    gFakeHost = [WaterUISimFakeHost new];
    gFakeConnection = [WaterUISimFakeConnection new];

    Class identCls = NSClassFromString(@"FBSWorkspaceScenesClientIdentifier");
    id ident = [identCls new];
    id endpoint = [NSClassFromString(@"BSServiceConnectionEndpoint")
        nullEndpointForService:[NSClassFromString(@"FBSWorkspaceServiceSpecification") identifier]
                      instance:nil];
    set_ivar_required(ident, "_hostEndpoint", endpoint);
    set_ivar_required(ident, "_description", @"waterui-preview-local-host");

    Class clientCls = NSClassFromString(@"FBSWorkspaceScenesClient");
    id client = class_createInstance(clientCls, 0);
    set_ivar_required(client, "_workspace", ws);
    set_ivar_required(client, "_workspaceDelegate", [ws delegate]);
    set_ivar_required(client, "_queue", q);
    set_ivar_required(client, "_callOutQueue", cq);
    set_ivar_required(client, "_connection", gFakeConnection);
    set_ivar_required(client, "_identifier", ident);
    set_ivar_required(client, "_scenesByIdentity", [NSMutableDictionary dictionary]);
    set_ivar_required(client, "_clientSettingsByIdentity", [NSMutableDictionary dictionary]);
    set_ivar_required(
        client,
        "_reconnectingScenes",
        [NSHashTable hashTableWithOptions:NSPointerFunctionsWeakMemory]
    );
    set_ivar_required(client, "_pendedSendBlocks", nil);
    set_ivar_if_present(client, "_hostEvent_pendingEvents", [NSMutableArray array]);
    set_ivar_required(client, "_reportingLock_scenesByIdentity", [NSMutableDictionary dictionary]);
    set_ivar_required(client, "_handshakeLock_remoteTarget", gFakeHost);
    Ivar activatedIvar = class_getInstanceVariable(object_getClass(client), "_activated");
    if (activatedIvar) {
        *(BOOL *)((char *)(__bridge void *)client + ivar_getOffset(activatedIvar)) = YES;
    }

    gScenesClient = client;
    [ws _registerSource:client];
    [gFakeHost handshakeWithRemnants:nil];
    workspace_async(^{
        create_preview_scene();
    });
}

static void enable_backlight_notify_state(void) {
    static const char *kBacklightKey = "com.apple.backboardd.backlight.changed";
    int token = 0;
    if (notify_register_check(kBacklightKey, &token) == NOTIFY_STATUS_OK) {
        uint64_t state = 0;
        notify_get_state(token, &state);
        if (state != 100 && notify_set_state(token, 100) == NOTIFY_STATUS_OK) {
            notify_post(kBacklightKey);
        }
    }
}

static Boolean on_bks_display_services_start(void) {
    gs_set_main_screen_info_fn set_screen =
        (gs_set_main_screen_info_fn)dlsym(RTLD_DEFAULT, "GSSetMainScreenInfo");
    if (set_screen) {
        set_screen(
            round(gLogicalWidth * gScale),
            round(gLogicalHeight * gScale),
            (float)gScale,
            0.0f
        );
    }
    enable_backlight_notify_state();
    return 1;
}

// ---- Swizzles for headless operation without backboardd / runningboardd / lsd ----

static id make_sink_endpoint(NSString *mach, NSString *service, NSString *instance) {
    static xpc_connection_t listener;
    static dispatch_queue_t q;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        gSinkPeers = [NSMutableArray array];
        q = dispatch_queue_create("dev.waterui.preview.hid-sink", DISPATCH_QUEUE_SERIAL);
        listener = xpc_connection_create(NULL, q);
        xpc_connection_set_event_handler(listener, ^(xpc_object_t peer) {
            if (xpc_get_type(peer) != XPC_TYPE_CONNECTION) {
                return;
            }
            [gSinkPeers addObject:peer];
            xpc_connection_set_event_handler((xpc_connection_t)peer, ^(xpc_object_t msg) {
                (void)msg;
            });
            xpc_connection_resume((xpc_connection_t)peer);
        });
        xpc_connection_resume(listener);
    });

    id ep = class_createInstance(NSClassFromString(@"BSServiceConnectionEndpoint"), 0);
    set_ivar_required(ep, "_service", service);
    set_ivar_required(ep, "_instance", instance);
    id xpcEndpoint = (id)xpc_endpoint_create(listener);
    NSString *target = [@"sink:" stringByAppendingString:mach];
    Class xpcCls = NSClassFromString(@"BSXPCServiceConnectionEndpoint");
    if (xpcCls) {
        id underlying = class_createInstance(xpcCls, 0);
        set_ivar_required(underlying, "_xpcEndpoint", xpcEndpoint);
        set_ivar_required(underlying, "_targetDescription", target);
        set_ivar_required(ep, "_underlyingEndpoint", underlying);
    } else {
        set_ivar_required(ep, "_endpoint", xpcEndpoint);
        set_ivar_required(ep, "_targetDescription", target);
    }
    return ep;
}

static IMP gOrigEndpointForMachName;
static id swizzled_endpoint_for_mach_name(
    id self,
    SEL _cmd,
    NSString *mach,
    NSString *service,
    NSString *instance
) {
    id ep = ((id (*)(id, SEL, id, id, id))gOrigEndpointForMachName)(
        self,
        _cmd,
        mach,
        service,
        instance
    );
    if (!ep && [mach isEqualToString:@"com.apple.backboard.hid-services.xpc"]) {
        ep = make_sink_endpoint(mach, service, instance);
    }
    return ep;
}

static IMP gOrigAcquireAssertion;
static id swizzled_acquire_assertion(id self, SEL _cmd, id assertion, NSError **error) {
    static BOOL missing;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        bootstrap_look_up_fn lookup =
            (bootstrap_look_up_fn)dlsym(RTLD_DEFAULT, "bootstrap_look_up");
        mach_port_t p = MACH_PORT_NULL;
        missing = !lookup || (lookup(bootstrap_port, "com.apple.runningboard", &p) != KERN_SUCCESS);
        if (p) {
            mach_port_deallocate(mach_task_self(), p);
        }
    });
    if (!missing) {
        return ((id (*)(id, SEL, id, NSError **))gOrigAcquireAssertion)(
            self,
            _cmd,
            assertion,
            error
        );
    }
    if (error) {
        *error = [NSError
            errorWithDomain:NSCocoaErrorDomain
                       code:NSXPCConnectionInvalid
                   userInfo:@{ NSLocalizedFailureReasonErrorKey: @"runningboard unavailable" }];
    }
    return nil;
}

static IMP gOrigConformsToType;
static BOOL swizzled_conforms_to_type(id self, SEL _cmd, id type) {
    return type ? ((BOOL (*)(id, SEL, id))gOrigConformsToType)(self, _cmd, type) : NO;
}

// ---- UIApplication / UIWindowScene Delegates ----

@interface WaterUIPreviewSceneDelegate : UIResponder <UIWindowSceneDelegate>
@property (strong, nonatomic) UIWindow *window;
@end

@implementation WaterUIPreviewSceneDelegate
- (void)scene:(UIScene *)scene
    willConnectToSession:(UISceneSession *)session
                 options:(UISceneConnectionOptions *)connectionOptions {
    (void)session;
    (void)connectionOptions;
    if (![scene isKindOfClass:[UIWindowScene class]]) {
        return;
    }
    UIWindowScene *windowScene = (UIWindowScene *)scene;
    UIWindow *window = [[UIWindow alloc] initWithWindowScene:windowScene];
    window.frame = CGRectMake(0.0, 0.0, gLogicalWidth, gLogicalHeight);
    if (gColorSchemeOverride == 1) {
        window.overrideUserInterfaceStyle = UIUserInterfaceStyleLight;
    } else if (gColorSchemeOverride == 2) {
        window.overrideUserInterfaceStyle = UIUserInterfaceStyleDark;
    }
    self.window = window;

    if (gSceneReadyCb) {
        waterui_scene_ready_cb_t cb = gSceneReadyCb;
        void *ud = gSceneReadyUserData;
        gSceneReadyCb = NULL;
        gSceneReadyUserData = NULL;
        cb((__bridge void *)window, ud);
    }
}
@end

@interface WaterUIPreviewAppDelegate : UIResponder <UIApplicationDelegate>
@end

@implementation WaterUIPreviewAppDelegate
- (UISceneConfiguration *)application:(UIApplication *)application
    configurationForConnectingSceneSession:(UISceneSession *)connectingSceneSession
                                   options:(UISceneConnectionOptions *)options {
    (void)application;
    (void)options;
    UISceneConfiguration *config =
        [[UISceneConfiguration alloc] initWithName:@"WaterUIPreview"
                                       sessionRole:connectingSceneSession.role];
    config.delegateClass = [WaterUIPreviewSceneDelegate class];
    return config;
}
@end

// ---- HID Touch & Scroll Injection ----

enum {
    WATERUI_TOUCH_BEGAN = 0,
    WATERUI_TOUCH_MOVED = 1,
    WATERUI_TOUCH_ENDED = 2,
    WATERUI_TOUCH_CANCELLED = 3,
};

enum {
    kTransducerHand = 3,
    kMaskRange = 0x1,
    kMaskTouch = 0x2,
    kMaskPosition = 0x4,
    kMaskIdentity = 0x20,
    kMaskCancel = 0x80,
};

#define kFieldDigitizerIsDisplayIntegrated ((11u << 16) | 25u)
#define kFieldDigitizerMajorRadius ((11u << 16) | 20u)
#define kFieldDigitizerMinorRadius ((11u << 16) | 21u)

static const uint64_t kDigitizerSenderID = 0x000000010000AB1DULL;

typedef struct {
    bool down;
    float x;
    float y;
    uint32_t identity;
    uint32_t ctx;
} waterui_finger_t;

static waterui_finger_t gFinger;
static uint32_t gNextTouchIdentity = 2;
static uint64_t gLastTouchTimestamp;

static UIWindowScene *active_window_scene(void) {
    UIWindowScene *best = nil;
    for (UIScene *s in UIApplication.sharedApplication.connectedScenes) {
        if (![s isKindOfClass:[UIWindowScene class]]) {
            continue;
        }
        if (!best || s.activationState == UISceneActivationStateForegroundActive) {
            best = (UIWindowScene *)s;
        }
    }
    return best;
}

static UIWindow *hit_test_window_at(CGPoint point) {
    UIWindowScene *ws = active_window_scene();
    if (!ws) {
        return nil;
    }
    NSArray<UIWindow *> *sorted = [ws.windows
        sortedArrayWithOptions:NSSortStable
               usingComparator:^NSComparisonResult(UIWindow *a, UIWindow *b) {
                   if (a.windowLevel < b.windowLevel) {
                       return NSOrderedDescending;
                   }
                   if (a.windowLevel > b.windowLevel) {
                       return NSOrderedAscending;
                   }
                   return NSOrderedSame;
               }];

    UIWindow *hit = nil;
    CGFloat hitLevel = 0.0;
    BOOL hasHit = NO;
    for (UIWindow *w in sorted) {
        if (hasHit && w.windowLevel != hitLevel) {
            break;
        }
        if (w.hidden || w.alpha < 0.01 || !w.userInteractionEnabled || ![w _contextId]) {
            continue;
        }
        id<UICoordinateSpace> sceneSpace = ws.effectiveGeometry.coordinateSpace;
        CGPoint local = [w convertPoint:point fromCoordinateSpace:sceneSpace];
        if ([w hitTest:local withEvent:nil]) {
            hit = w;
            hasHit = YES;
            hitLevel = w.windowLevel;
        }
    }
    return hit ?: ws.keyWindow ?: ws.windows.firstObject;
}

void waterui_ios_sim_dispatch_touch(int32_t phase, float x, float y) {
    static iohid_create_digitizer_fn create_digitizer;
    static iohid_create_finger_fn create_finger;
    static iohid_append_fn append_event;
    static iohid_set_int_fn set_int;
    static iohid_set_float_fn set_float;
    static iohid_set_sender_fn set_sender;
    static bks_set_digitizer_info_fn set_bks_info;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        create_digitizer =
            (iohid_create_digitizer_fn)dlsym(RTLD_DEFAULT, "IOHIDEventCreateDigitizerEvent");
        create_finger =
            (iohid_create_finger_fn)dlsym(RTLD_DEFAULT, "IOHIDEventCreateDigitizerFingerEvent");
        append_event = (iohid_append_fn)dlsym(RTLD_DEFAULT, "IOHIDEventAppendEvent");
        set_int = (iohid_set_int_fn)dlsym(RTLD_DEFAULT, "IOHIDEventSetIntegerValue");
        set_float = (iohid_set_float_fn)dlsym(RTLD_DEFAULT, "IOHIDEventSetFloatValue");
        set_sender = (iohid_set_sender_fn)dlsym(RTLD_DEFAULT, "IOHIDEventSetSenderID");
        set_bks_info =
            (bks_set_digitizer_info_fn)dlsym(RTLD_DEFAULT, "BKSHIDEventSetDigitizerInfo");
    });

    if (!create_digitizer || !create_finger || !append_event || !set_bks_info) {
        return;
    }

    if (phase == WATERUI_TOUCH_BEGAN) {
        if (gFinger.down) {
            waterui_ios_sim_dispatch_touch(WATERUI_TOUCH_ENDED, gFinger.x, gFinger.y);
        }
        UIWindow *target = hit_test_window_at(CGPointMake(x, y));
        if (!target) {
            return;
        }
        uint32_t ctx = [target _contextId];
        if (!ctx) {
            return;
        }
        gFinger = (waterui_finger_t){
            .down = true,
            .x = x,
            .y = y,
            .identity = gNextTouchIdentity++,
            .ctx = ctx,
        };
    } else {
        if (!gFinger.down) {
            return;
        }
        gFinger.x = x;
        gFinger.y = y;
    }

    bool ending = (phase == WATERUI_TOUCH_ENDED || phase == WATERUI_TOUCH_CANCELLED);
    uint64_t ts = mach_absolute_time();
    if (ts < gLastTouchTimestamp) {
        ts = gLastTouchTimestamp;
    }
    gLastTouchTimestamp = ts;

    uint32_t mask = (phase == WATERUI_TOUCH_MOVED)
        ? kMaskPosition
        : (kMaskRange | kMaskTouch | kMaskIdentity);
    if (phase == WATERUI_TOUCH_CANCELLED) {
        mask |= kMaskCancel;
    }
    Boolean touching = !ending;

    IOHIDEventRef hand = create_digitizer(
        kCFAllocatorDefault,
        ts,
        kTransducerHand,
        0,
        0,
        mask,
        0,
        x,
        y,
        0.0,
        0.0,
        0.0,
        touching,
        touching,
        0
    );
    if (!hand) {
        if (ending) {
            gFinger.down = false;
        }
        return;
    }
    if (set_int) {
        set_int(hand, kFieldDigitizerIsDisplayIntegrated, 1);
    }

    IOHIDEventRef child = create_finger(
        kCFAllocatorDefault,
        ts,
        1,
        gFinger.identity,
        mask,
        gFinger.x,
        gFinger.y,
        0.0,
        touching ? 1.0 : 0.0,
        0.0,
        touching,
        touching,
        0
    );
    if (child) {
        if (set_float) {
            set_float(child, kFieldDigitizerMajorRadius, 5.0);
            set_float(child, kFieldDigitizerMinorRadius, 5.0);
        }
        append_event(hand, child, 0);
        CFRelease(child);
    }
    if (set_sender) {
        set_sender(hand, kDigitizerSenderID);
    }
    set_bks_info(hand, gFinger.ctx, false, false, NULL, 0.0, 0.0);
    [UIApplication.sharedApplication _enqueueHIDEvent:hand];
    CFRelease(hand);

    if (ending) {
        gFinger.down = false;
    }
}

void waterui_ios_sim_dispatch_scroll(float x, float y, float dx, float dy) {
    waterui_ios_sim_dispatch_touch(WATERUI_TOUCH_BEGAN, x, y);
    const int steps = 4;
    for (int i = 1; i <= steps; i++) {
        float t = (float)i / (float)steps;
        waterui_ios_sim_dispatch_touch(WATERUI_TOUCH_MOVED, x + dx * t, y + dy * t);
    }
    waterui_ios_sim_dispatch_touch(WATERUI_TOUCH_ENDED, x + dx, y + dy);
}

// ---- Framebuffer readback bridges ----

static void resolve_simfb_symbols(void) {
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        gSimfbFrameCount =
            (waterui_simfb_frame_count_fn)dlsym(RTLD_DEFAULT, "waterui_simfb_frame_count");
        gSimfbCopyLatest =
            (waterui_simfb_copy_latest_fn)dlsym(RTLD_DEFAULT, "waterui_simfb_copy_latest");
        gSimfbCopyColorSpace = (waterui_simfb_copy_colorspace_name_fn)dlsym(
            RTLD_DEFAULT,
            "waterui_simfb_copy_colorspace_name"
        );
    });
}

uint64_t waterui_ios_sim_frame_count(void) {
    resolve_simfb_symbols();
    return gSimfbFrameCount ? gSimfbFrameCount() : 0;
}

bool waterui_ios_sim_has_hosted_layers(void) {
    return gHostedContextIDs && gHostedContextIDs.count > 0;
}

void waterui_ios_sim_request_present(void) {
    if (gHostRoot) {
        [CATransaction begin];
        [CATransaction setDisableActions:YES];
        // Perturb zPosition on the root orthographic layer to trigger a fresh composite
        // with zero pixel/alpha effect.
        gHostRoot.zPosition = (gHostRoot.zPosition == 0.0) ? 0.0001 : 0.0;
        [gHostRoot setNeedsDisplay];
        [CATransaction commit];
        [CATransaction flush];
    }
}

bool waterui_ios_sim_copy_latest_frame(
    uint8_t *out_buf,
    size_t out_capacity,
    uint32_t *out_width,
    uint32_t *out_height,
    uint32_t *out_pixel_format,
    uint32_t *out_bytes_per_element,
    uint64_t *out_seq
) {
    resolve_simfb_symbols();
    if (!gSimfbCopyLatest) {
        return false;
    }
    return gSimfbCopyLatest(
        out_buf,
        out_capacity,
        out_width,
        out_height,
        out_pixel_format,
        out_bytes_per_element,
        out_seq
    );
}

size_t waterui_ios_sim_copy_colorspace_name(char *out_buf, size_t out_capacity) {
    resolve_simfb_symbols();
    if (!gSimfbCopyColorSpace) {
        return 0;
    }
    return gSimfbCopyColorSpace(out_buf, out_capacity);
}

// ---- Entry points called from Rust (`preview::ios_sim::drive`) ----

int32_t waterui_ios_sim_preview_bootstrap(
    double logical_width,
    double logical_height,
    double scale,
    int32_t color_scheme_override
) {
    @autoreleasepool {
        gLogicalWidth = logical_width > 0.0 ? logical_width : 375.0;
        gLogicalHeight = logical_height > 0.0 ? logical_height : 667.0;
        gScale = scale > 0.0 ? scale : 2.0;
        gColorSchemeOverride = color_scheme_override;
        gSceneID = [NSString
            stringWithFormat:@"sceneID:%@-preview",
                             NSBundle.mainBundle.bundleIdentifier ?: @"dev.waterui.preview"];

        // Wire interposers exported by libwaterui_sim_interpose.dylib BEFORE
        // anything touches UIScreen / FBSWorkspace / BKSDisplayServices.
        waterui_set_bks_fn set_bks =
            (waterui_set_bks_fn)dlsym(RTLD_DEFAULT, "waterui_sim_set_bks_handler");
        waterui_set_fbs_fn set_fbs =
            (waterui_set_fbs_fn)dlsym(RTLD_DEFAULT, "waterui_sim_set_fbs_handler");
        if (!set_bks || !set_fbs) {
            fprintf(
                stderr,
                "waterui-ios-sim: interposer hooks missing — launch via `water preview --platform ios`\n"
            );
            return -1;
        }
        set_bks(on_bks_display_services_start);
        set_fbs(on_fbs_workspace_initialized);

        // Install lightweight swizzles so UIKit does not stall waiting on backboardd/runningboardd/lsd.
        Method mEndpoint = class_getClassMethod(
            NSClassFromString(@"BSServiceConnectionEndpoint"),
            NSSelectorFromString(@"endpointForMachName:service:instance:")
        );
        if (mEndpoint) {
            gOrigEndpointForMachName =
                method_setImplementation(mEndpoint, (IMP)swizzled_endpoint_for_mach_name);
        }
        Method mAssertion = class_getInstanceMethod(
            NSClassFromString(@"RBSConnection"),
            NSSelectorFromString(@"acquireAssertion:error:")
        );
        if (mAssertion) {
            gOrigAcquireAssertion =
                method_setImplementation(mAssertion, (IMP)swizzled_acquire_assertion);
        }
        Method mConforms =
            class_getInstanceMethod(NSClassFromString(@"UTType"), @selector(conformsToType:));
        if (mConforms) {
            gOrigConformsToType =
                method_setImplementation(mConforms, (IMP)swizzled_conforms_to_type);
        }

        // Start in-process Core Animation window server (loads SimFramebuffer)
        // before startup queries UIScreen / CADisplay.
        Class wsCls = NSClassFromString(@"CAWindowServer");
        if (!wsCls) {
            fprintf(stderr, "waterui-ios-sim: CAWindowServer class unavailable\n");
            return -2;
        }
        gWindowServer = [wsCls serverWithOptions:@{}];
        if (!gWindowServer) {
            fprintf(stderr, "waterui-ios-sim: CAWindowServer failed to start\n");
            return -3;
        }
        return 0;
    }
}

int32_t waterui_ios_sim_preview_enter_app(
    waterui_scene_ready_cb_t on_scene_ready,
    void *user_data
) {
    @autoreleasepool {
        gSceneReadyCb = on_scene_ready;
        gSceneReadyUserData = user_data;
        char prog[] = "waterui-preview";
        char *argv[] = { prog, NULL };
        return UIApplicationMain(
            1,
            argv,
            nil,
            NSStringFromClass([WaterUIPreviewAppDelegate class])
        );
    }
}
