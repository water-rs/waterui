// Competitive benchmark contestant — GTK 4 (water-rs/waterui#1262).
// Implements the canonical workload spec (benchmarks/competitive/README.md):
// the same constants the shared apps/* contestants render. Workload selected
// by BENCH_WORKLOAD (w1..w6); a missing or unrecognized value traps.

#include <gtk/gtk.h>
#include <math.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>

#define WIDTH 1280
#define HEIGHT 800
#define W2_ROWS 10000

// Canonical W3/W5 geometry: rects wander a fixed 720x440 logical field.
#define FIELD_W 720.0
#define FIELD_H 440.0
#define RECT_SIDE 40

// Canonical palette — identical in every contestant.
static const char *PALETTE[6] = {
    "#3B82F6", "#10B981", "#F59E0B", "#EF4444", "#8B5CF6", "#EC4899",
};

// Canonical W4 text — benchmarks/competitive/lib/paragraphs.txt, embedded
// (an app cannot read the suite's file at runtime).
static const char *PARAGRAPHS[10] = {
    "The quick brown fox jumps over the lazy dog. 敏捷的棕色狐狸跳過懶惰的狗。🦊🐶 Packing my box with five dozen liquor jugs.",
    "WaterUI renders native widgets from a single Rust view tree. 水のインターフェースはネイティブウィジェットを描画する。🌊",
    "Almost all programming can be viewed as state management. 几乎所有的编程都可以视为状态管理。📚 Signals flow through the graph.",
    "Sphinx of black quartz, judge my vow. 黒い水晶のスフィンクス、私の誓いを裁け。🗻 Typography is the visual component of the written word.",
    "How vexingly quick daft zebras jump! 빠른 얼룩말이 얼마나 성가시게 뛰는가! 🦓 The first principle is that you must not fool yourself.",
    "Bright vixens jump; dozy fowl quack. 밝은 여우가 뛰고 졸린 새가 꽥꽥 운다. 🐦 Rendering pipelines measure progress in milliseconds per frame.",
    "ベンチマークが正直であれば最適化も正直になる。Benchmarks that are honest make optimisation honest. 📏",
    "Two driven jocks help fax my big quiz. 두 명의 조키가 내 큰 퀴즈를 팩스로 보내는 것을 돕는다. 🌲 Lazily built lists keep memory flat.",
    "The five boxing wizards jump quickly. 五個拳擊巫師跳得很快。🧙 Every frame has a budget of 8.33 milliseconds at 120 Hz.",
    "Jackdaws love my big sphinx of quartz. 寒鸦喜欢我巨大的石英斯芬克斯。🐦‍⬛ Measure, then optimise; never optimise on faith alone.",
};

static void set_rgba(cairo_t *cr, uint32_t i, double alpha) {
    GdkRGBA c;
    gdk_rgba_parse(&c, PALETTE[i % 6]);
    cairo_set_source_rgba(cr, c.red, c.green, c.blue, alpha);
}

// xorshift64 — the shared per-rect random stream (identical constants in
// every contestant).
static uint64_t xs64_next(uint64_t *s) {
    *s ^= *s << 13; *s ^= *s >> 7; *s ^= *s << 17;
    return *s;
}
static double xs64_01(uint64_t *s) { return (xs64_next(s) % 10000) / 10000.0; }

// ---------------------------------------------------------------------------
// W1 — centred label + counter button
// ---------------------------------------------------------------------------

static void w1_clicked(GtkButton *b, gpointer data) {
    GtkLabel *label = data;
    static int count;
    count++;
    char buf[64];
    snprintf(buf, sizeof buf, "Count: %d", count);
    gtk_label_set_text(label, buf);
}

