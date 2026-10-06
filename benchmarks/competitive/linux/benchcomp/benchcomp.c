// benchcomp — a minimal wlroots compositor for competitive benchmarking.
//
// One headless output driven by a fixed vsync timer, one maximized
// xdg_toplevel per spawned client, a uniform JSONL event log:
// spawn/map/commit/present/frame/mem/input/anchor/window_start/drm/
// window_end/lib/fd/evidence_error/app_exit/exit. Present
// timestamps are taken at scene commit in CLOCK_MONOTONIC — identical
// plumbing for every contestant app.
//
// Script file lines:  <ms> motion <x> <y> | click | press | release
//                     | axis <steps>     (wheel; + scrolls down)
#define _POSIX_C_SOURCE 200809L
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <linux/input-event-codes.h>
#include <signal.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define _GNU_SOURCE
#include <dlfcn.h>
#include <drm/drm_fourcc.h>
#include <xf86drm.h>
#include <sys/sysmacros.h>
#include <wayland-server-core.h>
#include <wayland-util.h>
#include <wlr/backend.h>
#include <wlr/backend/headless.h>
#include <wlr/render/allocator.h>
#include <wlr/render/wlr_renderer.h>
#include <wlr/types/wlr_compositor.h>
#include <wlr/types/wlr_data_device.h>
#include <wlr/types/wlr_fractional_scale_v1.h>
#include <wlr/types/wlr_linux_dmabuf_v1.h>
#include <wlr/types/wlr_output.h>
#include <wlr/types/wlr_output_layout.h>
#include <wlr/types/wlr_presentation_time.h>
#include <wlr/types/wlr_scene.h>
#include <wlr/types/wlr_seat.h>
#include <wlr/types/wlr_single_pixel_buffer_v1.h>
#include <wlr/types/wlr_subcompositor.h>
#include <wlr/types/wlr_viewporter.h>
#include <wlr/types/wlr_xdg_output_v1.h>
#include <wlr/types/wlr_xdg_shell.h>
#include <wlr/util/log.h>

static uint64_t now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

struct benchcomp;

struct bench_surface {
    struct wl_list link;
    struct benchcomp *comp;
    struct wlr_xdg_surface *xsurface;
    struct wlr_xdg_toplevel *toplevel;
    struct wlr_scene_tree *tree;
    bool committed;
    bool configured;
    int id;
    struct wl_listener commit;
    struct wl_listener map;
    struct wl_listener unmap;
    struct wl_listener destroy;
};

struct script_event {
    uint64_t at_ms;
    char op[16];
    double a, b;
    struct wl_list link;
};

struct benchcomp {
    struct wl_display *display;
    struct wl_event_loop *loop;
    struct wlr_backend *backend;
    struct wlr_renderer *renderer;
    struct wlr_allocator *allocator;
    struct wlr_scene *scene;
    struct wlr_output_layout *layout;
    struct wlr_seat *seat;
    struct wlr_output *output;
    struct wlr_scene_output *scene_output;
    struct wlr_xdg_shell *xdg_shell;

    FILE *log;
    int out_w, out_h, refresh_mhz;
    uint64_t frame_seq;
    struct wl_listener new_output;
    struct wl_listener output_frame;
    struct wl_listener output_destroy;
    struct wl_listener new_xdg_surface;

    struct wl_list surfaces;
    int next_surface_id;

    double cx, cy;

    struct wl_list script;
    struct script_event *script_next;
    struct wl_event_source *script_source;
    struct wl_event_source *vsync_timer;
    struct wl_event_source *sig_timer;
    struct wl_event_source *window_timer;
    struct wl_event_source *dur_timer;
    bool frame_pending;

    char *spawn_cmd;
    pid_t spawn_pid;
    struct wl_event_source *child_timer;

    char *cgroup;
    struct wl_event_source *mem_timer;

    uint64_t duration_ms;
    uint64_t warmup_ms;
    uint64_t start_t;
    // measurement window anchor: CLOCK_MONOTONIC ns of the first committed
    // present owned by the spawned app. The drive script and the capture
    // end are scheduled relative to anchor + warmup, never to spawn.
    uint64_t anchor_t;
    bool anchored;
    struct wl_event_source *present_timer;
    int exit_code;
    bool done;
    bool nested;
};

static volatile sig_atomic_t g_quit = 0;

static void logf_ev(struct benchcomp *c, const char *ev, const char *fmt, ...) {
    uint64_t t = now_ns();
    fprintf(c->log, "{\"ev\":\"%s\",\"t\":%llu", ev, (unsigned long long)t);
    if (fmt && fmt[0]) {
        fputc(',', c->log);
        va_list ap;
        va_start(ap, fmt);
        vfprintf(c->log, fmt, ap);
        va_end(ap);
    }
    fputc('}', c->log);
    fputc('\n', c->log);
    fflush(c->log);
}

// wlroots' linux-dmabuf sanity check resolves feedback.main_device via
// libdrm and opens its render node. This VM has no /dev/dri at all, so we
// fabricate a minimal drmDevice (primary node, no render node) that wlroots
// accepts and then skips fd sanity checks (main_device_fd = -1). Only the
// sentinel dev_t we put into our own feedback is intercepted; real lookups
// pass through. benchcomp is linked -rdynamic so libwlroots' PLT call
// resolves here; the real libdrm symbol is reached via RTLD_NEXT.
#define BENCH_FAKE_DRM_DEV ((dev_t)0xBE7C0000)
// bound on the spawned app producing its first owned present; a
// non-presenting app is a failed rep, not a hung measurement
#define PRESENT_DEADLINE_MS 60000

