#include "simns_proto.h"

#include <CoreFoundation/CoreFoundation.h>
#include <dirent.h>
#include <dispatch/dispatch.h>
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <spawn.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>
#include <xpc/xpc.h>

extern int xpc_receive_mach_msg(void *dmsg, void *a, void *b, void *c, xpc_object_t *out);
extern void *dispatch_mach_msg_create(
    mach_msg_header_t *msg,
    size_t size,
    void *destructor,
    mach_msg_header_t **ptr
);
extern int xpc_pipe_routine_reply(xpc_object_t reply);
extern void xpc_dictionary_set_mach_send(xpc_object_t dict, const char *key, mach_port_t port);
extern void xpc_dictionary_set_mach_recv(xpc_object_t dict, const char *key, mach_port_t port);
extern mach_port_t xpc_dictionary_copy_mach_send(xpc_object_t dict, const char *key);
extern int posix_spawnattr_setspecialport_np(posix_spawnattr_t *attr, mach_port_t port, int which);
extern char **environ;

static inline pid_t audit_token_pid(audit_token_t token) {
    return (pid_t)token.val[5];
}

typedef struct entry {
    char *name;
    mach_port_t send;
    mach_port_t recv;
    uint32_t flags;
    struct entry *next;
} entry_t;

typedef struct ns {
    mach_port_t port;
    entry_t *entries;
    struct ns *next;
} ns_t;

typedef struct daemon {
    char *label;
    char **argv;
    char **env;
    char **services;
    int nservices;
    pid_t pid;
    int gone;
    struct daemon *next;
} daemon_t;

static ns_t gGlobal;
static ns_t *gApps;
static pthread_mutex_t gLock = PTHREAD_MUTEX_INITIALIZER;
static pthread_mutex_t gDaemonLock = PTHREAD_MUTEX_INITIALIZER;
static mach_port_t gNS;
static mach_port_t gCtl;
static mach_port_t gSet;
static mach_port_t gReturned;
static mach_port_t gIdle;
static daemon_t *gDaemons;
static char **gChildEnv;
static char *gRoot;
static int gVerbose;
static volatile int gReady;
static volatile int gStopping;

static void log_msg(const char *fmt, ...) {
    if (!gVerbose) {
        return;
    }
    char buf[1024];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(buf, sizeof(buf), fmt, ap);
    va_end(ap);
    fprintf(stderr, "[waterui-simns] %s\n", buf);
}

static entry_t *find_entry(ns_t *n, const char *name) {
    for (entry_t *e = n->entries; e; e = e->next) {
        if (strcmp(e->name, name) == 0) {
            return e;
        }
    }
    return NULL;
}

static entry_t *get_entry(ns_t *n, const char *name) {
    entry_t *e = find_entry(n, name);
    if (e) {
        return e;
    }
    e = calloc(1, sizeof(*e));
    e->name = strdup(name);
    e->next = n->entries;
    n->entries = e;
    return e;
}

static ns_t *ns_for_port(mach_port_t port) {
    for (ns_t *n = gApps; n; n = n->next) {
        if (n->port == port) {
            return n;
        }
    }
    return &gGlobal;
}

static void register_port(ns_t *n, const char *name, mach_port_t send, uint32_t flags) {
    pthread_mutex_lock(&gLock);
    entry_t *e = get_entry(n, name);
    if (e->send) {
        mach_port_deallocate(mach_task_self(), e->send);
    }
    e->send = send;
    e->flags = flags;
    pthread_mutex_unlock(&gLock);
}

static daemon_t *daemon_for(const char *name) {
    for (daemon_t *d = gDaemons; d; d = d->next) {
        for (int i = 0; i < d->nservices; i++) {
            if (strcmp(d->services[i], name) == 0) {
                return d;
            }
        }
    }
    return NULL;
}

