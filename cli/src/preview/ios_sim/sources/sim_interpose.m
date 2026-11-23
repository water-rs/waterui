#import <Foundation/Foundation.h>
#include <dispatch/dispatch.h>
#include <mach/mach.h>
#include <signal.h>
#include <spawn.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>
#include <xpc/xpc.h>

#define EXPORT __attribute__((visibility("default")))

extern Boolean BKSDisplayServicesStart(void);
extern id FBSWorkspaceInitialize(id options);
extern xpc_connection_t waterui_orig_xpc_connection_create(const char *name, dispatch_queue_t targetq)
    __asm("_xpc_connection_create");
extern xpc_connection_t waterui_orig_xpc_connection_create_from_endpoint(xpc_endpoint_t endpoint)
    __asm("_xpc_connection_create_from_endpoint");
extern xpc_endpoint_t xpc_endpoint_create(xpc_connection_t listener);
extern mach_port_t xpc_endpoint_copy_listener_port_4sim(xpc_endpoint_t endpoint);
extern xpc_endpoint_t xpc_endpoint_create_mach_port_4sim(mach_port_t port);
extern void waterui_orig_xpc_main(void (*handler)(xpc_connection_t)) __asm("_xpc_main");
extern char **environ;

typedef Boolean (*waterui_bks_handler_t)(void);
typedef void (*waterui_fbs_handler_t)(id workspace);

static waterui_bks_handler_t gBksHandler;
static waterui_fbs_handler_t gFbsHandler;

EXPORT void waterui_sim_set_bks_handler(waterui_bks_handler_t handler) {
    gBksHandler = handler;
}

EXPORT void waterui_sim_set_fbs_handler(waterui_fbs_handler_t handler) {
    gFbsHandler = handler;
}

static Boolean hooked_BKSDisplayServicesStart(void) {
    if (gBksHandler) {
        return gBksHandler();
    }
    return BKSDisplayServicesStart();
}

static id hooked_FBSWorkspaceInitialize(id options) {
    id ws = FBSWorkspaceInitialize(options);
    if (gFbsHandler && ws) {
        gFbsHandler(ws);
    }
    return ws;
}

// ---- On-demand XPC service spawning (com.apple.MTLCompilerService) ----

enum {
    WATERUI_XPC_CHECKIN_MSG_ID = 0x57490101u,
};

typedef struct {
    mach_msg_header_t h;
    mach_msg_body_t b;
    mach_msg_port_descriptor_t port;
} waterui_xpc_checkin_msg_t;

typedef struct {
    const char *name;
    const char *rel_path;
    mach_port_t port;
    pid_t pid;
} waterui_xpc_service_t;

static waterui_xpc_service_t gMtlCompiler = {
    .name = "com.apple.MTLCompilerService",
    .rel_path = "/System/Library/Frameworks/Metal.framework/XPCServices/MTLCompilerService.xpc/MTLCompilerService",
    .port = MACH_PORT_NULL,
    .pid = 0,
};

static dispatch_queue_t xpc_service_queue(void) {
    static dispatch_queue_t q;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        q = dispatch_queue_create("dev.waterui.sim.xpcservices", DISPATCH_QUEUE_SERIAL);
    });
    return q;
}