static void anchor(struct benchcomp *c);
typedef int (*getdev_fn_t)(dev_t, uint32_t, drmDevicePtr *);
int drmGetDeviceFromDevId(dev_t dev, uint32_t flags, drmDevicePtr *device) {
    static getdev_fn_t real_fn;
    if (!real_fn)
        real_fn = (getdev_fn_t)dlsym(RTLD_NEXT, "drmGetDeviceFromDevId");
    int ret = real_fn ? real_fn(dev, flags, device) : -ENOENT;
    if (ret == 0 || dev != BENCH_FAKE_DRM_DEV) return ret;
    static char node_primary[] = "/dev/dri/card0";
    static char *nodes[DRM_NODE_MAX];
    nodes[DRM_NODE_PRIMARY] = node_primary;
    drmDevice *d = calloc(1, sizeof *d);
    if (!d) return -ENOMEM;
    d->nodes = nodes;
    d->available_nodes = 1 << DRM_NODE_PRIMARY;
    d->bustype = DRM_BUS_PCI; // plain free() in drmFreeDevice
    *device = d;
    return 0;
}

// ---------------- memory sampling (cgroup v2) ----------------

// One cgroup counter; false (errno set, EINVAL for unparseable content)
// when it cannot be read.
static bool read_u64_file(const char *path, uint64_t *v) {
    FILE *f = fopen(path, "r");
    if (!f) return false;
    unsigned long long x;
    int n = fscanf(f, "%llu", &x);
    fclose(f);
    if (n != 1) {
        errno = EINVAL;
        return false;
    }
    *v = x;
    return true;
}

// The cgroup's memory every 100 ms. A counter that cannot be read is
// logged as mem_error with its errno — the runner fails the rep; it is
// never recorded as zero.
static int mem_sample(void *data) {
    struct benchcomp *c = data;
    char cur_p[PATH_MAX + 32], peak_p[PATH_MAX + 32];
    snprintf(cur_p, sizeof cur_p, "%s/memory.current", c->cgroup);
    snprintf(peak_p, sizeof peak_p, "%s/memory.peak", c->cgroup);
    uint64_t cur, peak;
    if (!read_u64_file(cur_p, &cur))
        logf_ev(c, "mem_error", "\"what\":\"memory.current\",\"errno\":%d",
                errno);
    else if (!read_u64_file(peak_p, &peak))
        logf_ev(c, "mem_error", "\"what\":\"memory.peak\",\"errno\":%d",
                errno);
    else
        logf_ev(c, "mem", "\"current\":%llu,\"peak\":%llu",
                (unsigned long long)cur, (unsigned long long)peak);
    wl_event_source_timer_update(c->mem_timer, 100);
    return 0;
}

static bool cgroup_fail(const char *step, const char *path) {
    fprintf(stderr, "benchcomp: cgroup setup: %s %s: %s\n", step, path,
            strerror(errno));
    return false;
}

// The app's memory cgroup under the container's private cgroup namespace.
// The no-internal-process rule blocks enabling +memory on a cgroup that
// still holds procs, so every root-cgroup process (benchcomp included)
// first moves into an `init` child — one pid per write, as cgroup.procs
// takes them; a pid that exited before its move (ESRCH) no longer occupies
// the root. Every other failure fails the setup with its errno.
static bool cgroup_setup(struct benchcomp *c, const char *name) {
    const char *init_dir = "/sys/fs/cgroup/init";
    if (mkdir(init_dir, 0755) != 0 && errno != EEXIST)
        return cgroup_fail("mkdir", init_dir);
    FILE *procs = fopen("/sys/fs/cgroup/cgroup.procs", "r");
    if (!procs) return cgroup_fail("open", "/sys/fs/cgroup/cgroup.procs");
    int dst = open("/sys/fs/cgroup/init/cgroup.procs", O_WRONLY);
    if (dst < 0) {
        fclose(procs);
        return cgroup_fail("open", "/sys/fs/cgroup/init/cgroup.procs");
    }
    int pid;
    while (fscanf(procs, "%d", &pid) == 1) {
        char buf[32];
        int n = snprintf(buf, sizeof buf, "%d", pid);
        if (write(dst, buf, (size_t)n) != n && errno != ESRCH) {
            close(dst);
            fclose(procs);
            return cgroup_fail("move pid into", init_dir);
        }
    }
    close(dst);
    fclose(procs);
    char path[PATH_MAX];
    int n = snprintf(path, sizeof path, "/sys/fs/cgroup/%s", name);
    if (n < 0 || (size_t)n >= sizeof path) {
        errno = ENAMETOOLONG;
        return cgroup_fail("name", name);
    }
    FILE *sc = fopen("/sys/fs/cgroup/cgroup.subtree_control", "w");
    if (!sc) return cgroup_fail("open", "/sys/fs/cgroup/cgroup.subtree_control");
    fputs("+memory", sc);
    if (fclose(sc) != 0)
        return cgroup_fail("enable +memory in",
                           "/sys/fs/cgroup/cgroup.subtree_control");
    if (mkdir(path, 0755) != 0 && errno != EEXIST)
        return cgroup_fail("mkdir", path);
    char probe[PATH_MAX + 32];
    snprintf(probe, sizeof probe, "%s/memory.current", path);
    if (access(probe, R_OK) != 0) return cgroup_fail("read", probe);
    c->cgroup = strdup(path);
    c->mem_timer = wl_event_loop_add_timer(c->loop, mem_sample, c);
    wl_event_source_timer_update(c->mem_timer, 100);
    return true;
}

// ---------------- renderer evidence ----------------