static mach_port_t checkin_port(ns_t *n, const char *name) {
    pthread_mutex_lock(&gLock);
    entry_t *e = get_entry(n, name);
    mach_port_t r = e->recv;
    if (!r) {
        mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_RECEIVE, &r);
        mach_port_insert_right(mach_task_self(), r, r, MACH_MSG_TYPE_MAKE_SEND);
        if (e->send) {
            mach_port_deallocate(mach_task_self(), e->send);
        }
        e->send = r;
    }
    e->recv = MACH_PORT_NULL;
    pthread_mutex_unlock(&gLock);

    if (n == &gGlobal && daemon_for(name) != NULL) {
        mach_port_move_member(mach_task_self(), r, MACH_PORT_NULL);
        mach_port_t prev = MACH_PORT_NULL;
        mach_port_request_notification(
            mach_task_self(),
            r,
            MACH_NOTIFY_PORT_DESTROYED,
            0,
            gReturned,
            MACH_MSG_TYPE_MAKE_SEND_ONCE,
            &prev
        );
        if (prev) {
            mach_port_deallocate(mach_task_self(), prev);
        }
    }
    return r;
}

static mach_port_t lookup_in(ns_t *n, const char *name, uint32_t *flags) {
    entry_t *e = find_entry(n, name);
    if (e && e->send) {
        mach_port_type_t t = 0;
        if (mach_port_type(mach_task_self(), e->send, &t) == KERN_SUCCESS &&
            (t & (MACH_PORT_TYPE_SEND | MACH_PORT_TYPE_RECEIVE)) &&
            !(t & MACH_PORT_TYPE_DEAD_NAME)) {
            if (flags) {
                *flags = e->flags;
            }
            return e->send;
        }
    }
    return MACH_PORT_NULL;
}

static mach_port_t lookup_port(ns_t *n, const char *name, uint32_t *flags) {
    pthread_mutex_lock(&gLock);
    mach_port_t p = (n != &gGlobal) ? lookup_in(n, name, flags) : MACH_PORT_NULL;
    if (!p) {
        p = lookup_in(&gGlobal, name, flags);
    }
    pthread_mutex_unlock(&gLock);
    return p;
}

static void free_entries(entry_t *e) {
    while (e) {
        entry_t *next = e->next;
        if (e->send) {
            mach_port_deallocate(mach_task_self(), e->send);
        }
        if (e->recv) {
            mach_port_mod_refs(mach_task_self(), e->recv, MACH_PORT_RIGHT_RECEIVE, -1);
        }
        free(e->name);
        free(e);
        e = next;
    }
}

static int daemon_allowed(const char *label) {
    const char *list = getenv("WATERUI_SIMNS_DAEMONS");
    if (!list) {
        list = "com.apple.notifyd";
    }
    if (strcmp(list, "none") == 0) {
        return 0;
    }
    size_t len = strlen(label);
    for (const char *p = list; (p = strstr(p, label)) != NULL; p += len) {
        if ((p == list || p[-1] == ',') && (p[len] == '\0' || p[len] == ',')) {
            return 1;
        }
    }
    return 0;
}

static char *dup_cfstring(CFTypeRef value) {
    if (!value || CFGetTypeID(value) != CFStringGetTypeID()) {
        return NULL;
    }
    char buf[2048];
    if (!CFStringGetCString((CFStringRef)value, buf, sizeof(buf), kCFStringEncodingUTF8)) {
        return NULL;
    }
    return strdup(buf);
}

