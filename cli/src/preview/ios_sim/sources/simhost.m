#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#include <dispatch/dispatch.h>
#include <dlfcn.h>
#include <mach/mach.h>
#include <signal.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>
#include <xpc/xpc.h>

#include "simns_proto.h"

@interface IOSurfaceRemoteServer : NSObject
- (instancetype)initWithListener:(xpc_connection_t)listener options:(NSDictionary *)options;
@end

extern xpc_endpoint_t xpc_endpoint_create(xpc_connection_t connection);
extern mach_port_t xpc_endpoint_copy_listener_port_4sim(xpc_endpoint_t endpoint);
extern void xpc_connection_enable_sim2host_4sim(xpc_connection_t connection);
extern int posix_spawnattr_setspecialport_np(posix_spawnattr_t *attr, mach_port_t port, int which);
extern char **environ;

static IOSurfaceRemoteServer *gIOSurfaceServer;
static xpc_connection_t gIOSurfaceListener;
static xpc_connection_t gMetalListener;
static pid_t gSimnsPid;
static pid_t gAppPid;
static dispatch_source_t gAppExitSource;
static dispatch_source_t gParentExitSource;
static dispatch_source_t gSigTermSource;
static dispatch_source_t gSigIntSource;

static mach_port_t listener_port(xpc_connection_t listener) {
    xpc_endpoint_t ep = xpc_endpoint_create(listener);
    return ep ? xpc_endpoint_copy_listener_port_4sim(ep) : MACH_PORT_NULL;
}

static int start_gpu_services(mach_port_t *out_ios_port, mach_port_t *out_mtl_port) {
    *out_ios_port = MACH_PORT_NULL;
    *out_mtl_port = MACH_PORT_NULL;

    dispatch_queue_t ios_queue =
        dispatch_queue_create("dev.waterui.simhost.iosurface", DISPATCH_QUEUE_SERIAL);
    gIOSurfaceListener = xpc_connection_create(NULL, ios_queue);
    xpc_connection_enable_sim2host_4sim(gIOSurfaceListener);
    Class server_cls = NSClassFromString(@"IOSurfaceRemoteServer");
    if (!server_cls) {
        fprintf(stderr, "waterui-simhost: IOSurfaceRemoteServer class unavailable\n");
        return -1;
    }
    gIOSurfaceServer = [[server_cls alloc] initWithListener:gIOSurfaceListener options:@{}];
    if (!gIOSurfaceServer) {
        fprintf(stderr, "waterui-simhost: IOSurfaceRemoteServer init failed\n");
        return -1;
    }
    *out_ios_port = listener_port(gIOSurfaceListener);

    void *handle = dlopen(
        "/System/Library/PrivateFrameworks/MTLSimImplementation.framework/MTLSimImplementation",
        RTLD_NOW
    );
    int (*init_fn)(xpc_connection_t, uint64_t, uint64_t) =
        handle ? dlsym(handle, "init_with_xpc_connection") : NULL;
    id<MTLDevice> device = MTLCreateSystemDefaultDevice();
    if (!init_fn || !device) {
        fprintf(
            stderr,
            "waterui-simhost: MTLSimImplementation unavailable (%s)\n",
            dlerror() ?: "no Metal device"
        );
        return -2;
    }
    gMetalListener = xpc_connection_create(NULL, dispatch_get_main_queue());
    xpc_connection_enable_sim2host_4sim(gMetalListener);
    init_fn(gMetalListener, device.registryID, 0);
    *out_mtl_port = listener_port(gMetalListener);

    return (*out_ios_port && *out_mtl_port) ? 0 : -3;
}

static int register_service(
    mach_port_t simns_ctl,
    const char *name,
    mach_port_t port,
    uint32_t flags
) {
    waterui_simns_register_msg_t msg = {0};
    msg.h.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0) | MACH_MSGH_BITS_COMPLEX;
    msg.h.msgh_size = sizeof(msg);
    msg.h.msgh_remote_port = simns_ctl;
    msg.h.msgh_id = WATERUI_SIMNS_MSG_REGISTER;
    msg.b.msgh_descriptor_count = 1;
    msg.port.name = port;
    msg.port.disposition = MACH_MSG_TYPE_COPY_SEND;
    msg.port.type = MACH_MSG_PORT_DESCRIPTOR;
    msg.flags = flags;
    strlcpy(msg.name, name, sizeof(msg.name));
    kern_return_t kr = mach_msg(&msg.h, MACH_SEND_MSG | MACH_SEND_TIMEOUT, sizeof(msg), 0, 0, 5000, 0);
    if (kr != KERN_SUCCESS) {
        fprintf(stderr, "waterui-simhost: register %s failed (kr=%#x)\n", name, kr);
        return -1;
    }
    return 0;
}