static GtkWidget *w1_build(void) {
    GtkWidget *label = gtk_label_new("Count: 0");
    gtk_widget_add_css_class(label, "w1-count");
    GtkWidget *button = gtk_button_new_with_label("Increment");
    gtk_widget_add_css_class(button, "suggested-action");
    gtk_widget_add_css_class(button, "pill");
    g_signal_connect(button, "clicked", G_CALLBACK(w1_clicked), label);
    GtkWidget *col = gtk_box_new(GTK_ORIENTATION_VERTICAL, 16);
    gtk_widget_set_halign(col, GTK_ALIGN_CENTER);
    gtk_widget_set_valign(col, GTK_ALIGN_CENTER);
    gtk_box_append(GTK_BOX(col), label);
    gtk_box_append(GTK_BOX(col), button);
    return col;
}

// ---------------------------------------------------------------------------
// W2 — lazy list of 10,000 rows (canonical row content)
// ---------------------------------------------------------------------------

// w6: cells per row — 0 for w2. Palette classes carry each square's color.
static const char *const cell_palette_classes[6] = {
    "cell-c0", "cell-c1", "cell-c2", "cell-c3", "cell-c4", "cell-c5",
};
static uint32_t w6_cells;

static void draw_circle(GtkDrawingArea *da, cairo_t *cr, int w, int h,
                        gpointer data) {
    (void)da;
    (void)h;
    uint32_t i = (uint32_t)(uintptr_t)data;
    set_rgba(cr, i, 1.0);
    cairo_arc(cr, w / 2.0, w / 2.0, MIN(w, h) / 2.0, 0, 2 * M_PI);
    cairo_fill(cr);
}

static void w2_setup(GtkSignalListItemFactory *f, GtkListItem *item,
                     gpointer data) {
    (void)f;
    (void)data;
    GtkWidget *circle = gtk_drawing_area_new();
    gtk_widget_set_size_request(circle, 40, 40);
    gtk_drawing_area_set_draw_func(GTK_DRAWING_AREA(circle), draw_circle,
                                   NULL, NULL);
    gtk_widget_set_valign(circle, GTK_ALIGN_CENTER);

    GtkWidget *title = gtk_label_new("");
    gtk_label_set_xalign(GTK_LABEL(title), 0);
    gtk_widget_add_css_class(title, "row-title");
    GtkWidget *subtitle = gtk_label_new("");
    gtk_label_set_xalign(GTK_LABEL(subtitle), 0);
    gtk_widget_add_css_class(subtitle, "dim-label");
    gtk_widget_add_css_class(subtitle, "row-subtitle");
    GtkWidget *vbox = gtk_box_new(GTK_ORIENTATION_VERTICAL, 4);
    gtk_widget_set_valign(vbox, GTK_ALIGN_CENTER);
    gtk_widget_set_hexpand(vbox, TRUE);
    gtk_box_append(GTK_BOX(vbox), title);
    gtk_box_append(GTK_BOX(vbox), subtitle);

    GtkWidget *cells = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 4);
    gtk_widget_set_valign(cells, GTK_ALIGN_CENTER);
    for (uint32_t j = 0; j < w6_cells; j++) {
        GtkWidget *sq = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0);
        gtk_widget_set_size_request(sq, 14, 14);
        gtk_widget_add_css_class(sq, "cell-sq");
        GtkWidget *txt = gtk_label_new("");
        gtk_widget_add_css_class(txt, "cell-txt");
        GtkWidget *c = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
        gtk_box_append(GTK_BOX(c), sq);
        gtk_box_append(GTK_BOX(c), txt);
        gtk_box_append(GTK_BOX(cells), c);
    }

    GtkWidget *ts = gtk_label_new("");
    gtk_widget_add_css_class(ts, "dim-label");
    gtk_widget_add_css_class(ts, "row-subtitle");
    gtk_widget_set_valign(ts, GTK_ALIGN_CENTER);

    GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 12);
    gtk_widget_set_margin_top(row, 10);
    gtk_widget_set_margin_bottom(row, 10);
    gtk_widget_set_margin_start(row, 16);
    gtk_widget_set_margin_end(row, 16);
    gtk_box_append(GTK_BOX(row), circle);
    gtk_box_append(GTK_BOX(row), vbox);
    gtk_box_append(GTK_BOX(row), cells);
    gtk_box_append(GTK_BOX(row), ts);

    gtk_list_item_set_child(item, row);
}