static void load_daemon_plist(const char *path) {
    CFURLRef url =
        CFURLCreateFromFileSystemRepresentation(NULL, (const UInt8 *)path, (CFIndex)strlen(path), false);
    if (!url) {
        return;
    }
    CFReadStreamRef stream = CFReadStreamCreateWithFile(NULL, url);
    CFRelease(url);
    if (!stream || !CFReadStreamOpen(stream)) {
        if (stream) {
            CFRelease(stream);
        }
        return;
    }
    CFPropertyListRef plist = CFPropertyListCreateWithStream(NULL, stream, 0, 0, NULL, NULL);
    CFReadStreamClose(stream);
    CFRelease(stream);
    if (!plist || CFGetTypeID(plist) != CFDictionaryGetTypeID()) {
        if (plist) {
            CFRelease(plist);
        }
        return;
    }

    CFDictionaryRef dict = (CFDictionaryRef)plist;
    char *label = dup_cfstring(CFDictionaryGetValue(dict, CFSTR("Label")));
    CFDictionaryRef mach_services = CFDictionaryGetValue(dict, CFSTR("MachServices"));
    CFArrayRef args = CFDictionaryGetValue(dict, CFSTR("ProgramArguments"));
    char *prog = dup_cfstring(CFDictionaryGetValue(dict, CFSTR("Program")));

    if (label && daemon_allowed(label) && mach_services &&
        CFGetTypeID(mach_services) == CFDictionaryGetTypeID() && (args || prog)) {
        daemon_t *d = calloc(1, sizeof(*d));
        d->label = label;
        CFIndex na = args ? CFArrayGetCount(args) : 0;
        d->argv = calloc((size_t)na + 2, sizeof(char *));
        for (CFIndex i = 0; i < na; i++) {
            d->argv[i] = dup_cfstring(CFArrayGetValueAtIndex(args, i));
        }
        if (prog) {
            free(d->argv[0]);
            d->argv[0] = prog;
            prog = NULL;
        }
        if (d->argv[0] && d->argv[0][0] == '/') {
            char full[2048];
            snprintf(full, sizeof(full), "%s%s", gRoot, d->argv[0]);
            if (access(full, X_OK) == 0) {
                free(d->argv[0]);
                d->argv[0] = strdup(full);
            }
        }
        CFIndex ns = CFDictionaryGetCount(mach_services);
        const void **keys = calloc((size_t)ns, sizeof(void *));
        CFDictionaryGetKeysAndValues(mach_services, keys, NULL);
        d->services = calloc((size_t)ns, sizeof(char *));
        for (CFIndex i = 0; i < ns; i++) {
            char *svc = dup_cfstring(keys[i]);
            if (svc) {
                d->services[d->nservices++] = svc;
            }
        }
        free(keys);

        CFDictionaryRef ev = CFDictionaryGetValue(dict, CFSTR("EnvironmentVariables"));
        CFIndex ne = (ev && CFGetTypeID(ev) == CFDictionaryGetTypeID()) ? CFDictionaryGetCount(ev) : 0;
        d->env = calloc((size_t)ne + 1, sizeof(char *));
        if (ne > 0) {
            const void **ek = calloc((size_t)ne, sizeof(void *));
            const void **evv = calloc((size_t)ne, sizeof(void *));
            CFDictionaryGetKeysAndValues(ev, ek, evv);
            int idx = 0;
            for (CFIndex i = 0; i < ne; i++) {
                char *k = dup_cfstring(ek[i]);
                char *v = dup_cfstring(evv[i]);
                if (k && v) {
                    asprintf(&d->env[idx++], "%s=%s", k, v);
                }
                free(k);
                free(v);
            }
            free(ek);
            free(evv);
        }
        d->next = gDaemons;
        gDaemons = d;
    } else {
        free(label);
    }
    free(prog);
    CFRelease(plist);
}

static void load_daemons(void) {
    const char *dirs[2] = { "/System/Library/LaunchDaemons", "/System/Library/LaunchAgents" };
    for (int i = 0; i < 2; i++) {
        char dir[1024];
        snprintf(dir, sizeof(dir), "%s%s", gRoot, dirs[i]);
        DIR *dp = opendir(dir);
        if (!dp) {
            continue;
        }
        struct dirent *de;
        while ((de = readdir(dp)) != NULL) {
            size_t len = strlen(de->d_name);
            if (len > 6 && strcmp(de->d_name + len - 6, ".plist") == 0) {
                char file[2048];
                snprintf(file, sizeof(file), "%s/%s", dir, de->d_name);
                load_daemon_plist(file);
            }
        }
        closedir(dp);
    }
}

