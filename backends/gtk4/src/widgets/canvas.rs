//! Canvas widget rendering for GTK4 backend.

use gtk4::{DrawingArea, Widget, prelude::*};
use glib::clone;
use waterui_canvas::{CanvasView, Drawable};
use std::sync::Arc;

/// Renders a canvas view as a GTK4 DrawingArea widget.
pub fn render_canvas<T: Drawable + Clone + 'static>(canvas: CanvasView<T>) -> Widget {
    let drawing_area = DrawingArea::new();
    
    // Get canvas dimensions and content
    let (width, height) = canvas.dimensions();
    let content = Arc::new(canvas.content().clone());
    
    // Set the preferred size for the drawing area
    drawing_area.set_content_width(width as i32);
    drawing_area.set_content_height(height as i32);
    
    // Set up the drawing callback
    drawing_area.set_draw_func(clone!(
        #[strong]
        content,
        move |_, cr, width, height| {
        // Clear the background with white
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.paint().expect("Failed to paint background");
        
        // For now, we'll render using Cairo instead of Vello since GTK4 uses Cairo
        // In a full implementation, you'd want to integrate Vello with Cairo or use
        // a different approach like rendering to a texture
        render_canvas_content_with_cairo(content.as_ref(), cr, width as f32, height as f32);
        }
    ));
    
    drawing_area.upcast()
}

/// Renders canvas content using Cairo (simplified implementation).
/// In a production setup, you'd want to convert Vello paths to Cairo paths properly.
fn render_canvas_content_with_cairo<T: Drawable>(
    content: &T, 
    cr: &gtk4::cairo::Context, 
    _width: f32, 
    _height: f32
) {
    // For now, we'll create a simple Vello scene and try to extract drawing operations
    // This is a simplified implementation - a full implementation would need proper
    // Vello-to-Cairo conversion or render to a texture that can be displayed
    
    let mut scene = vello::Scene::new();
    content.draw(&mut scene);
    
    // TODO: Implement proper Vello scene to Cairo conversion
    // For now, just render a placeholder
    cr.set_source_rgb(0.5, 0.5, 0.5);
    cr.set_line_width(2.0);
    cr.rectangle(10.0, 10.0, 100.0, 50.0);
    cr.stroke().expect("Failed to draw placeholder rectangle");
    
    // Add text to indicate this is a canvas placeholder
    cr.set_source_rgb(0.0, 0.0, 0.0);
    cr.move_to(10.0, 30.0);
    cr.show_text("Canvas (Vello integration pending)").expect("Failed to draw text");
}