static void w2_bind(GtkSignalListItemFactory *f, GtkListItem *item,
                    gpointer data) {
    (void)f;
    (void)data;
    uint32_t i = gtk_list_item_get_position(item);
    GtkWidget *row = gtk_list_item_get_child(item);
    GtkWidget *circle = gtk_widget_get_first_child(row);
    GtkWidget *vbox = gtk_widget_get_next_sibling(circle);
    GtkWidget *cells = gtk_widget_get_next_sibling(vbox);
    GtkWidget *ts = gtk_widget_get_next_sibling(cells);
    GtkWidget *title = gtk_widget_get_first_child(vbox);
    GtkWidget *subtitle = gtk_widget_get_next_sibling(title);

    gtk_drawing_area_set_draw_func(GTK_DRAWING_AREA(circle), draw_circle,
                                   (gpointer)(uintptr_t)i, NULL);

    char buf[128];
    snprintf(buf, sizeof buf, "Row title %u", i);
    gtk_label_set_text(GTK_LABEL(title), buf);
    snprintf(buf, sizeof buf, "Second line of subtitle for item %u", i);
    gtk_label_set_text(GTK_LABEL(subtitle), buf);

    // W6 cells: the row's cell count is fixed for the launch, so rebinds
    // repaint each retained cell rather than rebuild it.
    GtkWidget *cell = gtk_widget_get_first_child(cells);
    for (uint32_t j = 0; cell && j < w6_cells; j++) {
        GtkWidget *sq = gtk_widget_get_first_child(cell);
        GtkWidget *txt = gtk_widget_get_next_sibling(sq);
        for (int k = 0; k < 6; k++)
            gtk_widget_remove_css_class(sq, cell_palette_classes[k]);
        gtk_widget_add_css_class(sq, cell_palette_classes[(i + j) % 6]);
        snprintf(buf, sizeof buf, "c%u", j);
        gtk_label_set_text(GTK_LABEL(txt), buf);
        cell = gtk_widget_get_next_sibling(cell);
    }

    snprintf(buf, sizeof buf, "%02u:%02u", (i / 60) % 24, i % 60);
    gtk_label_set_text(GTK_LABEL(ts), buf);
}

static GtkWidget *w2_build(void) {
    GListStore *store = g_list_store_new(G_TYPE_OBJECT);
    for (int i = 0; i < W2_ROWS; i++) {
        g_list_store_append(store, g_object_new(G_TYPE_OBJECT, NULL));
    }
    GtkListItemFactory *factory = gtk_signal_list_item_factory_new();
    g_signal_connect(factory, "setup", G_CALLBACK(w2_setup), NULL);
    g_signal_connect(factory, "bind", G_CALLBACK(w2_bind), NULL);
    GtkSelectionModel *sel = GTK_SELECTION_MODEL(
        gtk_no_selection_new(G_LIST_MODEL(store)));
    GtkWidget *list = gtk_list_view_new(sel, factory);
    GtkWidget *scroll = gtk_scrolled_window_new();
    gtk_scrolled_window_set_child(GTK_SCROLLED_WINDOW(scroll), list);
    return scroll;
}

// ---------------------------------------------------------------------------
// W3 Motion / W5 capacity — `count` rects wander the field, each on its own
// xorshift64 stream; every duration tick picks new position/rotation/
// opacity targets eased in-out.
// ---------------------------------------------------------------------------

#define MAX_RECTS 25601

struct w3_rect {
    uint64_t rng;
    int dur_ms;
    gint64 tick_us;        // start of the current waypoint animation
    double fx, fy, fr, fo; // departure values of the current animation
    double tx, ty, tr, to; // waypoint targets
    double cx, cy, cr, co; // rendered values this frame
};

static struct w3_rect w3_rects[MAX_RECTS];
static int w3_count;
static gint64 w3_start_us;
static GdkRGBA w3_palette[6];