static int port_alive(mach_port_t p) {
    mach_port_type_t t = 0;
    return p && mach_port_type(mach_task_self(), p, &t) == KERN_SUCCESS &&
        !(t & MACH_PORT_TYPE_DEAD_NAME) &&
        (t & (MACH_PORT_TYPE_SEND | MACH_PORT_TYPE_RECEIVE));
}

static void ensure_daemon(const char *name) {
    if (gStopping) {
        return;
    }
    pthread_mutex_lock(&gDaemonLock);
    for (daemon_t *d = daemon_for(name); d && !gStopping; d = NULL) {
        if (d->gone && d->pid > 0) {
            for (int t = 0; t < 50 && waitpid(d->pid, NULL, WNOHANG) == 0; t++) {
                usleep(10000);
            }
        }
        if (d->gone) {
            d->pid = 0;
            d->gone = 0;
        }
        if (d->pid > 0) {
            int st = 0;
            if (waitpid(d->pid, &st, WNOHANG) == d->pid) {
                d->pid = 0;
            }
        }
        if (d->pid > 0 && kill(d->pid, 0) == 0) {
            break;
        }

        pthread_mutex_lock(&gLock);
        for (int i = 0; i < d->nservices; i++) {
            entry_t *e = get_entry(&gGlobal, d->services[i]);
            if (e->recv || port_alive(e->send)) {
                mach_port_type_t t = 0;
                if (e->recv ||
                    (mach_port_type(mach_task_self(), e->send, &t) == KERN_SUCCESS &&
                     (t & MACH_PORT_TYPE_RECEIVE))) {
                    continue;
                }
            }
            if (e->send) {
                mach_port_deallocate(mach_task_self(), e->send);
            }
            mach_port_t r = MACH_PORT_NULL;
            mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_RECEIVE, &r);
            mach_port_insert_right(mach_task_self(), r, r, MACH_MSG_TYPE_MAKE_SEND);
            e->recv = r;
            e->send = r;
        }
        pthread_mutex_unlock(&gLock);

        int nb = 0;
        while (gChildEnv[nb]) {
            nb++;
        }
        int ne = 0;
        while (d->env[ne]) {
            ne++;
        }
        char **env = calloc((size_t)(nb + ne + 2), sizeof(char *));
        int k = 0;
        for (int i = 0; i < nb; i++) {
            env[k++] = gChildEnv[i];
        }
        for (int i = 0; i < ne; i++) {
            env[k++] = d->env[i];
        }
        char svc[300];
        snprintf(svc, sizeof(svc), "XPC_SERVICE_NAME=%s", d->label);
        env[k++] = svc;

        posix_spawnattr_t attr;
        posix_spawnattr_init(&attr);
        posix_spawnattr_setspecialport_np(&attr, gNS, TASK_BOOTSTRAP_PORT);
        posix_spawnattr_setflags(&attr, POSIX_SPAWN_CLOEXEC_DEFAULT);
        posix_spawn_file_actions_t fa;
        posix_spawn_file_actions_init(&fa);
        for (int i = 0; i < 3; i++) {
            posix_spawn_file_actions_addinherit_np(&fa, i);
        }
        int r = posix_spawn(&d->pid, d->argv[0], &fa, &attr, d->argv, env);
        posix_spawn_file_actions_destroy(&fa);
        posix_spawnattr_destroy(&attr);
        free(env);
        if (r != 0) {
            log_msg("spawn daemon %s failed: %s", d->label, strerror(r));
            d->pid = 0;
        } else {
            log_msg("launched %s (pid %d) for %s", d->label, d->pid, name);
        }
    }
    pthread_mutex_unlock(&gDaemonLock);
}