// A JSON string value: quotes, backslashes and control bytes escaped.
static void json_str(FILE *f, const char *v) {
    fputc('"', f);
    for (const unsigned char *s = (const unsigned char *)v; *s; s++) {
        if (*s == '"' || *s == '\\')
            fprintf(f, "\\%c", *s);
        else if (*s < 0x20)
            fprintf(f, "\\u%04x", *s);
        else
            fputc(*s, f);
    }
    fputc('"', f);
}

// One JSONL event carrying a path string.
static void log_path_ev(struct benchcomp *c, const char *ev, int pid,
                        const char *key, const char *path) {
    fprintf(c->log, "{\"ev\":\"%s\",\"t\":%llu,\"pid\":%d,\"%s\":", ev,
            (unsigned long long)now_ns(), pid, key);
    json_str(c->log, path);
    fputs("}\n", c->log);
}

// A piece of renderer evidence that could not be read: what, for which
// pid, and errno. The runner fails the rep on any of these — evidence is
// never silently partial.
static void evidence_error(struct benchcomp *c, const char *what, int pid,
                           int err) {
    logf_ev(c, "evidence_error", "\"what\":\"%s\",\"pid\":%d,\"errno\":%d",
            what, pid, err);
}

typedef void (*fd_visit_fn)(struct benchcomp *c, int pid, const char *fd,
                            const char *target, void *ctx);

// Every device node (/dev/...) held open by every process of the app's
// cgroup, handed to `visit`. A pid whose fd directory cannot be opened is
// an evidence error; an fd that closes between listing and readlink is no
// longer held and is skipped.
static void visit_device_fds(struct benchcomp *c, fd_visit_fn visit,
                             void *ctx) {
    char p[PATH_MAX + 32];
    snprintf(p, sizeof p, "%s/cgroup.procs", c->cgroup);
    FILE *procs = fopen(p, "r");
    if (!procs) {
        evidence_error(c, "cgroup.procs", 0, errno);
        return;
    }
    int pid;
    while (fscanf(procs, "%d", &pid) == 1) {
        snprintf(p, sizeof p, "/proc/%d/fd", pid);
        DIR *fds = opendir(p);
        if (!fds) {
            evidence_error(c, "fd", pid, errno);
            continue;
        }
        struct dirent *de;
        while ((de = readdir(fds))) {
            if (de->d_name[0] == '.') continue;
            char link[PATH_MAX], target[PATH_MAX];
            int len = snprintf(link, sizeof link, "%s/%s", p, de->d_name);
            if (len < 0 || (size_t)len >= sizeof link) {
                evidence_error(c, "fd", pid, ENAMETOOLONG);
                continue;
            }
            ssize_t n = readlink(link, target, sizeof target - 1);
            if (n < 0) {
                if (errno != ENOENT) evidence_error(c, "fd", pid, errno);
                continue;
            }
            target[n] = 0;
            if (strncmp(target, "/dev/", 5)) continue;
            visit(c, pid, de->d_name, target, ctx);
        }
        closedir(fds);
    }
    fclose(procs);
}

// One "drm" event per render-node fd: the DRM fdinfo usage counters the
// kernel keeps for that client — drm-engine-<engine> busy ns (amdgpu,
// i915, msm, panfrost, v3d, ...) and drm-cycles-<class> (xe) — with the
// client id and driver that identify it. Read at window start and again
// at window end (one read each, no sampling); the runner's delta across
// the window is what proves the GPU did the contestant's work.
static void log_drm_usage_fd(struct benchcomp *c, int pid, const char *fd,
                             const char *target, void *ctx) {
    const char *phase = ctx;
    if (strncmp(target, "/dev/dri/renderD", 16)) return;
    char p[PATH_MAX];
    int len = snprintf(p, sizeof p, "/proc/%d/fdinfo/%s", pid, fd);
    if (len < 0 || (size_t)len >= sizeof p) {
        evidence_error(c, "fdinfo", pid, ENAMETOOLONG);
        return;
    }
    FILE *info = fopen(p, "r");
    if (!info) {
        if (errno != ENOENT) evidence_error(c, "fdinfo", pid, errno);
        return;
    }
    fprintf(c->log, "{\"ev\":\"drm\",\"t\":%llu,\"phase\":\"%s\","
            "\"pid\":%d,\"fd\":%s,\"target\":",
            (unsigned long long)now_ns(), phase, pid, fd);
    json_str(c->log, target);
    fputs(",\"counters\":{", c->log);
    // Every `drm-` line is "<key>:<value>" with any run of spaces/tabs
    // (including none) after the colon; one that cannot be split, or that
    // does not fit the line buffer, is an evidence error — never a
    // silently missing counter. Errors are logged after this event's
    // line closes, so they cannot interleave with it.
    char line[512];
    bool first = true, in_tail = false;
    int bad = 0;
    while (fgets(line, sizeof line, info)) {
        size_t n = strlen(line);
        bool ends = n > 0 && line[n - 1] == '\n';
        bool tail = in_tail;    // this chunk continues an over-long line
        in_tail = !ends;
        if (tail || strncmp(line, "drm-", 4)) continue;
        if (!ends && !feof(info)) {
            bad = EOVERFLOW;
            continue;
        }
        while (n > 0 && strchr("\n\r \t", line[n - 1])) line[--n] = 0;
        char *colon = strchr(line, ':');
        if (!colon) {
            bad = EPROTO;
            continue;
        }
        *colon = 0;
        const char *val = colon + 1;
        val += strspn(val, " \t");
        if (!first) fputc(',', c->log);
        first = false;
        json_str(c->log, line);
        fputc(':', c->log);
        json_str(c->log, val);
    }
    if (ferror(info)) bad = EIO;
    fclose(info);
    fputs("}}\n", c->log);
    if (bad) evidence_error(c, "fdinfo-format", pid, bad);
}