// One retained widget paints the whole rect field as GSK render nodes —
// transform + rounded-clip + color per rect — rather than per-frame cairo
// redraws of per-rect drawing areas.
#define W3_FIELD_TYPE (w3_field_get_type())
G_DECLARE_FINAL_TYPE(W3Field, w3_field, W3, FIELD, GtkWidget)
struct _W3Field { GtkWidget parent_instance; };
G_DEFINE_TYPE(W3Field, w3_field, GTK_TYPE_WIDGET)

static void w3_field_snapshot(GtkWidget *w, GtkSnapshot *snapshot) {
    (void)w;
    GskRoundedRect clip;
    gsk_rounded_rect_init(
        &clip,
        &GRAPHENE_RECT_INIT(0, 0, (float)RECT_SIDE, (float)RECT_SIDE),
        &GRAPHENE_SIZE_INIT(10, 10),
        &GRAPHENE_SIZE_INIT(10, 10),
        &GRAPHENE_SIZE_INIT(10, 10),
        &GRAPHENE_SIZE_INIT(10, 10));
    const graphene_rect_t area =
        GRAPHENE_RECT_INIT(0, 0, (float)RECT_SIDE, (float)RECT_SIDE);
    for (int i = 0; i < w3_count; i++) {
        struct w3_rect *r = &w3_rects[i];
        gtk_snapshot_save(snapshot);
        // rotation about the rect's centre
        GskTransform *t = gsk_transform_translate(
            NULL, &GRAPHENE_POINT_INIT(r->cx + RECT_SIDE / 2.0,
                                       r->cy + RECT_SIDE / 2.0));
        t = gsk_transform_rotate(t, r->cr);
        t = gsk_transform_translate(
            t, &GRAPHENE_POINT_INIT(-RECT_SIDE / 2.0, -RECT_SIDE / 2.0));
        gtk_snapshot_transform(snapshot, t);
        gsk_transform_unref(t);
        gtk_snapshot_push_rounded_clip(snapshot, &clip);
        GdkRGBA color = w3_palette[i % 6];
        color.alpha *= (float)r->co;
        gtk_snapshot_append_color(snapshot, &color, &area);
        gtk_snapshot_pop(snapshot);
        gtk_snapshot_restore(snapshot);
    }
}

static void w3_field_init(W3Field *self) { (void)self; }
static void w3_field_class_init(W3FieldClass *klass) {
    GtkWidgetClass *wc = GTK_WIDGET_CLASS(klass);
    gtk_widget_class_set_layout_manager_type(wc, GTK_TYPE_BIN_LAYOUT);
    wc->snapshot = w3_field_snapshot;
}

// ease over the tick's progress: the spec's cubic-bezier(0.42, 0, 0.58, 1)
// — solve t for x = f by bisection, then evaluate the y channel.
static double ease_bezier(double f) {
    double lo = 0.0, hi = 1.0, t = f;
    for (int it = 0; it < 24; it++) {
        double x = 3 * (1 - t) * (1 - t) * t * 0.42 +
                   3 * (1 - t) * t * t * 0.58 + t * t * t;
        if (fabs(x - f) < 1e-7) break;
        if (x < f) lo = t; else hi = t;
        t = (lo + hi) / 2.0;
    }
    return 3 * (1 - t) * t * t + t * t * t;
}

static void w3_retarget(struct w3_rect *r) {
    r->tx = xs64_01(&r->rng) * (FIELD_W - RECT_SIDE);
    r->ty = xs64_01(&r->rng) * (FIELD_H - RECT_SIDE);
    r->tr = xs64_01(&r->rng) * 360.0;
    r->to = 0.3 + xs64_01(&r->rng) * 0.7;
    r->tick_us = g_get_monotonic_time();
}