static void port_returned(mach_port_destroyed_notification_t *m) {
    mach_port_t p = m->not_port.name;
    const char *name = NULL;
    pthread_mutex_lock(&gLock);
    for (entry_t *e = gGlobal.entries; e; e = e->next) {
        if (e->send == p) {
            e->recv = p;
            name = e->name;
            break;
        }
    }
    pthread_mutex_unlock(&gLock);
    daemon_t *d = (name && !gStopping) ? daemon_for(name) : NULL;
    if (!d) {
        mach_port_mod_refs(mach_task_self(), p, MACH_PORT_RIGHT_RECEIVE, -1);
        return;
    }
    pthread_mutex_lock(&gDaemonLock);
    d->gone = 1;
    pthread_mutex_unlock(&gDaemonLock);
    mach_port_move_member(mach_task_self(), p, gIdle);
}

static void *idle_watch(void *arg) {
    (void)arg;
    mach_port_t me = mach_task_self();
    for (;;) {
        mach_msg_header_t h;
        kern_return_t kr = mach_msg(&h, MACH_RCV_MSG | MACH_RCV_LARGE, 0, sizeof(h), gIdle, 0, 0);
        if (kr != MACH_RCV_TOO_LARGE) {
            if (kr != KERN_SUCCESS) {
                usleep(100000);
            }
            continue;
        }
        mach_port_name_array_t members = NULL;
        mach_msg_type_number_t count = 0;
        if (mach_port_get_set_status(me, gIdle, &members, &count) != KERN_SUCCESS) {
            continue;
        }
        for (mach_msg_type_number_t i = 0; i < count; i++) {
            mach_port_status_t st;
            mach_msg_type_number_t sc = MACH_PORT_RECEIVE_STATUS_COUNT;
            if (mach_port_get_attributes(
                    me,
                    members[i],
                    MACH_PORT_RECEIVE_STATUS,
                    (mach_port_info_t)&st,
                    &sc
                ) != KERN_SUCCESS ||
                !st.mps_msgcount) {
                continue;
            }
            mach_port_move_member(me, members[i], MACH_PORT_NULL);
            char name[256] = "";
            pthread_mutex_lock(&gLock);
            for (entry_t *e = gGlobal.entries; e; e = e->next) {
                if (e->recv == members[i]) {
                    strlcpy(name, e->name, sizeof(name));
                    break;
                }
            }
            pthread_mutex_unlock(&gLock);
            if (name[0]) {
                ensure_daemon(name);
            }
        }
        vm_deallocate(me, (vm_address_t)members, count * sizeof(*members));
    }
    return NULL;
}

static void stop_daemons(void) {
    gStopping = 1;
    int alive = 0;
    for (daemon_t *d = gDaemons; d; d = d->next) {
        if (d->pid > 0 && kill(d->pid, SIGTERM) == 0) {
            alive++;
        }
    }
    for (int t = 0; alive && t < 20; t++) {
        usleep(50000);
        alive = 0;
        for (daemon_t *d = gDaemons; d; d = d->next) {
            if (d->pid > 0) {
                if (waitpid(d->pid, NULL, WNOHANG) == d->pid || kill(d->pid, 0) != 0) {
                    d->pid = 0;
                } else {
                    alive++;
                }
            }
        }
    }
    for (daemon_t *d = gDaemons; d; d = d->next) {
        if (d->pid > 0) {
            kill(d->pid, SIGKILL);
            d->pid = 0;
        }
    }
}

static void on_term(int sig) {
    stop_daemons();
    _exit(128 + sig);
}

