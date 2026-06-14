//! WaterUI Gallery - a browsable catalog of every example in this repository.
//!
//! The gallery is a single navigable app. Its home screen groups examples by
//! category; tapping a row pushes a detail screen. Examples that render fine in
//! a shared window (static layouts and GPU-drawn effects) are embedded live via
//! each crate's `demo()` entry. Examples that need dedicated hardware or
//! window-level capabilities (camera, microphone, native WebView, additional OS
//! windows, network video, native map, heavy profiling) are listed with a card
//! explaining how to run them standalone.

use waterui::app::App;
use waterui::navigation::{NavigationLink, NavigationPath, NavigationStack, NavigationView};
use waterui::prelude::theme_color::{Accent, MutedForeground};
use waterui::prelude::*;
use waterui::preview;

/// The route value pushed onto the navigation path: an index into [`examples`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct Route(usize);

/// Top-level grouping, mirroring the `examples/` directory taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Codes,
    Data,
    Effects,
    Media,
    App,
    Components,
    Interaction,
    Visual,
}

impl Category {
    /// Categories in display order.
    const ALL: [Self; 8] = [
        Self::Codes,
        Self::Data,
        Self::Effects,
        Self::Media,
        Self::App,
        Self::Components,
        Self::Interaction,
        Self::Visual,
    ];

    /// Section heading shown on the home screen.
    const fn title(self) -> &'static str {
        match self {
            Self::Codes => "Codes",
            Self::Data => "Data",
            Self::Effects => "Effects",
            Self::Media => "Media",
            Self::App => "App Showcases",
            Self::Components => "Components",
            Self::Interaction => "Interaction",
            Self::Visual => "Visual",
        }
    }
}

/// How a catalog entry surfaces its content in the gallery.
enum Demo {
    /// Rendered live by invoking the example crate's `demo()`.
    Embed(fn() -> AnyView),
    /// Listed with run instructions; needs dedicated hardware or window features.
    Standalone {
        /// Cargo package name to run with `water run -p <package>`.
        package: &'static str,
        /// Why it cannot be embedded inline.
        reason: &'static str,
    },
}

/// A single catalog entry.
struct Example {
    category: Category,
    title: &'static str,
    summary: &'static str,
    demo: Demo,
}

impl Example {
    const fn embed(
        category: Category,
        title: &'static str,
        summary: &'static str,
        build: fn() -> AnyView,
    ) -> Self {
        Self {
            category,
            title,
            summary,
            demo: Demo::Embed(build),
        }
    }

    const fn standalone(
        category: Category,
        title: &'static str,
        summary: &'static str,
        package: &'static str,
        reason: &'static str,
    ) -> Self {
        Self {
            category,
            title,
            summary,
            demo: Demo::Standalone { package, reason },
        }
    }
}