static gboolean w3_tick(GtkWidget *w, GdkFrameClock *fc, gpointer data) {
    (void)fc;
    (void)data;
    gint64 now = g_get_monotonic_time();
    for (int i = 0; i < w3_count; i++) {
        struct w3_rect *r = &w3_rects[i];
        if (now - r->tick_us >= r->dur_ms * 1000) {
            // the next waypoint departs from wherever the rect is now
            r->fx = r->cx; r->fy = r->cy; r->fr = r->cr; r->fo = r->co;
            w3_retarget(r);
        }
        double f = (now - r->tick_us) / (r->dur_ms * 1000.0);
        if (f > 1.0) f = 1.0;
        double e = ease_bezier(f);
        r->cx = r->fx + (r->tx - r->fx) * e;
        r->cy = r->fy + (r->ty - r->fy) * e;
        r->cr = r->fr + (r->tr - r->fr) * e;
        r->co = r->fo + (r->to - r->fo) * e;
    }
    gtk_widget_queue_draw(w);
    return G_SOURCE_CONTINUE;
}

static GtkWidget *w3_build(int count) {
    w3_count = count;
    GtkWidget *field = g_object_new(W3_FIELD_TYPE, NULL);
    gtk_widget_set_size_request(field, (int)FIELD_W, (int)FIELD_H);
    gtk_widget_set_halign(field, GTK_ALIGN_CENTER);
    gtk_widget_set_valign(field, GTK_ALIGN_CENTER);
    for (int k = 0; k < 6; k++)
        gdk_rgba_parse(&w3_palette[k], PALETTE[k]);
    w3_start_us = g_get_monotonic_time();
    for (int i = 0; i < count; i++) {
        struct w3_rect *r = &w3_rects[i];
        uint64_t init = 0xD1B54A32D192ED03ULL ^ (uint64_t)i * 0x2545F4914F6CDD1DULL;
        r->cx = xs64_01(&init) * (FIELD_W - RECT_SIDE);
        r->cy = xs64_01(&init) * (FIELD_H - RECT_SIDE);
        r->cr = xs64_01(&init) * 360.0;
        r->co = 0.3 + xs64_01(&init) * 0.7;
        r->fx = r->cx; r->fy = r->cy; r->fr = r->cr; r->fo = r->co;
        r->rng = 0x9E3779B97F4A7C15ULL ^ (uint64_t)i * 0xBF58476D1CE4E5B9ULL;
        r->dur_ms = 1200 + (i % 5) * 200;
        r->tick_us = w3_start_us;
        w3_retarget(r);
        r->tick_us = w3_start_us;
    }
    gtk_widget_add_tick_callback(field, w3_tick, NULL, NULL);
    GtkWidget *center = gtk_center_box_new();
    gtk_center_box_set_center_widget(GTK_CENTER_BOX(center), field);
    return center;
}

// ---------------------------------------------------------------------------
// W4 — scrolling screen of 50 paragraphs of mixed Latin/CJK/emoji
// ---------------------------------------------------------------------------

static GtkWidget *w4_build(void) {
    GtkWidget *col = gtk_box_new(GTK_ORIENTATION_VERTICAL, 6);
    for (int i = 0; i < 50; i++) {
        GtkWidget *p = gtk_label_new(PARAGRAPHS[i % 10]);
        gtk_widget_add_css_class(p, "w4-para");
        gtk_label_set_xalign(GTK_LABEL(p), 0);
        gtk_label_set_wrap(GTK_LABEL(p), TRUE);
        gtk_widget_set_margin_top(p, 10);
        gtk_widget_set_margin_bottom(p, 10);
        gtk_widget_set_margin_start(p, 16);
        gtk_widget_set_margin_end(p, 16);
        gtk_box_append(GTK_BOX(col), p);
    }
    GtkWidget *scroll = gtk_scrolled_window_new();
    gtk_scrolled_window_set_child(GTK_SCROLLED_WINDOW(scroll), col);
    return scroll;
}

// ---------------------------------------------------------------------------

// BENCH_STEP pins one ladder step per launch — a missing or out-of-ladder
// step is a hard failure, never a silent default (M9/WORKLOADS.md).
static const int W5_LADDER[8] = {200, 400, 800, 1600, 3200, 6400, 12800,
                                 25600};