static void handle_pipe(mach_msg_header_t *h, pid_t pid) {
    uint32_t routine = (uint32_t)(h->msgh_id & 0xffffff);
    int wants_reply = (h->msgh_id & 0x40000000) != 0;

    if (routine == 406 && MACH_MSGH_BITS_REMOTE(h->msgh_bits) == MACH_MSG_TYPE_PORT_SEND_ONCE) {
        static mach_port_t polls[16];
        mach_port_t so = h->msgh_remote_port;
        h->msgh_remote_port = MACH_PORT_NULL;
        h->msgh_bits = MACH_MSGH_BITS_SET(
            0,
            MACH_MSGH_BITS_LOCAL(h->msgh_bits),
            MACH_MSGH_BITS_VOUCHER(h->msgh_bits),
            MACH_MSGH_BITS_OTHER(h->msgh_bits)
        );
        for (int i = 0; i < 16; i++) {
            mach_port_type_t t = 0;
            if (polls[i] &&
                (mach_port_type(mach_task_self(), polls[i], &t) != KERN_SUCCESS ||
                 (t & MACH_PORT_TYPE_DEAD_NAME))) {
                mach_port_deallocate(mach_task_self(), polls[i]);
                polls[i] = MACH_PORT_NULL;
            }
            if (!polls[i] && so) {
                polls[i] = so;
                so = MACH_PORT_NULL;
            }
        }
    }

    h->msgh_id &= 0xff000000u;
    mach_msg_header_t *copy = NULL;
    void *dm = dispatch_mach_msg_create(
        h,
        round_msg(h->msgh_size) + sizeof(mach_msg_audit_trailer_t),
        NULL,
        &copy
    );
    xpc_object_t msg = NULL;
    if (xpc_receive_mach_msg(dm, 0, 0, 0, &msg) != 0 || !msg) {
        dispatch_release(dm);
        return;
    }
    const char *name = xpc_dictionary_get_string(msg, "name");
    if (!name) {
        name = "";
    }
    pthread_mutex_lock(&gLock);
    ns_t *n = ns_for_port(h->msgh_local_port);
    pthread_mutex_unlock(&gLock);
    xpc_object_t reply = wants_reply ? xpc_dictionary_create_reply(msg) : NULL;

    switch (routine) {
        case 207:
        case 804: {
            ensure_daemon(name);
            uint32_t fl = 0;
            mach_port_t p = lookup_port(n, name, &fl);
            if (reply) {
                if (p) {
                    xpc_dictionary_set_mach_send(reply, "port", p);
                    if (routine == 804) {
                        xpc_dictionary_set_bool(reply, "non-launching", false);
                    }
                } else {
                    xpc_dictionary_set_int64(reply, "error", routine == 207 ? 1102 : 3);
                }
            }
            break;
        }
        case 204: {
            mach_port_t p = xpc_dictionary_copy_mach_send(msg, "port");
            if (p) {
                register_port(n, name, p, 0);
            }
            if (reply) {
                xpc_dictionary_set_int64(reply, "error", 0);
            }
            break;
        }
        case 206:
        case 805: {
            mach_port_t r = checkin_port(n, name);
            if (reply) {
                xpc_dictionary_set_mach_recv(reply, "port", r);
            }
            break;
        }
        case 403:
            if (reply) {
                xpc_object_t ev = xpc_array_create(NULL, 0);
                xpc_dictionary_set_value(reply, "events", ev);
                xpc_release(ev);
                xpc_dictionary_set_bool(reply, "initial-load-completed", true);
                xpc_dictionary_set_int64(reply, "error", 0);
            }
            break;
        case 406:
            break;
        case 712:
            if (reply) {
                xpc_dictionary_set_int64(reply, "error", 113);
            }
            break;
        default:
            log_msg("pid %d unhandled routine %u", pid, routine);
            if (reply) {
                xpc_dictionary_set_int64(reply, "error", 45);
            }
            break;
    }

    if (reply) {
        xpc_pipe_routine_reply(reply);
        xpc_release(reply);
    }
    xpc_release(msg);
    dispatch_release(dm);
}

