// Competitive benchmark contestant — GTK 4 (water-rs/waterui#1262).
// Implements the canonical workload spec (benchmarks/competitive/README.md):
// the same constants the shared apps/* contestants render. Workload selected
// by BENCH_WORKLOAD (w1..w5); a missing or unrecognized value traps.

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
    "The quick brown fox jumps over the lazy dog. 。🦊🐶 Packing my box with five dozen liquor jugs.",
    "WaterUI renders native widgets from a single Rust view tree. 。🌊 Fine-grained reactivity updates only the widgets that read the value.",
    "Almost all programming can be viewed as state management. ，。📚 Signals flow through the graph and wake the views that observe them.",
    "Sphinx of black quartz, judge my vow. のテキストもぜます。🗻 Typography is the visual component of the written word.",
    "How vexingly quick daft zebras jump! ，。🦓 The first principle is that you must not fool yourself.",
    "Bright vixens jump; dozy fowl quack. ，。🐦 Rendering pipelines measure progress in milliseconds per frame.",
    "。Benchmarks that are honest make optimisation honest. 📏",
    "Two driven jocks help fax my big quiz. ，。🌲 Lazily built lists keep memory flat while content grows without bound.",
    "The five boxing wizards jump quickly. ，。🧙 Every frame has a budget of 8.33 milliseconds at 120 Hz.",
    "Jackdaws love my big sphinx of quartz. ，。🐦‍⬛ Measure, then optimise; never optimise on faith alone.",
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
    GtkWidget *col = gtk_box_new(GTK_ORIENTATION_VERTICAL, 12);
    gtk_widget_set_halign(col, GTK_ALIGN_CENTER);
    gtk_widget_set_valign(col, GTK_ALIGN_CENTER);
    gtk_box_append(GTK_BOX(col), label);
    gtk_box_append(GTK_BOX(col), button);
    return col;
}

// ---------------------------------------------------------------------------
// W2 — lazy list of 10,000 rows (canonical row content)
// ---------------------------------------------------------------------------

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
    GtkWidget *vbox = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
    gtk_widget_set_valign(vbox, GTK_ALIGN_CENTER);
    gtk_widget_set_hexpand(vbox, TRUE);
    gtk_box_append(GTK_BOX(vbox), title);
    gtk_box_append(GTK_BOX(vbox), subtitle);

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
    GtkWidget *title = gtk_widget_get_first_child(vbox);
    GtkWidget *subtitle = gtk_widget_get_next_sibling(title);
    GtkWidget *ts = gtk_widget_get_next_sibling(vbox);

    gtk_drawing_area_set_draw_func(GTK_DRAWING_AREA(circle), draw_circle,
                                   (gpointer)(uintptr_t)i, NULL);

    char buf[128];
    snprintf(buf, sizeof buf, "Row title %u", i);
    gtk_label_set_text(GTK_LABEL(title), buf);
    snprintf(buf, sizeof buf, "Second line of subtitle for item %u", i);
    gtk_label_set_text(GTK_LABEL(subtitle), buf);
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
    GtkWidget *widget;
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

// ease-in-out over the tick's progress (canonical: cubic in/out).
static double ease_in_out(double f) {
    return f < 0.5 ? 4.0 * f * f * f
                   : 1.0 - pow(-2.0 * f + 2.0, 3.0) / 2.0;
}

static void draw_rect(GtkDrawingArea *da, cairo_t *cr, int w, int h,
                      gpointer data) {
    intptr_t i = (intptr_t)data;
    double r = 10.0, rot = w3_rects[i].cr, op = w3_rects[i].co;
    cairo_translate(cr, w / 2.0, h / 2.0);
    cairo_rotate(cr, rot * M_PI / 180.0);
    cairo_translate(cr, -w / 2.0, -h / 2.0);
    cairo_new_sub_path(cr);
    cairo_arc(cr, w - r, r, r, -M_PI / 2, 0);
    cairo_arc(cr, w - r, h - r, r, 0, M_PI / 2);
    cairo_arc(cr, r, h - r, r, M_PI / 2, M_PI);
    cairo_arc(cr, r, r, r, M_PI, 3 * M_PI / 2);
    cairo_close_path(cr);
    set_rgba(cr, (uint32_t)i, op);
    cairo_fill(cr);
}