static void cleanup_children(void) {
    if (gAppPid > 0) {
        kill(gAppPid, SIGTERM);
        for (int i = 0; i < 10 && waitpid(gAppPid, NULL, WNOHANG) == 0; i++) {
            usleep(20000);
        }
        if (kill(gAppPid, 0) == 0) {
            kill(gAppPid, SIGKILL);
            waitpid(gAppPid, NULL, WNOHANG);
        }
        gAppPid = 0;
    }
    if (gSimnsPid > 0) {
        kill(gSimnsPid, SIGTERM);
        for (int i = 0; i < 20 && waitpid(gSimnsPid, NULL, WNOHANG) == 0; i++) {
            usleep(25000);
        }
        if (kill(gSimnsPid, 0) == 0) {
            kill(gSimnsPid, SIGKILL);
            waitpid(gSimnsPid, NULL, WNOHANG);
        }
        gSimnsPid = 0;
    }
}

static char **build_sim_env(
    const char *runtime_root,
    const char *container_dir,
    const char *simfb_path,
    const char *interpose_path
) {
    int count = 0;
    while (environ[count]) {
        count++;
    }
    char **out = calloc((size_t)count + 24, sizeof(char *));
    int idx = 0;
    for (int i = 0; i < count; i++) {
        if (strncmp(environ[i], "WATERUI_SIMHOST_", 16) == 0 ||
            strncmp(environ[i], "DYLD_", 5) == 0 ||
            strncmp(environ[i], "HOME=", 5) == 0 ||
            strncmp(environ[i], "CFFIXED_USER_HOME=", 18) == 0 ||
            strncmp(environ[i], "TMPDIR=", 7) == 0 ||
            strncmp(environ[i], "IPHONE_SHARED_RESOURCES_DIRECTORY=", 34) == 0 ||
            strncmp(environ[i], "SIMULATOR_SHARED_RESOURCES_DIRECTORY=", 37) == 0 ||
            strncmp(environ[i], "SIMULATOR_ROOT=", 15) == 0 ||
            strncmp(environ[i], "IPHONE_SIMULATOR_ROOT=", 22) == 0 ||
            strncmp(environ[i], "SIMULATOR_FRAMEBUFFER_FRAMEWORK=", 32) == 0) {
            continue;
        }
        out[idx++] = strdup(environ[i]);
    }
    asprintf(&out[idx++], "HOME=%s", container_dir);
    asprintf(&out[idx++], "CFFIXED_USER_HOME=%s", container_dir);
    asprintf(&out[idx++], "TMPDIR=%s/tmp/", container_dir);
    asprintf(&out[idx++], "IPHONE_SHARED_RESOURCES_DIRECTORY=%s", container_dir);
    asprintf(&out[idx++], "SIMULATOR_SHARED_RESOURCES_DIRECTORY=%s", container_dir);
    asprintf(&out[idx++], "DYLD_ROOT_PATH=%s", runtime_root);
    asprintf(&out[idx++], "SIMULATOR_ROOT=%s", runtime_root);
    asprintf(&out[idx++], "IPHONE_SIMULATOR_ROOT=%s", runtime_root);
    asprintf(&out[idx++], "DYLD_SHARED_CACHE_DIR=%s/System/Library/Caches/com.apple.dyld", runtime_root);
    asprintf(&out[idx++], "DYLD_FALLBACK_FRAMEWORK_PATH=%s/System/Library/Frameworks", runtime_root);
    asprintf(&out[idx++], "DYLD_FALLBACK_LIBRARY_PATH=%s/usr/lib", runtime_root);
    if (simfb_path) {
        asprintf(&out[idx++], "SIMULATOR_FRAMEBUFFER_FRAMEWORK=%s", simfb_path);
    }
    if (interpose_path) {
        asprintf(&out[idx++], "DYLD_INSERT_LIBRARIES=%s", interpose_path);
    }
    out[idx] = NULL;
    return out;
}

static void free_env(char **env) {
    if (!env) {
        return;
    }
    for (int i = 0; env[i]; i++) {
        free(env[i]);
    }
    free(env);
}

