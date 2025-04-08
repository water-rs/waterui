# Icon Management for WaterUI

This crate provides icon management functionality for the WaterUI framework.
It allows for the creation, customization, and display of icons in a
no_std environment.

## Features

- Icon creation and configuration with customizable sizes and animations
- Icon management including aliasing and URL mapping
- Integration with WaterUI's component system
- Support for no_std environments

## Usage Example

```rust
use waterui_icon::{icon, IconManager, IconAnimation};
use waterui::Environment;

// Create an icon manager and register it with the environment
let mut manager = IconManager::new();
manager.insert("home".into(), "home_icon".into(), "assets/home.png".into());

// Create an environment and register the manager
let mut env = Environment::new();
env.register(manager);

// Use the icon in your UI
let home_icon = icon("home").animation(IconAnimation::Default);
```