static void w3_retarget(struct w3_rect *r) {
    r->tx = xs64_01(&r->rng) * (FIELD_W - RECT_SIDE);
    r->ty = xs64_01(&r->rng) * (FIELD_H - RECT_SIDE);
    r->tr = xs64_01(&r->rng) * 360.0;
    r->to = 0.3 + xs64_01(&r->rng) * 0.7;
    r->tick_us = g_get_monotonic_time();
}

static gboolean w3_tick(GtkWidget *w, GdkFrameClock *fc, gpointer data) {
    (void)w;
    (void)fc;
    GtkFixed *fixed = data;
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
        double e = ease_in_out(f);
        r->cx = r->fx + (r->tx - r->fx) * e;
        r->cy = r->fy + (r->ty - r->fy) * e;
        r->cr = r->fr + (r->tr - r->fr) * e;
        r->co = r->fo + (r->to - r->fo) * e;
        gtk_fixed_move(fixed, r->widget, r->cx, r->cy);
        gtk_widget_queue_draw(r->widget);
    }
    return G_SOURCE_CONTINUE;
}

static GtkWidget *w3_build(int count) {
    w3_count = count;
    GtkWidget *fixed = gtk_fixed_new();
    gtk_widget_set_size_request(fixed, (int)FIELD_W, (int)FIELD_H);
    gtk_widget_set_halign(fixed, GTK_ALIGN_CENTER);
    gtk_widget_set_valign(fixed, GTK_ALIGN_CENTER);
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
        r->widget = gtk_drawing_area_new();
        gtk_widget_set_size_request(r->widget, RECT_SIDE, RECT_SIDE);
        gtk_drawing_area_set_draw_func(GTK_DRAWING_AREA(r->widget),
                                       draw_rect, (gpointer)(intptr_t)i, NULL);
        gtk_fixed_put(GTK_FIXED(fixed), r->widget, r->cx, r->cy);
        w3_retarget(r);
        r->tick_us = w3_start_us;
    }
    gtk_widget_add_tick_callback(fixed, w3_tick, fixed, NULL);
    GtkWidget *center = gtk_center_box_new();
    gtk_center_box_set_center_widget(GTK_CENTER_BOX(center), fixed);
    return center;
}

// ---------------------------------------------------------------------------
// W4 — scrolling screen of 50 paragraphs of mixed Latin/CJK/emoji
// ---------------------------------------------------------------------------

static GtkWidget *w4_build(void) {
    GtkWidget *col = gtk_box_new(GTK_ORIENTATION_VERTICAL, 6);
    gtk_widget_set_margin_top(col, 10);
    gtk_widget_set_margin_bottom(col, 10);
    gtk_widget_set_margin_start(col, 16);
    gtk_widget_set_margin_end(col, 16);
    for (int i = 0; i < 50; i++) {
        GtkWidget *p = gtk_label_new(PARAGRAPHS[i % 10]);
        gtk_label_set_xalign(GTK_LABEL(p), 0);
        gtk_label_set_wrap(GTK_LABEL(p), TRUE);
        gtk_box_append(GTK_BOX(col), p);
    }
    GtkWidget *scroll = gtk_scrolled_window_new();
    gtk_scrolled_window_set_child(GTK_SCROLLED_WINDOW(scroll), col);
    return scroll;
}

// ---------------------------------------------------------------------------

static int w5_step(void) {
    const char *s = getenv("BENCH_STEP");
    if (s && *s) {
        int n = atoi(s);
        if (n > 0 && n < MAX_RECTS) return n;
    }
    return 200;
}

static void activate(GtkApplication *app, gpointer data) {
    (void)data;
    GtkWidget *win = gtk_application_window_new(app);
    gtk_window_set_default_size(GTK_WINDOW(win), WIDTH, HEIGHT);
    gtk_window_set_title(GTK_WINDOW(win), "WaterUI Bench");

    GtkCssProvider *css = gtk_css_provider_new();
    gtk_css_provider_load_from_string(
        css,
        ".w1-count { font-size: 24pt; }\n"
        ".row-title { font-weight: 600; }\n"
        ".row-subtitle { opacity: 0.7; }\n");
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
        child = w3_build(w5_step());
    else {
        g_critical("missing or unrecognized BENCH_WORKLOAD (%s); "
                   "expected w1..w5", wl ? wl : "<unset>");
        g_assert_not_reached();
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
