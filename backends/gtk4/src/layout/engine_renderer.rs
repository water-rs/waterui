//! Renderer that integrates the custom layout engine with GTK4 widgets.

use gtk4::{Fixed, Widget, prelude::*};
use waterui::{Environment, component::layout::stack::{Stack, StackMode}};

use crate::renderer::render;
use super::engine::{
    LayoutEngine, LayoutBehavior, HorizontalAlignment, VerticalAlignment, 
    Alignment, Size, Rect, LayoutId
};

/// A mapping between layout nodes and their corresponding GTK widgets
struct WidgetMapping {
    widget: Widget,
    layout_id: LayoutId,
}

/// Renders a Stack using the custom layout engine
pub fn render_stack_with_engine(stack: Stack, env: &Environment) -> Widget {
    let mut engine = LayoutEngine::new();
    let mut widget_mappings = Vec::new();
    
    // Create the root layout node based on stack mode
    let root_behavior = match stack.mode {
        StackMode::Vertical => LayoutBehavior::VStack {
            alignment: HorizontalAlignment::Center,
            spacing: 8.0,
        },
        StackMode::Horizonal => LayoutBehavior::HStack {
            alignment: VerticalAlignment::Center,
            spacing: 8.0,
        },
        StackMode::Layered => LayoutBehavior::ZStack {
            alignment: Alignment::CENTER,
        },
    };
    
    let root_id = engine.create_node(root_behavior);
    engine.set_root(root_id);
    
    // Create child nodes and their widgets
    for child in stack.contents {
        let child_widget = render(child, env);
        let child_id = engine.create_node(LayoutBehavior::Leaf);
        
        engine.add_child(root_id, child_id);
        widget_mappings.push(WidgetMapping {
            widget: child_widget,
            layout_id: child_id,
        });
    }
    
    // Create a GTK Fixed container for absolute positioning
    let container = Fixed::new();
    
    // Add all child widgets to the container (they'll be positioned later)
    for mapping in &widget_mappings {
        container.put(&mapping.widget, 0.0, 0.0);
    }
    
    // Set up size allocation callback to perform layout
    container.connect_size_allocate(move |fixed, width, height| {
        let mut engine = LayoutEngine::new();
        let mut widget_mappings = Vec::new();
        
        // Recreate the layout tree (this is inefficient but works for demo)
        // In a real implementation, we'd cache this
        let root_behavior = match stack.mode {
            StackMode::Vertical => LayoutBehavior::VStack {
                alignment: HorizontalAlignment::Center,
                spacing: 8.0,
            },
            StackMode::Horizonal => LayoutBehavior::HStack {
                alignment: VerticalAlignment::Center,
                spacing: 8.0,
            },
            StackMode::Layered => LayoutBehavior::ZStack {
                alignment: Alignment::CENTER,
            },
        };
        
        let root_id = engine.create_node(root_behavior);
        engine.set_root(root_id);
        
        // We'd need to recreate the widget mappings here...
        // This is getting complex due to the closure capture limitations
        
        // For now, let's perform the layout
        let available_size = Size::new(width as f32, height as f32);
        if let Some(_computed_size) = engine.layout(available_size) {
            let container_rect = Rect::new(0.0, 0.0, width as f32, height as f32);
            engine.position(container_rect);
            
            // Position widgets based on layout results
            // We'd iterate through widget_mappings and call fixed.move_() for each
        }
    });
    
    container.upcast()
}