static void log_drm_usage(struct benchcomp *c, const char *phase) {
    visit_device_fds(c, log_drm_usage_fd, (void *)phase);
    fflush(c->log);
}

static void log_fd_target(struct benchcomp *c, int pid, const char *fd,
                          const char *target, void *ctx) {
    (void)fd;
    (void)ctx;
    log_path_ev(c, "fd", pid, "target", target);
}

// At window end, before the app is stopped — supporting evidence beside
// the DRM usage counters: every shared object mapped and every device
// node held open by each process of the app's cgroup (the Mesa/Vulkan
// driver libraries the loader mapped and the render nodes opened). Read
// once, so no sampling perturbs the capture; a maps file or fd directory
// that cannot be read is an evidence error.
static void log_renderer_evidence(struct benchcomp *c) {
    char p[PATH_MAX + 32];
    snprintf(p, sizeof p, "%s/cgroup.procs", c->cgroup);
    FILE *procs = fopen(p, "r");
    if (!procs) {
        evidence_error(c, "cgroup.procs", 0, errno);
        return;
    }
    int pid;
    while (fscanf(procs, "%d", &pid) == 1) {
        snprintf(p, sizeof p, "/proc/%d/maps", pid);
        FILE *maps = fopen(p, "r");
        if (!maps) {
            evidence_error(c, "maps", pid, errno);
            continue;
        }
        char line[PATH_MAX + 256], last[sizeof line] = "";
        while (fgets(line, sizeof line, maps)) {
            char *path = strchr(line, '/');
            if (!path || !strstr(path, ".so")) continue;
            path[strcspn(path, "\n")] = 0;
            // a library spans several consecutive mappings
            if (!strcmp(path, last)) continue;
            snprintf(last, sizeof last, "%s", path);
            log_path_ev(c, "lib", pid, "path", path);
        }
        fclose(maps);
    }
    fclose(procs);
    visit_device_fds(c, log_fd_target, NULL);
    fflush(c->log);
}

// ---------------- surfaces ----------------

static bool is_toplevel(struct bench_surface *s);
static void surface_recheck(struct bench_surface *s);

static void surface_commit(struct wl_listener *listener, void *data) {
    (void)data;
    struct bench_surface *s = wl_container_of(listener, s, commit);
    s->committed = true;
    surface_recheck(s);
    // Toolkits hold their first buffer until they receive a configure;
    // drive the initial configure (full-output size, maximized) here.
    if (is_toplevel(s) && !s->xsurface->surface->mapped &&
        !s->configured) {
        struct benchcomp *c = s->comp;
        wlr_xdg_toplevel_set_size(s->toplevel, c->out_w, c->out_h);
        wlr_xdg_toplevel_set_maximized(s->toplevel, true);
        wlr_xdg_toplevel_set_activated(s->toplevel, true);
        wlr_xdg_surface_schedule_configure(s->xsurface);
        s->configured = true;
    }
    logf_ev(s->comp, "commit", "\"surf\":%d", s->id);
}

static void surface_recheck(struct bench_surface *s) {
    if (!s->toplevel && s->xsurface->role == WLR_XDG_SURFACE_ROLE_TOPLEVEL)
        s->toplevel = s->xsurface->toplevel;
}

static void surface_map(struct wl_listener *listener, void *data) {
    (void)data;
    struct bench_surface *s = wl_container_of(listener, s, map);
    surface_recheck(s);
    if (!is_toplevel(s)) return;
    struct benchcomp *c = s->comp;
    wlr_xdg_toplevel_set_size(s->toplevel, c->out_w, c->out_h);
    wlr_xdg_toplevel_set_maximized(s->toplevel, true);
    wlr_xdg_toplevel_set_activated(s->toplevel, true);
    wlr_xdg_surface_schedule_configure(s->xsurface);
    wlr_scene_node_set_position(&s->tree->node, 0, 0);
    // Clients that gate rAF/BeginFrame on output visibility (Chromium)
    // need wl_surface.enter before they'll produce frames.
    wlr_surface_send_enter(s->xsurface->surface, c->output);
    c->cx = c->out_w / 2.0;
    c->cy = c->out_h / 2.0;
    wlr_seat_pointer_notify_enter(c->seat, s->toplevel->base->surface,
                                  c->cx, c->cy);
    wlr_seat_pointer_notify_motion(c->seat,
                                   (uint32_t)(now_ns() / 1000000), c->cx, c->cy);
    wlr_seat_pointer_notify_frame(c->seat);
    logf_ev(c, "map", "\"surf\":%d,\"app_id\":\"%s\"",
            s->id,
            s->toplevel->app_id ? s->toplevel->app_id : "");
}

static void surface_unmap(struct wl_listener *listener, void *data) {
    (void)data;
    struct bench_surface *s = wl_container_of(listener, s, unmap);
    surface_recheck(s);
    if (!is_toplevel(s)) return;
    logf_ev(s->comp, "unmap", "\"surf\":%d", s->id);
}

static void surface_destroy(struct wl_listener *listener, void *data) {
    (void)data;
    struct bench_surface *s = wl_container_of(listener, s, destroy);
    wl_list_remove(&s->commit.link);
    wl_list_remove(&s->map.link);
    wl_list_remove(&s->unmap.link);
    wl_list_remove(&s->destroy.link);
    wl_list_remove(&s->link);
    free(s);
}