static pid_t spawn_mtl_compiler(mach_port_t *out_listener) {
    *out_listener = MACH_PORT_NULL;
    const char *root = getenv("DYLD_ROOT_PATH");
    if (!root || !root[0]) {
        return -1;
    }
    char bin_path[2048];
    snprintf(bin_path, sizeof(bin_path), "%s%s", root, gMtlCompiler.rel_path);
    if (access(bin_path, X_OK) != 0) {
        return -1;
    }

    mach_port_t me = mach_task_self();
    mach_port_t reply_port = MACH_PORT_NULL;
    if (mach_port_allocate(me, MACH_PORT_RIGHT_RECEIVE, &reply_port) != KERN_SUCCESS) {
        return -1;
    }
    mach_port_insert_right(me, reply_port, reply_port, MACH_MSG_TYPE_MAKE_SEND);

    int envc = 0;
    while (environ[envc]) {
        envc++;
    }
    char **child_env = calloc((size_t)envc + 4, sizeof(char *));
    int k = 0;
    for (int i = 0; i < envc; i++) {
        if (strncmp(environ[i], "WATERUI_SIM_XPC_ROLE=", 21) == 0 ||
            strncmp(environ[i], "WATERUI_PREVIEW_RUN_CONFIG=", 27) == 0) {
            continue;
        }
        child_env[k++] = environ[i];
    }
    child_env[k++] = "WATERUI_SIM_XPC_ROLE=1";
    child_env[k++] = "XPC_FLAGS=0x0";

    mach_port_t reg[1] = { reply_port };
    mach_ports_register(me, reg, 1);

    char *argv[] = { bin_path, NULL };
    pid_t pid = 0;
    int sr = posix_spawn(&pid, bin_path, NULL, NULL, argv, child_env);
    mach_ports_register(me, NULL, 0);
    free(child_env);

    if (sr != 0) {
        mach_port_deallocate(me, reply_port);
        mach_port_mod_refs(me, reply_port, MACH_PORT_RIGHT_RECEIVE, -1);
        return -1;
    }

    union {
        waterui_xpc_checkin_msg_t msg;
        char buf[512];
    } rep = {0};
    kern_return_t kr = mach_msg(
        &rep.msg.h,
        MACH_RCV_MSG | MACH_RCV_TIMEOUT,
        0,
        sizeof(rep),
        reply_port,
        15000,
        0
    );
    mach_port_deallocate(me, reply_port);
    mach_port_mod_refs(me, reply_port, MACH_PORT_RIGHT_RECEIVE, -1);

    if (kr != KERN_SUCCESS || rep.msg.h.msgh_id != WATERUI_XPC_CHECKIN_MSG_ID ||
        !rep.msg.port.name) {
        kill(pid, SIGKILL);
        waitpid(pid, NULL, 0);
        return -1;
    }

    *out_listener = rep.msg.port.name;
    return pid;
}

static xpc_connection_t hooked_xpc_connection_create(const char *name, dispatch_queue_t q) {
    if (name && strcmp(name, gMtlCompiler.name) == 0 && !getenv("WATERUI_SIM_XPC_ROLE")) {
        __block mach_port_t port = MACH_PORT_NULL;
        dispatch_sync(xpc_service_queue(), ^{
            mach_port_type_t t = 0;
            if (gMtlCompiler.port &&
                (mach_port_type(mach_task_self(), gMtlCompiler.port, &t) != KERN_SUCCESS ||
                 (t & MACH_PORT_TYPE_DEAD_NAME))) {
                mach_port_deallocate(mach_task_self(), gMtlCompiler.port);
                gMtlCompiler.port = MACH_PORT_NULL;
            }
            if (!gMtlCompiler.port) {
                mach_port_t p = MACH_PORT_NULL;
                pid_t pid = spawn_mtl_compiler(&p);
                if (pid > 0 && p) {
                    gMtlCompiler.port = p;
                    gMtlCompiler.pid = pid;
                    dispatch_source_t ds = dispatch_source_create(
                        DISPATCH_SOURCE_TYPE_PROC,
                        (uintptr_t)pid,
                        DISPATCH_PROC_EXIT,
                        xpc_service_queue()
                    );
                    dispatch_source_set_event_handler(ds, ^{
                        int st = 0;
                        waitpid(pid, &st, WNOHANG);
                        if (gMtlCompiler.pid == pid && gMtlCompiler.port) {
                            mach_port_deallocate(mach_task_self(), gMtlCompiler.port);
                            gMtlCompiler.port = MACH_PORT_NULL;
                        }
                        dispatch_source_cancel(ds);
                    });
                    dispatch_resume(ds);
                }
            }
            if (gMtlCompiler.port) {
                port = gMtlCompiler.port;
                mach_port_mod_refs(mach_task_self(), port, MACH_PORT_RIGHT_SEND, 1);
            }
        });
        if (port) {
            xpc_endpoint_t ep = xpc_endpoint_create_mach_port_4sim(port);
            if (ep) {
                xpc_connection_t conn = waterui_orig_xpc_connection_create_from_endpoint(ep);
                if (q) {
                    xpc_connection_set_target_queue(conn, q);
                }
                return conn;
            }
        }
    }
    return waterui_orig_xpc_connection_create(name, q);
}

