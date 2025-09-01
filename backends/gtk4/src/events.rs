//! Event handling for GTK4 backend.

use gtk4::{Entry, Switch, prelude::*};
use waterui::{Binding, Signal, Str};

/// Set up basic event handlers for GTK4 widgets.
///
/// This is a placeholder implementation that would be expanded
/// when the full event handler system is implemented.
pub fn setup_basic_events() {
    // This would set up global event handlers, window management, etc.
    // TODO: Implement global event handler setup when needed
    // For now, individual widget bindings are handled in their render functions
}

/// Connect a GTK4 Entry to a WaterUI text binding for two-way data flow.
///
/// This is a placeholder that would implement reactive binding
/// when the binding API is fully worked out.
pub fn connect_text_binding(entry: &Entry, binding: &Binding<Str>) {
    // Set up two-way binding between GTK4 Entry and WaterUI Binding

    // Update GTK when binding changes
    let entry_weak = entry.downgrade();
    let _watcher = binding.watch(move |value| {
        if let Some(entry) = entry_weak.upgrade() {
            entry.set_text(&value.value);
        }
    });

    // Update binding when GTK Entry changes
    let binding_for_signal = binding.clone();
    entry.connect_changed(move |entry| {
        let text = entry.text().to_string();
        binding_for_signal.set(Str::from(text));
    });

    // TODO: Store watcher to prevent dropping - this requires proper lifetime management
}

/// Connect a GTK4 Switch to a WaterUI boolean binding.
///
/// This is a placeholder that would implement reactive binding
/// when the binding API is fully worked out.
pub fn connect_bool_binding(switch: &Switch, binding: &Binding<bool>) {
    // Set up two-way binding between GTK4 Switch and WaterUI Binding

    // Update GTK when binding changes
    let switch_weak = switch.downgrade();
    let _watcher = binding.watch(move |value| {
        if let Some(switch) = switch_weak.upgrade() {
            switch.set_active(value.value);
        }
    });

    // Update binding when GTK Switch changes
    let binding_for_signal = binding.clone();
    switch.connect_active_notify(move |switch| {
        let active = switch.is_active();
        binding_for_signal.set(active);
    });

    // TODO: Store watcher to prevent dropping - this requires proper lifetime management
}