static void new_xdg_surface(struct wl_listener *listener, void *data) {
    struct benchcomp *c = wl_container_of(listener, c, new_xdg_surface);
    struct wlr_xdg_surface *xs = data;
    struct bench_surface *s = calloc(1, sizeof *s);
    s->comp = c;
    s->id = ++c->next_surface_id;
    s->xsurface = xs;
    s->toplevel = xs->toplevel; // NULL until role assign; checked lazily
    s->tree = wlr_scene_xdg_surface_create(&c->scene->tree, xs);
    wl_list_insert(&c->surfaces, &s->link);
    s->commit.notify = surface_commit;
    wl_signal_add(&xs->surface->events.commit, &s->commit);
    s->map.notify = surface_map;
    wl_signal_add(&xs->surface->events.map, &s->map);
    s->unmap.notify = surface_unmap;
    wl_signal_add(&xs->surface->events.unmap, &s->unmap);
    s->destroy.notify = surface_destroy;
    wl_signal_add(&xs->events.destroy, &s->destroy);
    logf_ev(c, "new_surface", "\"surf\":%d", s->id);
}

// new_surface fires on get_xdg_surface, before the role is assigned, so we
// attach to every xdg surface and gate on role inside the handlers.
static bool is_toplevel(struct bench_surface *s) {
    return s->toplevel &&
           s->toplevel->base->role == WLR_XDG_SURFACE_ROLE_TOPLEVEL;
}

// ---------------- frame/present ----------------

static void output_frame(struct wl_listener *listener, void *data) {
    (void)data;
    struct benchcomp *c = wl_container_of(listener, c, output_frame);
    c->frame_pending = false;
    wlr_scene_output_commit(c->scene_output, NULL);
    c->frame_seq++;
    logf_ev(c, "frame", "\"seq\":%llu", (unsigned long long)c->frame_seq);
    struct bench_surface *s;
    wl_list_for_each(s, &c->surfaces, link) {
        surface_recheck(s);
        if (is_toplevel(s) && s->xsurface->surface &&
            s->xsurface->surface->mapped) {
            wlr_presentation_surface_textured_on_output(
                s->xsurface->surface, c->output);
            logf_ev(c, "present", "\"surf\":%d,\"seq\":%llu,\"committed\":%d",
                    s->id, (unsigned long long)c->frame_seq,
                    s->committed ? 1 : 0);
            if (s->committed && !c->anchored)
                anchor(c);
            s->committed = false;
        }
    }
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    wlr_scene_output_send_frame_done(c->scene_output, &ts);
}

static int vsync_tick(void *data) {
    struct benchcomp *c = data;
    if (c->output && !c->frame_pending) {
        wlr_output_schedule_frame(c->output);
        c->frame_pending = true;
    }
    int period_ms = (int)(1000000 / c->refresh_mhz);
    if (period_ms < 1) period_ms = 1;
    wl_event_source_timer_update(c->vsync_timer, period_ms);
    return 0;
}

static void output_destroy(struct wl_listener *listener, void *data) {
    (void)data;
    struct benchcomp *c = wl_container_of(listener, c, output_destroy);
    c->output = NULL;
    wl_display_terminate(c->display);
}

static void new_output(struct wl_listener *listener, void *data) {
    struct benchcomp *c = wl_container_of(listener, c, new_output);
    struct wlr_output *o = data;
    wlr_output_init_render(o, c->allocator, c->renderer);
    struct wlr_output_state st;
    wlr_output_state_init(&st);
    wlr_output_state_set_enabled(&st, true);
    wlr_output_state_set_custom_mode(&st, c->out_w, c->out_h, c->refresh_mhz);
    if (!wlr_output_commit_state(o, &st)) {
        wlr_log(WLR_ERROR, "output commit failed");
    }
    wlr_output_state_finish(&st);
    wlr_output_layout_add_auto(c->layout, o);
    // The headless backend never fills ->refresh from the custom mode;
    // clients pace vsync from wp_presentation_feedback.presented(refresh).
    o->refresh = c->refresh_mhz;
    c->output = o;
    c->scene_output = wlr_scene_output_create(c->scene, o);
    if (!c->scene_output) {
        wlr_log(WLR_ERROR, "scene output create failed");
        wl_display_terminate(c->display);
        return;
    }
    wl_signal_add(&o->events.frame, &c->output_frame);
    c->output_frame.notify = output_frame;
    wl_signal_add(&o->events.destroy, &c->output_destroy);
    c->output_destroy.notify = output_destroy;
    logf_ev(c, "output", "\"w\":%d,\"h\":%d,\"refresh_mhz\":%d",
            c->out_w, c->out_h, c->refresh_mhz);
}

// ---------------- input scripting ----------------

