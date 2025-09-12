//! Media widget implementations for the web backend.

use crate::element::WebElement;

/// Configuration for image rendering.
#[derive(Debug, Clone)]
pub struct ImageConfig {
    pub src: String,
    pub alt: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub fit: ImageFit,
}

/// Image fit modes.
#[derive(Debug, Clone)]
pub enum ImageFit {
    Fill,
    Contain,
    Cover,
    ScaleDown,
    None,
}

/// Render an image element.
pub fn render_image(config: ImageConfig) -> WebElement {
    let img = WebElement::create("img").expect("Failed to create img element");

    // Set source URL
    let _ = img.set_attribute("src", &config.src);

    // Set alt text
    if let Some(alt) = &config.alt {
        let _ = img.set_attribute("alt", alt);
    }

    // Set object-fit based on ImageFit
    let object_fit = match config.fit {
        ImageFit::Fill => "fill",
        ImageFit::Contain => "contain",
        ImageFit::Cover => "cover",
        ImageFit::ScaleDown => "scale-down",
        ImageFit::None => "none",
    };
    let _ = img.set_style("object-fit", object_fit);

    // Set dimensions using individual style calls if specified
    if let Some(width) = config.width {
        let _ = img.set_style("width", &format!("{}px", width));
    }

    if let Some(height) = config.height {
        let _ = img.set_style("height", &format!("{}px", height));
    }

    img
}

/// Render a video element.
pub fn render_video(src: &str, controls: bool, autoplay: bool, muted: bool) -> WebElement {
    let video = WebElement::create("video").expect("Failed to create video element");

    let _ = video.set_attribute("src", src);

    if controls {
        let _ = video.set_attribute("controls", "true");
    }

    if autoplay {
        let _ = video.set_attribute("autoplay", "true");
    }

    if muted {
        let _ = video.set_attribute("muted", "true");
    }

    let _ = video.set_style("max-width", "100%");
    let _ = video.set_style("height", "auto");

    video
}

/// Render an audio element.
pub fn render_audio(src: &str, controls: bool, autoplay: bool) -> WebElement {
    let audio = WebElement::create("audio").expect("Failed to create audio element");

    let _ = audio.set_attribute("src", src);

    if controls {
        let _ = audio.set_attribute("controls", "true");
    }

    if autoplay {
        let _ = audio.set_attribute("autoplay", "true");
    }

    audio
}

/// Render a canvas element for drawing.
pub fn render_canvas(width: f64, height: f64) -> WebElement {
    let canvas = WebElement::create("canvas").expect("Failed to create canvas element");

    let _ = canvas.set_attribute("width", &width.to_string());
    let _ = canvas.set_attribute("height", &height.to_string());
    let _ = canvas.set_style("border", "1px solid #ccc");

    canvas
}

/// Render an SVG container.
pub fn render_svg(width: f64, height: f64, viewbox: Option<&str>) -> WebElement {
    // Note: This might not work correctly as WebElement expects HTML elements
    // SVG would need special handling in a real implementation
    let svg = WebElement::create("svg").expect("Failed to create svg element");

    let _ = svg.set_attribute("width", &width.to_string());
    let _ = svg.set_attribute("height", &height.to_string());

    if let Some(vb) = viewbox {
        let _ = svg.set_attribute("viewBox", vb);
    }

    // Add SVG namespace (this might not work correctly without proper SVG support)
    let _ = svg.set_attribute("xmlns", "http://www.w3.org/2000/svg");

    svg
}

/// Render an iframe for embedding external content.
pub fn render_iframe(src: &str, width: Option<f64>, height: Option<f64>) -> WebElement {
    let iframe = WebElement::create("iframe").expect("Failed to create iframe element");

    let _ = iframe.set_attribute("src", src);
    let _ = iframe.set_attribute("frameborder", "0");

    // Set dimensions using individual style calls
    if let Some(w) = width {
        let _ = iframe.set_style("width", &format!("{}px", w));
    } else {
        let _ = iframe.set_style("width", "100%");
    }

    if let Some(h) = height {
        let _ = iframe.set_style("height", &format!("{}px", h));
    } else {
        let _ = iframe.set_style("height", "300px");
    }

    iframe
}