/// The full catalog. Order within a category is the display order.
fn examples() -> Vec<Example> {
    use Category::{App, Codes, Components, Data, Effects, Interaction, Media, Visual};
    vec![
        // -- Codes --
        Example::embed(
            Codes,
            "Barcode",
            "QR codes rendered as a scannable view",
            || AnyView::new(barcode_example::demo()),
        ),
        // -- Data --
        Example::embed(
            Data,
            "Charts",
            "Bars, lines, pies and more across many chart types",
            || AnyView::new(chart_example::demo()),
        ),
        Example::standalone(
            Data,
            "Map",
            "Interactive map with zoom controls",
            "map-example",
            "Renders a native platform map, so it needs a backend with map support.",
        ),
        // -- Effects --
        Example::embed(
            Effects,
            "Particles",
            "GPU particle systems: rain, snow, fireworks, confetti",
            || AnyView::new(particle_example::demo()),
        ),
        Example::embed(
            Effects,
            "Starfield",
            "Procedural GPU starfield animation",
            || AnyView::new(star_field_example::demo()),
        ),
        // -- Media --
        Example::standalone(
            Media,
            "Audio Visualizer",
            "Real-time waveform visualization",
            "audio-visualizer-example",
            "Needs live microphone input.",
        ),
        Example::standalone(
            Media,
            "Media Picker",
            "Photo, video, and live-photo picking",
            "media-picker-example",
            "Opens the native system media picker.",
        ),
        Example::standalone(
            Media,
            "Video Player",
            "Source switching with buffering status",
            "video-player-example",
            "Streams sample video over the network. Add `--features rust-fallback` for the self-rendered HDR pipeline.",
        ),
        Example::standalone(
            Media,
            "Camera Filters",
            "Live camera capture through a GPU filter pipeline",
            "waterkit-camera-filters-example",
            "Needs camera access for the live preview.",
        ),
        // -- App showcases --
        Example::embed(App, "Markdown", "Render a Markdown document", || {
            AnyView::new(markdown_example::demo())
        }),
        Example::embed(
            App,
            "Localization",
            "Live locale switching with plurals, date and unit formatting",
            || AnyView::new(locale_example::demo()),
        ),
        Example::embed(
            App,
            "Reminders",
            "A reminders app with sidebar and lists",
            || AnyView::new(reminders_example::demo()),
        ),
        Example::embed(
            App,
            "Streaming Markdown",
            "FlowMarkdown with token-by-token streaming animation",
            || AnyView::new(flow_markdown_e2e_example::demo()),
        ),
        Example::standalone(
            App,
            "Multi-Window",
            "Several window styles and background effects",
            "multi-window-example",
            "Opens additional OS windows, so it runs as its own app.",
        ),
        Example::standalone(
            App,
            "WebView",
            "Browser chrome with navigation and JavaScript",
            "webview-example",
            "Embeds a native WebView; run on a backend that installs a WebView controller.",
        ),
        Example::standalone(
            App,
            "Stress",
            "High-pressure profiling workload",
            "stress-example",
            "A profiling workload; run standalone so it does not throttle the gallery.",
        ),
        // -- Components --
        Example::embed(
            Components,
            "List",
            "Static and dynamic lists",
            || AnyView::new(list_example::demo()),
        ),
        Example::embed(
            Components,
            "Menu",
            "Popup, nested, and context menus",
            || AnyView::new(menu_example::demo()),
        ),
        Example::embed(
            Components,
            "Navigation",
            "Typed, route-driven navigation stack",
            || AnyView::new(navigation_example::demo()),
        ),
        Example::embed(
            Components,
            "Picker",
            "Text, date, color, file, and multi-date pickers",
            || AnyView::new(picker_example::demo()),
        ),
        Example::embed(
            Components,
            "Snackbar",
            "Queued snackbars with positioning",
            || AnyView::new(snackbar_example::demo()),
        ),
        Example::embed(
            Components,
            "Form",
            "Auto-generated forms via the #[form] derive",
            || AnyView::new(form_example::demo()),
        ),
        // -- Interaction --
        Example::embed(
            Interaction,
            "Animation",
            "Scale, rotation, translation, and spring curves",
            || AnyView::new(animation_example::demo()),
        ),
        Example::embed(
            Interaction,
            "Drag & Drop",
            "Draggable cards into a drop zone",
            || AnyView::new(drag_drop_example::demo()),
        ),
        Example::embed(
            Interaction,
            "Gestures",
            "Tap, double-tap, long-press, and drag",
            || AnyView::new(gesture_example::demo()),
        ),
        Example::embed(
            Interaction,
            "Hover & Cursor",
            "Hover events, cursor styles, and reactive backgrounds",
            || AnyView::new(hover_example::demo()),
        ),
        // -- Visual --
        Example::embed(
            Visual,
            "Filters",
            "Blur, brightness, saturation, contrast, hue, grayscale",
            || AnyView::new(filter_example::demo()),
        ),
        Example::embed(
            Visual,
            "Flame (HDR)",
            "Cinematic HDR flame with bloom and ACES tonemap",
            || AnyView::new(flame_example::demo()),
        ),
        Example::embed(
            Visual,
            "Gradients",
            "Animated mesh gradient plus linear and radial",
            || AnyView::new(gradient_example::demo()),
        ),
        Example::embed(
            Visual,
            "Icons",
            "SF Symbols, Material Design, and Lucide icon packs",
            || AnyView::new(icons_example::demo()),
        ),
        Example::embed(
            Visual,
            "Images",
            "Image processing and remote photo loading with filters",
            || AnyView::new(image_example::demo()),
        ),
        Example::embed(
            Visual,
            "Shapes",
            "Circle, ellipse, capsule, rounded rect, and paths",
            || AnyView::new(shape_example::demo()),
        ),
    ]
}