static void run_script_event(struct benchcomp *c, struct script_event *e) {
    uint32_t t = (uint32_t)(now_ns() / 1000000);
    if (!strcmp(e->op, "motion")) {
        c->cx = e->a;
        c->cy = e->b;
        wlr_seat_pointer_notify_motion(c->seat, t, c->cx, c->cy);
        logf_ev(c, "input", "\"op\":\"motion\",\"x\":%.1f,\"y\":%.1f",
                e->a, e->b);
    } else if (!strcmp(e->op, "press")) {
        wlr_seat_pointer_notify_button(c->seat, t, BTN_LEFT,
                                       WL_POINTER_BUTTON_STATE_PRESSED);
        logf_ev(c, "input", "\"op\":\"press\"");
    } else if (!strcmp(e->op, "release")) {
        wlr_seat_pointer_notify_button(c->seat, t, BTN_LEFT,
                                       WL_POINTER_BUTTON_STATE_RELEASED);
        logf_ev(c, "input", "\"op\":\"release\"");
    } else if (!strcmp(e->op, "click")) {
        wlr_seat_pointer_notify_button(c->seat, t, BTN_LEFT,
                                       WL_POINTER_BUTTON_STATE_PRESSED);
        wlr_seat_pointer_notify_button(c->seat, t + 1, BTN_LEFT,
                                       WL_POINTER_BUTTON_STATE_RELEASED);
        logf_ev(c, "input", "\"op\":\"click\"");
    } else if (!strcmp(e->op, "axis")) {
        // e->a = wheel detents (+ scrolls down): 15px/detent. wlroots takes
        // value_discrete in value120 units (WLR_POINTER_AXIS_DISCRETE_STEP =
        // 120 per detent): it forwards axis_discrete only once 120 units have
        // accumulated for clients whose wl_seat is < v8, so passing the raw
        // detent count here would leave them with no wheel events at all.
        wlr_seat_pointer_notify_axis(
            c->seat, t, WL_POINTER_AXIS_VERTICAL_SCROLL, e->a * 15.0,
            (int)(e->a * WLR_POINTER_AXIS_DISCRETE_STEP),
            WL_POINTER_AXIS_SOURCE_WHEEL, WL_POINTER_AXIS_RELATIVE_DIRECTION_IDENTICAL);
        logf_ev(c, "input", "\"op\":\"axis\",\"v\":%.2f", e->a);
    }
    // wl_pointer events are grouped into pointer frames; toolkits like GTK
    // and Chromium only apply them on wl_pointer.frame.
    wlr_seat_pointer_notify_frame(c->seat);
}

static int script_tick(void *data) {
    struct benchcomp *c = data;
    // event times are relative to the measurement window start
    // (first owned present + declared warmup), never to spawn
    int64_t now_ms = (int64_t)((now_ns() - c->anchor_t) / 1000000)
                     - (int64_t)c->warmup_ms;
    while (c->script_next && (int64_t)c->script_next->at_ms <= now_ms) {
        struct script_event *e = c->script_next;
        struct script_event *next = NULL;
        if (e->link.next != &c->script)
            next = wl_container_of(e->link.next, next, link);
        c->script_next = next;
        run_script_event(c, e);
        wl_list_remove(&e->link);
        free(e);
    }
    if (c->script_next) {
        int64_t d = (int64_t)c->script_next->at_ms - (int64_t)now_ms;
        wl_event_source_timer_update(c->script_source, d < 1 ? 1 : (int)d);
    }
    return 0;
}

static void load_script(struct benchcomp *c, const char *path) {
    FILE *f = fopen(path, "r");
    if (!f) {
        fprintf(stderr, "benchcomp: cannot open script %s\n", path);
        return;
    }
    char line[256];
    while (fgets(line, sizeof line, f)) {
        if (line[0] == '#' || line[0] == '\n') continue;
        struct script_event *e = calloc(1, sizeof *e);
        char op[16] = {0};
        double a = 0, b = 0;
        int n = sscanf(line, "%llu %15s %lf %lf",
                       (unsigned long long *)&e->at_ms, op, &a, &b);
        if (n < 2) {
            free(e);
            continue;
        }
        snprintf(e->op, sizeof e->op, "%s", op);
        e->a = a;
        e->b = b;
        wl_list_insert(c->script.prev, &e->link);
    }
    fclose(f);
    // the script is armed when the window anchors (first owned present),
    // not at spawn — at_ms values are relative to window start
    if (!wl_list_empty(&c->script))
        c->script_next = wl_container_of(c->script.next, c->script_next, link);
}

// ---------------- spawn / shutdown ----------------

// The child joins the app cgroup itself ("0" moves the writer) before it
// execs, so nothing it starts can run outside the cgroup the memory and
// renderer evidence are scoped to; a child that cannot join exits 126 and
// the rep fails.
static void spawn_app(struct benchcomp *c) {
    char procs[PATH_MAX + 32];
    snprintf(procs, sizeof procs, "%s/cgroup.procs", c->cgroup);
    pid_t pid = fork();
    if (pid == 0) {
        int fd = open(procs, O_WRONLY);
        if (fd < 0 || write(fd, "0", 1) != 1) {
            fprintf(stderr, "benchcomp: cannot join %s: %s\n", procs,
                    strerror(errno));
            _exit(126);
        }
        close(fd);
        setsid();
        execl("/bin/sh", "sh", "-c", c->spawn_cmd, (char *)NULL);
        _exit(127);
    }
    c->spawn_pid = pid;
    logf_ev(c, "spawn", "\"pid\":%d", (int)pid);
}

static int child_reap(void *data) {
    struct benchcomp *c = data;
    int st;
    pid_t r = waitpid(c->spawn_pid, &st, WNOHANG);
    if (r == c->spawn_pid) {
        int code = WIFEXITED(st) ? WEXITSTATUS(st) : -1;
        logf_ev(c, "app_exit", "\"pid\":%d,\"code\":%d", (int)r, code);
        wl_event_source_remove(c->child_timer);
        c->child_timer = NULL;
    } else if (c->child_timer) {
        wl_event_source_timer_update(c->child_timer, 100);
    }
    return 0;
}

static void finish(struct benchcomp *c, int code) {
    if (c->done) return;
    c->done = true;
    c->exit_code = code;
    if (c->spawn_pid > 0) {
        kill(c->spawn_pid, SIGTERM);
        for (int i = 0; i < 20; i++) {
            if (waitpid(c->spawn_pid, NULL, WNOHANG) == c->spawn_pid) break;
            struct timespec ts = {.tv_nsec = 50000000};
            nanosleep(&ts, NULL);
        }
        kill(c->spawn_pid, SIGKILL);
        // kill any descendants that escaped (e.g. electron children)
        kill(-c->spawn_pid, SIGKILL);
    }
    logf_ev(c, "exit", "\"code\":%d", code);
    wl_display_terminate(c->display);
}