static const int W6_LADDER[7] = {1, 2, 4, 8, 16, 32, 64};
static const char *w5_wl;
static int ladder_step(void) {
    const int *ladder = !strcmp(w5_wl, "w6") ? W6_LADDER : W5_LADDER;
    size_t len = !strcmp(w5_wl, "w6")
        ? sizeof(W6_LADDER) / sizeof(W6_LADDER[0])
        : sizeof(W5_LADDER) / sizeof(W5_LADDER[0]);
    const char *s = getenv("BENCH_STEP");
    if (!s || !*s) {
        g_critical("missing BENCH_STEP for %s; expected one of "
                   "the declared ladder members", w5_wl);
        abort();
    }
    char *end = NULL;
    long n = strtol(s, &end, 10);
    if (!end || *end != '\0') {
        g_critical("malformed BENCH_STEP '%s'; expected an integer", s);
        abort();
    }
    for (size_t k = 0; k < len; k++)
        if (ladder[k] == n)
            return (int)n;
    g_critical("unrecognized BENCH_STEP %ld; expected one of the %s ladder",
               n, w5_wl);
    abort();
}

static GtkWidget *w6_build(uint32_t cells) {
    w6_cells = cells;
    return w2_build();
}

static void activate(GtkApplication *app, gpointer data) {
    (void)data;
    GtkWidget *win = gtk_application_window_new(app);
    gtk_window_set_default_size(GTK_WINDOW(win), WIDTH, HEIGHT);
    gtk_window_set_title(GTK_WINDOW(win), "WaterUI Bench");

    GtkCssProvider *css = gtk_css_provider_new();
    gtk_css_provider_load_from_string(
        css,
        ".w1-count { font-size: 20px; }\n"
        ".w4-para { font-size: 16px; }\n"
        ".row-title { font-size: 16px; }\n"
        ".row-subtitle { font-size: 13px; opacity: 0.7; }\n"
        ".cell-sq { border-radius: 4px; }\n"
        ".cell-txt { font-size: 12px; opacity: 0.7; }\n"
        ".cell-c0 { background: #3B82F6; }\n"
        ".cell-c1 { background: #10B981; }\n"
        ".cell-c2 { background: #F59E0B; }\n"
        ".cell-c3 { background: #EF4444; }\n"
        ".cell-c4 { background: #8B5CF6; }\n"
        ".cell-c5 { background: #EC4899; }\n");
    gtk_style_context_add_provider_for_display(
        gdk_display_get_default(), GTK_STYLE_PROVIDER(css),
        GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);

    const char *wl = getenv("BENCH_WORKLOAD");
    GtkWidget *child;
    if (wl && !strcmp(wl, "w1"))
        child = w1_build();
    else if (wl && !strcmp(wl, "w2"))
        child = w2_build();
    else if (wl && !strcmp(wl, "w3"))
        child = w3_build(200);
    else if (wl && !strcmp(wl, "w4"))
        child = w4_build();
    else if (wl && !strcmp(wl, "w5"))
        child = w3_build(({ w5_wl = "w5"; ladder_step(); }));
    else if (wl && !strcmp(wl, "w6"))
        child = w6_build(({ w5_wl = "w6"; ladder_step(); }));
    else {
        g_critical("missing or unrecognized BENCH_WORKLOAD (%s); "
                   "expected w1..w6", wl ? wl : "<unset>");
        abort();
    }
    gtk_window_set_child(GTK_WINDOW(win), child);
    gtk_window_present(GTK_WINDOW(win));
}

int main(int argc, char **argv) {
    GtkApplication *app = gtk_application_new(
        "rs.water.bench.gtk4", G_APPLICATION_DEFAULT_FLAGS);
    g_signal_connect(app, "activate", G_CALLBACK(activate), NULL);
    int status = g_application_run(G_APPLICATION(app), argc, argv);
    g_object_unref(app);
    return status;
}