/// Home screen: a header followed by one section per category.
fn home_content() -> impl View {
    let all = examples();
    let sections = Category::ALL
        .iter()
        .map(|&category| AnyView::new(category_section(&all, category)))
        .collect::<Vec<_>>();

    scroll(
        vstack((
            vstack((
                text("WaterUI Gallery").title(),
                text("Browse every example. Tap a row to open it.")
                    .body()
                    .foreground(MutedForeground),
            )),
            vstack(sections),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

/// One category section: heading, divider, and its rows.
fn category_section(all: &[Example], category: Category) -> impl View {
    let rows = all
        .iter()
        .enumerate()
        .filter(|(_, example)| example.category == category)
        .map(|(index, example)| AnyView::new(example_row(index, example)))
        .collect::<Vec<_>>();

    vstack((
        spacer_min(20.0),
        text(category.title()).sub_headline().foreground(Accent),
        Divider,
        vstack(rows),
    ))
}

/// A single tappable row: title link plus a one-line summary.
fn example_row(index: usize, example: &Example) -> impl View {
    let icon = match example.demo {
        Demo::Embed(_) => ">",
        Demo::Standalone { .. } => "↗",
    };

    vstack((
        NavigationLink::value(
            label(example.title).icon(text(icon)).trailing(),
            Route(index),
        ),
        text(example.summary).caption().foreground(MutedForeground),
    ))
    .padding_with(EdgeInsets::symmetric(8.0, 0.0))
}

/// Builds the detail screen for the example at `index`.
fn build_detail(index: usize) -> NavigationView {
    let all = examples();
    let example = &all[index];
    let content: AnyView = match &example.demo {
        Demo::Embed(build) => build(),
        Demo::Standalone { package, reason } => {
            AnyView::new(standalone_card(example.summary, package, reason))
        }
    };
    NavigationView::new(example.title, content)
}

/// Detail screen for an example that must run on its own.
fn standalone_card(summary: &'static str, package: &'static str, reason: &'static str) -> impl View {
    scroll(
        vstack((
            text(summary).body(),
            spacer_min(16.0),
            Divider,
            spacer_min(16.0),
            text("Runs standalone").sub_headline(),
            text(reason).body().foreground(MutedForeground),
            spacer_min(12.0),
            text!("water run -p {package}")
                .body()
                .foreground(Accent),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

/// Self-contained entry: the whole gallery as a navigation stack.
#[preview]
pub fn demo() -> impl View {
    NavigationStack::with(
        NavigationPath::new(),
        NavigationView::new("WaterUI Gallery", home_content()),
    )
    .destination(|Route(index)| build_detail(index))
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}

#[cfg(test)]
mod tests {
    use super::demo;
    use core::time::Duration;
    use waterui::env::Environment;
    use waterui_testing::{SemanticApp, ui};

    /// Mounts the gallery into the semantic test host with the Material 3 theme
    /// installed (Hydrolysis widgets require a theme in the environment).
    fn mount() -> SemanticApp {
        let mut env = Environment::new();
        hydrolysis_m3::install(&mut env);
        ui().environment(env).viewport(390, 900).mount(demo)
    }

    /// The home screen lists the title, category headings, and example titles.
    #[test]
    fn home_lists_catalog() {
        let mut app = mount();
        app.query().label("WaterUI Gallery").assert_exists();
        for category in ["Codes", "Components", "Interaction", "Visual", "Media"] {
            app.query().label(category).assert_exists();
        }
        for title in ["Barcode", "Hover & Cursor", "Gradients", "Video Player"] {
            app.query().label_contains(title).assert_exists();
        }
    }

    /// Tapping an embeddable example opens its live demo in place.
    #[test]
    fn embedded_example_opens_live() {
        let mut app = mount();
        assert!(
            app.query().label_contains("Hover & Cursor").tap(),
            "row should be tappable"
        );
        // "Cursor Styles" is a heading inside the embedded hover demo.
        assert!(
            app.query()
                .label_contains("Cursor Styles")
                .wait_for_existence(Duration::from_secs(3)),
            "embedded hover demo should appear after navigation"
        );
    }

    /// Tapping a standalone example shows the `water run` command instead.
    #[test]
    fn standalone_example_shows_run_command() {
        let mut app = mount();
        assert!(
            app.query().label_contains("Video Player").tap(),
            "row should be tappable"
        );
        assert!(
            app.query()
                .label_contains("water run -p video-player-example")
                .wait_for_existence(Duration::from_secs(3)),
            "standalone card should show the run command after navigation"
        );
    }
}