static int window_start_timer(void *data) {
    // the window opens: the DRM usage counters' starting values
    struct benchcomp *c = data;
    logf_ev(c, "window_start", NULL);
    log_drm_usage(c, "start");
    return 0;
}

static int duration_timer(void *data) {
    // the window has closed: record what the app rendered with while it
    // is still alive — the usage counters' end values, then the mapped
    // libraries and held nodes — then stop it
    struct benchcomp *c = data;
    logf_ev(c, "window_end", NULL);
    log_drm_usage(c, "end");
    log_renderer_evidence(c);
    finish(c, 0);
    return 0;
}

// Called once on the first committed present owned by the spawned app:
// opens the measurement window (anchor + warmup) and arms the drive
// script and the capture-end timer against it.
static void anchor(struct benchcomp *c) {
    c->anchored = true;
    c->anchor_t = now_ns();
    logf_ev(c, "anchor", "\"warmup_ms\":%llu",
            (unsigned long long)c->warmup_ms);
    if (c->present_timer) {
        wl_event_source_remove(c->present_timer);
        c->present_timer = NULL;
    }
    if (c->script_next) {
        c->script_source = wl_event_loop_add_timer(c->loop, script_tick, c);
        wl_event_source_timer_update(
            c->script_source,
            (int)c->warmup_ms + (int)c->script_next->at_ms + 1);
    }
    c->window_timer = wl_event_loop_add_timer(c->loop, window_start_timer, c);
    wl_event_source_timer_update(c->window_timer, (int)c->warmup_ms);
    c->dur_timer = wl_event_loop_add_timer(c->loop, duration_timer, c);
    wl_event_source_timer_update(
        c->dur_timer, (int)c->warmup_ms + (int)c->duration_ms);
}

static int present_deadline(void *data) {
    // no owned present inside the bound: a failed rep, not a measurement
    logf_ev(data, "no_present", NULL);
    finish(data, 3);
    return 0;
}

static int sig_check(void *data) {
    struct benchcomp *c = data;
    if (g_quit) {
        finish(c, 128 + g_quit);
        return 0;
    }
    wl_event_source_timer_update(c->sig_timer, 50);
    return 0;
}

static void on_signal(int sig) { g_quit = sig; }

// ---------------- main ----------------

static void usage(const char *argv0) {
    (void)argv0;
    fprintf(stderr,
            "benchcomp --spawn CMD [--script FILE] [--duration MS]\n"
            "          [--warmup MS] [--size WxH] [--refresh MHZ] "
            "[--out FILE] [--cgroup NAME] [--nested]\n");
    exit(2);
}