static dispatch_source_t gParentWatch;

static void hooked_xpc_main(void (*handler)(xpc_connection_t)) {
    if (!getenv("WATERUI_SIM_XPC_ROLE")) {
        waterui_orig_xpc_main(handler);
        return;
    }

    // Exit automatically if the preview process that spawned us terminates.
    pid_t parent = getppid();
    gParentWatch = dispatch_source_create(
        DISPATCH_SOURCE_TYPE_PROC,
        (uintptr_t)parent,
        DISPATCH_PROC_EXIT,
        dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0)
    );
    dispatch_source_set_event_handler(gParentWatch, ^{
        _exit(0);
    });
    dispatch_resume(gParentWatch);

    xpc_connection_t listener = waterui_orig_xpc_connection_create(NULL, NULL);
    // Serialize peer compilation requests: concurrent requests on one MTLCompilerService
    // process race inside the simulator compiler's LLVM passes.
    dispatch_queue_t peers =
        dispatch_queue_create("dev.waterui.sim.mtlcompiler.peers", DISPATCH_QUEUE_SERIAL);
    xpc_connection_set_event_handler(listener, ^(xpc_object_t peer) {
        if (xpc_get_type(peer) != XPC_TYPE_CONNECTION) {
            return;
        }
        xpc_connection_set_target_queue((xpc_connection_t)peer, peers);
        handler((xpc_connection_t)peer);
    });
    xpc_connection_activate(listener);

    mach_port_t me = mach_task_self();
    mach_port_array_t ports = NULL;
    mach_msg_type_number_t count = 0;
    if (mach_ports_lookup(me, &ports, &count) == KERN_SUCCESS && count > 0 && ports[0]) {
        xpc_endpoint_t ep = xpc_endpoint_create(listener);
        mach_port_t lport = ep ? xpc_endpoint_copy_listener_port_4sim(ep) : MACH_PORT_NULL;
        if (lport) {
            waterui_xpc_checkin_msg_t msg = {0};
            msg.h.msgh_bits =
                MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0) | MACH_MSGH_BITS_COMPLEX;
            msg.h.msgh_size = sizeof(msg);
            msg.h.msgh_remote_port = ports[0];
            msg.h.msgh_id = WATERUI_XPC_CHECKIN_MSG_ID;
            msg.b.msgh_descriptor_count = 1;
            msg.port.name = lport;
            msg.port.disposition = MACH_MSG_TYPE_COPY_SEND;
            msg.port.type = MACH_MSG_PORT_DESCRIPTOR;
            mach_msg(&msg.h, MACH_SEND_MSG | MACH_SEND_TIMEOUT, sizeof(msg), 0, 0, 5000, 0);
        }
    }
    dispatch_main();
}

__attribute__((used, section("__DATA,__interpose"))) static const struct {
    const void *replacement;
    const void *original;
} kInterposers[] = {
    { (const void *)hooked_BKSDisplayServicesStart, (const void *)BKSDisplayServicesStart },
    { (const void *)hooked_FBSWorkspaceInitialize, (const void *)FBSWorkspaceInitialize },
    { (const void *)hooked_xpc_connection_create, (const void *)waterui_orig_xpc_connection_create },
    { (const void *)hooked_xpc_main, (const void *)waterui_orig_xpc_main },
};