static void handle_ctl(mach_msg_header_t *h) {
    switch (h->msgh_id) {
        case WATERUI_SIMNS_MSG_REGISTER: {
            waterui_simns_register_msg_t *m = (waterui_simns_register_msg_t *)h;
            m->name[WATERUI_SIMNS_NAME_MAX - 1] = '\0';
            register_port(&gGlobal, m->name, m->port.name, m->flags);
            break;
        }
        case WATERUI_SIMNS_MSG_READY:
            gReady = 1;
            break;
        case WATERUI_SIMNS_MSG_NEW_NS: {
            mach_port_t me = mach_task_self();
            mach_port_t p = MACH_PORT_NULL;
            mach_port_allocate(me, MACH_PORT_RIGHT_RECEIVE, &p);
            mach_port_move_member(me, p, gSet);
            ns_t *n = calloc(1, sizeof(*n));
            n->port = p;
            pthread_mutex_lock(&gLock);
            n->next = gApps;
            gApps = n;
            pthread_mutex_unlock(&gLock);

            waterui_simns_port_msg_t r = {0};
            r.h.msgh_bits =
                MACH_MSGH_BITS(MACH_MSG_TYPE_MOVE_SEND_ONCE, 0) | MACH_MSGH_BITS_COMPLEX;
            r.h.msgh_size = sizeof(r);
            r.h.msgh_remote_port = h->msgh_remote_port;
            r.h.msgh_id = WATERUI_SIMNS_MSG_NS_REPLY;
            r.b.msgh_descriptor_count = 1;
            r.port.name = p;
            r.port.disposition = MACH_MSG_TYPE_MAKE_SEND;
            r.port.type = MACH_MSG_PORT_DESCRIPTOR;
            mach_msg(&r.h, MACH_SEND_MSG | MACH_SEND_TIMEOUT, sizeof(r), 0, 0, 1000, 0);

            mach_port_t prev = MACH_PORT_NULL;
            mach_port_request_notification(
                me,
                p,
                MACH_NOTIFY_NO_SENDERS,
                1,
                p,
                MACH_MSG_TYPE_MAKE_SEND_ONCE,
                &prev
            );
            break;
        }
        default:
            mach_msg_destroy(h);
            break;
    }
}

static void destroy_ns(mach_port_t port) {
    pthread_mutex_lock(&gLock);
    ns_t **pp = &gApps;
    ns_t *n = NULL;
    while (*pp) {
        if ((*pp)->port == port) {
            n = *pp;
            *pp = n->next;
            break;
        }
        pp = &(*pp)->next;
    }
    pthread_mutex_unlock(&gLock);
    if (!n) {
        return;
    }
    free_entries(n->entries);
    mach_port_mod_refs(mach_task_self(), port, MACH_PORT_RIGHT_RECEIVE, -1);
    free(n);
}

static void *serve(void *arg) {
    (void)arg;
    size_t cap = 64 * 1024;
    mach_msg_header_t *h = malloc(cap);
    for (;;) {
        kern_return_t kr = mach_msg(
            h,
            MACH_RCV_MSG | MACH_RCV_LARGE |
                MACH_RCV_TRAILER_TYPE(MACH_MSG_TRAILER_FORMAT_0) |
                MACH_RCV_TRAILER_ELEMENTS(MACH_RCV_TRAILER_AUDIT),
            0,
            (mach_msg_size_t)cap,
            gSet,
            0,
            0
        );
        if (kr == MACH_RCV_TOO_LARGE) {
            cap = h->msgh_size + 1024;
            h = realloc(h, cap);
            continue;
        }
        if (kr != KERN_SUCCESS) {
            continue;
        }
        mach_msg_audit_trailer_t *t =
            (mach_msg_audit_trailer_t *)((char *)h + round_msg(h->msgh_size));
        pid_t pid = audit_token_pid(t->msgh_audit);
        if (h->msgh_local_port == gCtl) {
            handle_ctl(h);
        } else if (h->msgh_local_port == gReturned && h->msgh_id == MACH_NOTIFY_PORT_DESTROYED) {
            port_returned((mach_port_destroyed_notification_t *)h);
        } else if (h->msgh_id == MACH_NOTIFY_NO_SENDERS) {
            destroy_ns(h->msgh_local_port);
        } else if ((h->msgh_id & 0x40000000) || (h->msgh_id & 0x10000000)) {
            handle_pipe(h, pid);
        } else {
            mach_msg_destroy(h);
        }
    }
    return NULL;
}