int main(int argc, char **argv) {
    struct benchcomp c;
    memset(&c, 0, sizeof c);
    c.out_w = 1280;
    c.out_h = 800;
    c.refresh_mhz = 60000;
    c.log = stdout;
    wl_list_init(&c.surfaces);
    wl_list_init(&c.script);

    const char *script_path = NULL, *out_path = NULL,
               *cgroup_name = "benchapp";
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--spawn") && i + 1 < argc)
            c.spawn_cmd = argv[++i];
        else if (!strcmp(argv[i], "--script") && i + 1 < argc)
            script_path = argv[++i];
        else if (!strcmp(argv[i], "--duration") && i + 1 < argc)
            c.duration_ms = strtoul(argv[++i], NULL, 10);
        else if (!strcmp(argv[i], "--warmup") && i + 1 < argc)
            c.warmup_ms = strtoul(argv[++i], NULL, 10);
        else if (!strcmp(argv[i], "--size") && i + 1 < argc)
            sscanf(argv[++i], "%dx%d", &c.out_w, &c.out_h);
        else if (!strcmp(argv[i], "--refresh") && i + 1 < argc)
            c.refresh_mhz = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--out") && i + 1 < argc)
            out_path = argv[++i];
        else if (!strcmp(argv[i], "--cgroup") && i + 1 < argc)
            cgroup_name = argv[++i];
        else if (!strcmp(argv[i], "--nested"))
            c.nested = true;
        else
            usage(argv[0]);
    }
    if (!c.spawn_cmd) usage(argv[0]);
    if (!c.warmup_ms || !c.duration_ms) {
        fprintf(stderr, "benchcomp: --warmup and --duration must be declared "
                "and nonzero (METHOD: window = [first owned present + "
                "warmup, + capture])\n");
        return 2;
    }
    if (out_path && !(c.log = fopen(out_path, "w"))) {
        fprintf(stderr, "benchcomp: cannot open %s\n", out_path);
        return 1;
    }

    wlr_log_init(WLR_ERROR, NULL);
    c.display = wl_display_create();
    c.loop = wl_display_get_event_loop(c.display);

    if (c.nested) {
        c.backend = wlr_backend_autocreate(c.loop, NULL);
    } else {
        c.backend = wlr_headless_backend_create(c.loop);
    }
    if (!c.backend) {
        fprintf(stderr, "benchcomp: backend create failed\n");
        return 1;
    }
    c.renderer = wlr_renderer_autocreate(c.backend);
    if (!c.renderer) {
        fprintf(stderr, "benchcomp: renderer create failed\n");
        return 1;
    }
    wlr_renderer_init_wl_display(c.renderer, c.display);
    c.allocator = wlr_allocator_autocreate(c.backend, c.renderer);

    wlr_compositor_create(c.display, 6, c.renderer);
    wlr_subcompositor_create(c.display);
    wlr_data_device_manager_create(c.display);
    if (!wlr_linux_dmabuf_v1_create_with_renderer(c.display, 5, c.renderer)) {
        // The renderer exposes no DRM fd (software renderer); advertise the
        // global with a minimal feedback so dmabuf clients can submit
        // buffers. Linear-modifier formats cover llvmpipe/softpipe output.
        struct wlr_linux_dmabuf_feedback_v1 fb = {0};
        wl_array_init(&fb.tranches);
        struct wlr_linux_dmabuf_feedback_v1_tranche *tr =
            wlr_linux_dmabuf_feedback_add_tranche(&fb);
        static const uint32_t fmts[] = {
            DRM_FORMAT_ARGB8888, DRM_FORMAT_XRGB8888,
            DRM_FORMAT_ABGR8888, DRM_FORMAT_XBGR8888,
            DRM_FORMAT_RGB565,   DRM_FORMAT_BGR565,
            DRM_FORMAT_ARGB2101010, DRM_FORMAT_XRGB2101010,
            DRM_FORMAT_ABGR2101010, DRM_FORMAT_XBGR2101010,
        };
        for (size_t i = 0; i < sizeof(fmts) / sizeof(fmts[0]); i++) {
            wlr_drm_format_set_add(&tr->formats, fmts[i], DRM_FORMAT_MOD_LINEAR);
            wlr_drm_format_set_add(&tr->formats, fmts[i], DRM_FORMAT_MOD_INVALID);
        }
        fb.main_device = BENCH_FAKE_DRM_DEV;
        if (!wlr_linux_dmabuf_v1_create(c.display, 5, &fb)) {
            wlr_log(WLR_ERROR, "dmabuf global unavailable");
        }
        wlr_linux_dmabuf_feedback_v1_finish(&fb);
    }
    // NB: the headless backend emits wp_presentation_feedback.presented with
    // refresh=0/seq=0. Frame timing for the benchmark is collected
    // compositor-side (the "present" events we log per output frame), so this
    // does not affect measurement.
    wlr_presentation_create(c.display, c.backend);
    wlr_viewporter_create(c.display);
    wlr_fractional_scale_manager_v1_create(c.display, 1);
    wlr_single_pixel_buffer_manager_v1_create(c.display);

    c.scene = wlr_scene_create();
    c.layout = wlr_output_layout_create(c.display);
    wlr_xdg_output_manager_v1_create(c.display, c.layout);

    c.xdg_shell = wlr_xdg_shell_create(c.display, 3);
    c.new_xdg_surface.notify = new_xdg_surface;
    wl_signal_add(&c.xdg_shell->events.new_surface, &c.new_xdg_surface);

    c.new_output.notify = new_output;
    wl_signal_add(&c.backend->events.new_output, &c.new_output);

    c.seat = wlr_seat_create(c.display, "seat0");
    wlr_seat_set_capabilities(c.seat, WL_SEAT_CAPABILITY_POINTER);

    if (!wlr_backend_start(c.backend)) {
        fprintf(stderr, "benchcomp: backend start failed\n");
        return 1;
    }
    if (!c.nested) {
        wlr_headless_add_output(c.backend, c.out_w, c.out_h);
    }

    const char *sock = wl_display_add_socket_auto(c.display);
    if (!sock) {
        fprintf(stderr, "benchcomp: no wayland socket\n");
        return 1;
    }
    setenv("WAYLAND_DISPLAY", sock, 1);
    unsetenv("DISPLAY");
    setenv("XDG_RUNTIME_DIR", getenv("XDG_RUNTIME_DIR") ?: "/tmp/bench-xdg", 0);
    setenv("MOZ_ENABLE_WAYLAND", "1", 0);

    c.start_t = now_ns();
    if (!cgroup_setup(&c, cgroup_name)) {
        // without the benchapp cgroup there is no memory measurement and
        // no renderer-evidence scope — a failed run, not a degraded one
        fprintf(stderr, "benchcomp: cgroup setup failed (%s)\n",
                cgroup_name);
        return 1;
    }

    int period_ms = (int)(1000000 / c.refresh_mhz);
    if (period_ms < 1) period_ms = 1;
    c.vsync_timer = wl_event_loop_add_timer(c.loop, vsync_tick, &c);
    wl_event_source_timer_update(c.vsync_timer, period_ms);

    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_signal;
    sigaction(SIGTERM, &sa, NULL);
    sigaction(SIGINT, &sa, NULL);
    c.sig_timer = wl_event_loop_add_timer(c.loop, sig_check, &c);
    wl_event_source_timer_update(c.sig_timer, 50);

    if (script_path) load_script(&c, script_path);

    spawn_app(&c);
    c.child_timer = wl_event_loop_add_timer(c.loop, child_reap, &c);
    wl_event_source_timer_update(c.child_timer, 100);

    // the window anchors on the app's first owned present; the bound
    // keeps a non-presenting app from hanging the rep forever
    c.present_timer = wl_event_loop_add_timer(c.loop, present_deadline, &c);
    wl_event_source_timer_update(c.present_timer,
                                 (int)PRESENT_DEADLINE_MS);

    logf_ev(&c, "start", "\"socket\":\"%s\"", sock);
    wl_display_run(c.display);

    wl_display_destroy_clients(c.display);
    wlr_scene_node_destroy(&c.scene->tree.node);
    wlr_backend_destroy(c.backend);
    wl_display_destroy(c.display);
    return c.exit_code;
}