int main(int argc, char **argv) {
    @autoreleasepool {
        if (argc < 5) {
            fprintf(
                stderr,
                "usage: waterui-simhost <simns-path> <simfb-path> <interpose-path> <app-binary> [args...]\n"
            );
            return 2;
        }
        const char *simns_path = argv[1];
        const char *simfb_path = argv[2];
        const char *interpose_path = argv[3];
        const char *app_path = argv[4];
        const char *runtime_root = getenv("WATERUI_SIMHOST_RUNTIME_ROOT");
        if (!runtime_root || !runtime_root[0]) {
            fprintf(stderr, "waterui-simhost: WATERUI_SIMHOST_RUNTIME_ROOT is not set\n");
            return 2;
        }
        const char *container_dir = getenv("WATERUI_SIMHOST_CONTAINER");
        if (!container_dir || !container_dir[0]) {
            fprintf(stderr, "waterui-simhost: WATERUI_SIMHOST_CONTAINER is not set\n");
            return 2;
        }

        // Start host-side GPU services before launching any simulator processes.
        mach_port_t ios_port = MACH_PORT_NULL;
        mach_port_t mtl_port = MACH_PORT_NULL;
        if (start_gpu_services(&ios_port, &mtl_port) != 0) {
            return 1;
        }

        // Launch waterui-simns --serve and receive its control port.
        mach_port_t me = mach_task_self();
        mach_port_t hello_port = MACH_PORT_NULL;
        if (mach_port_allocate(me, MACH_PORT_RIGHT_RECEIVE, &hello_port) != KERN_SUCCESS) {
            fprintf(stderr, "waterui-simhost: failed to allocate hello port\n");
            return 1;
        }
        mach_port_insert_right(me, hello_port, hello_port, MACH_MSG_TYPE_MAKE_SEND);

        char **simns_env = build_sim_env(runtime_root, container_dir, NULL, NULL);
        char *simns_argv[] = { (char *)simns_path, "--serve", NULL };
        mach_port_t reg_ports[1] = { hello_port };
        mach_ports_register(me, reg_ports, 1);
        int sr = posix_spawn(&gSimnsPid, simns_path, NULL, NULL, simns_argv, simns_env);
        mach_ports_register(me, NULL, 0);
        free_env(simns_env);
        if (sr != 0) {
            fprintf(stderr, "waterui-simhost: failed to spawn simns (%s): %s\n", simns_path, strerror(sr));
            return 1;
        }

        union {
            waterui_simns_port_msg_t m;
            char buf[512];
        } hello = {0};
        kern_return_t kr = mach_msg(
            &hello.m.h,
            MACH_RCV_MSG | MACH_RCV_TIMEOUT,
            0,
            sizeof(hello),
            hello_port,
            10000,
            0
        );
        mach_port_deallocate(me, hello_port);
        mach_port_mod_refs(me, hello_port, MACH_PORT_RIGHT_RECEIVE, -1);
        if (kr != KERN_SUCCESS || hello.m.h.msgh_id != WATERUI_SIMNS_MSG_HELLO) {
            fprintf(stderr, "waterui-simhost: simns handshake failed (kr=%#x)\n", kr);
            cleanup_children();
            return 1;
        }
        mach_port_t simns_ctl = hello.m.port.name;

        if (register_service(simns_ctl, "com.apple.IOSurface.Remote", ios_port, WATERUI_SIMNS_F_HOST_SIDE) != 0 ||
            register_service(simns_ctl, "com.apple.metal.simulator", mtl_port, WATERUI_SIMNS_F_HOST_SIDE) != 0) {
            cleanup_children();
            return 1;
        }

        mach_msg_header_t ready = {
            .msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0),
            .msgh_size = sizeof(ready),
            .msgh_remote_port = simns_ctl,
            .msgh_id = WATERUI_SIMNS_MSG_READY,
        };
        mach_msg(&ready, MACH_SEND_MSG | MACH_SEND_TIMEOUT, sizeof(ready), 0, 0, 5000, 0);

        // Request an isolated per-app bootstrap port from waterui-simns.
        mach_port_t reply_port = MACH_PORT_NULL;
        mach_port_allocate(me, MACH_PORT_RIGHT_RECEIVE, &reply_port);
        mach_msg_header_t req_ns = {
            .msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, MACH_MSG_TYPE_MAKE_SEND_ONCE),
            .msgh_size = sizeof(req_ns),
            .msgh_remote_port = simns_ctl,
            .msgh_local_port = reply_port,
            .msgh_id = WATERUI_SIMNS_MSG_NEW_NS,
        };
        mach_msg(&req_ns, MACH_SEND_MSG | MACH_SEND_TIMEOUT, sizeof(req_ns), 0, 0, 5000, 0);

        union {
            waterui_simns_port_msg_t m;
            char buf[512];
        } ns_rep = {0};
        kr = mach_msg(
            &ns_rep.m.h,
            MACH_RCV_MSG | MACH_RCV_TIMEOUT,
            0,
            sizeof(ns_rep),
            reply_port,
            10000,
            0
        );
        mach_port_mod_refs(me, reply_port, MACH_PORT_RIGHT_RECEIVE, -1);
        if (kr != KERN_SUCCESS || !ns_rep.m.port.name) {
            fprintf(stderr, "waterui-simhost: failed to acquire app bootstrap port (kr=%#x)\n", kr);
            cleanup_children();
            return 1;
        }
        mach_port_t app_bootstrap = ns_rep.m.port.name;

        // Spawn the iOS Simulator preview binary with its isolated bootstrap port.
        char **app_env = build_sim_env(runtime_root, container_dir, simfb_path, interpose_path);
        int app_argc = argc - 4;
        char **app_argv = calloc((size_t)app_argc + 1, sizeof(char *));
        for (int i = 0; i < app_argc; i++) {
            app_argv[i] = argv[4 + i];
        }

        posix_spawnattr_t attr;
        posix_spawnattr_init(&attr);
        posix_spawnattr_setspecialport_np(&attr, app_bootstrap, TASK_BOOTSTRAP_PORT);
        int ar = posix_spawn(&gAppPid, app_path, NULL, &attr, app_argv, app_env);
        posix_spawnattr_destroy(&attr);
        free(app_argv);
        free_env(app_env);
        mach_port_deallocate(me, app_bootstrap);
        if (ar != 0) {
            fprintf(stderr, "waterui-simhost: failed to spawn preview binary (%s): %s\n", app_path, strerror(ar));
            cleanup_children();
            return 1;
        }

        // Ignore SIGTERM/SIGINT synchronously and handle via GCD so children are reaped cleanly.
        signal(SIGTERM, SIG_IGN);
        signal(SIGINT, SIG_IGN);

        gSigTermSource = dispatch_source_create(DISPATCH_SOURCE_TYPE_SIGNAL, SIGTERM, 0, dispatch_get_main_queue());
        dispatch_source_set_event_handler(gSigTermSource, ^{
            cleanup_children();
            _exit(128 + SIGTERM);
        });
        dispatch_resume(gSigTermSource);

        gSigIntSource = dispatch_source_create(DISPATCH_SOURCE_TYPE_SIGNAL, SIGINT, 0, dispatch_get_main_queue());
        dispatch_source_set_event_handler(gSigIntSource, ^{
            cleanup_children();
            _exit(128 + SIGINT);
        });
        dispatch_resume(gSigIntSource);

        // Terminate cleanly if the parent CLI exits unexpectedly.
        pid_t parent = getppid();
        gParentExitSource = dispatch_source_create(
            DISPATCH_SOURCE_TYPE_PROC,
            (uintptr_t)parent,
            DISPATCH_PROC_EXIT,
            dispatch_get_main_queue()
        );
        dispatch_source_set_event_handler(gParentExitSource, ^{
            cleanup_children();
            _exit(1);
        });
        dispatch_resume(gParentExitSource);

        // Wait for the preview binary to finish while dispatch_main serves Metal + IOSurface XPC.
        pid_t watched_app = gAppPid;
        gAppExitSource = dispatch_source_create(
            DISPATCH_SOURCE_TYPE_PROC,
            (uintptr_t)watched_app,
            DISPATCH_PROC_EXIT,
            dispatch_get_main_queue()
        );
        dispatch_source_set_event_handler(gAppExitSource, ^{
            int status = 0;
            waitpid(watched_app, &status, 0);
            gAppPid = 0;
            cleanup_children();
            if (WIFEXITED(status)) {
                _exit(WEXITSTATUS(status));
            }
            if (WIFSIGNALED(status)) {
                _exit(128 + WTERMSIG(status));
            }
            _exit(1);
        });
        dispatch_resume(gAppExitSource);

        dispatch_main();
    }
}