int main(int argc, char **argv) {
    if (argc != 2 || strcmp(argv[1], "--serve") != 0) {
        fprintf(stderr, "usage: waterui-simns --serve\n");
        return 2;
    }
    gVerbose = getenv("WATERUI_SIM_VERBOSE") != NULL;

    mach_port_t me = mach_task_self();
    mach_port_allocate(me, MACH_PORT_RIGHT_RECEIVE, &gNS);
    mach_port_insert_right(me, gNS, gNS, MACH_MSG_TYPE_MAKE_SEND);
    gGlobal.port = gNS;
    mach_port_allocate(me, MACH_PORT_RIGHT_RECEIVE, &gCtl);
    mach_port_insert_right(me, gCtl, gCtl, MACH_MSG_TYPE_MAKE_SEND);
    mach_port_allocate(me, MACH_PORT_RIGHT_PORT_SET, &gSet);
    mach_port_move_member(me, gNS, gSet);
    mach_port_move_member(me, gCtl, gSet);
    mach_port_allocate(me, MACH_PORT_RIGHT_RECEIVE, &gReturned);
    mach_port_move_member(me, gReturned, gSet);
    mach_port_allocate(me, MACH_PORT_RIGHT_PORT_SET, &gIdle);

    pthread_t serve_thread;
    pthread_create(&serve_thread, NULL, serve, NULL);
    pthread_t idle_thread;
    pthread_create(&idle_thread, NULL, idle_watch, NULL);

    mach_port_array_t ports = NULL;
    mach_msg_type_number_t count = 0;
    if (mach_ports_lookup(me, &ports, &count) == KERN_SUCCESS && count > 0 && ports[0]) {
        waterui_simns_port_msg_t hello = {0};
        hello.h.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0) | MACH_MSGH_BITS_COMPLEX;
        hello.h.msgh_size = sizeof(hello);
        hello.h.msgh_remote_port = ports[0];
        hello.h.msgh_id = WATERUI_SIMNS_MSG_HELLO;
        hello.b.msgh_descriptor_count = 1;
        hello.port.name = gCtl;
        hello.port.disposition = MACH_MSG_TYPE_MAKE_SEND;
        hello.port.type = MACH_MSG_PORT_DESCRIPTOR;
        mach_msg(&hello.h, MACH_SEND_MSG | MACH_SEND_TIMEOUT, sizeof(hello), 0, 0, 1000, 0);
        for (int waited = 0; waited < 5000 && !gReady; waited += 10) {
            usleep(10000);
        }
    }

    int envc = 0;
    while (environ[envc]) {
        envc++;
    }
    gChildEnv = calloc((size_t)envc + 3, sizeof(char *));
    int k = 0;
    for (int i = 0; i < envc; i++) {
        if (strncmp(environ[i], "WATERUI_SIMNS_", 14) == 0 ||
            strncmp(environ[i], "DYLD_INSERT_LIBRARIES=", 22) == 0 ||
            strncmp(environ[i], "XPC_SIMULATOR_LAUNCHD_NAME=", 27) == 0 ||
            strncmp(environ[i], "XPC_FLAGS=", 10) == 0) {
            continue;
        }
        gChildEnv[k++] = environ[i];
    }
    gChildEnv[k++] = "XPC_SIMULATOR_LAUNCHD_NAME=dev.waterui.simns";
    gChildEnv[k++] = "XPC_FLAGS=0x0";
    gRoot = getenv("SIMULATOR_ROOT") ? getenv("SIMULATOR_ROOT") : "";
    load_daemons();

    signal(SIGTERM, on_term);
    signal(SIGINT, on_term);
    signal(SIGHUP, on_term);

    pid_t parent = getppid();
    while (getppid() == parent) {
        usleep(100000);
    }
    stop_daemons();
    return 0;
}
