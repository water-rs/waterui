# Accessibility

Building accessible applications is crucial for inclusive user experiences. WaterUI provides comprehensive accessibility features that work seamlessly with screen readers, keyboard navigation, and assistive technologies.

## Accessibility Fundamentals

### Semantic Structure

```rust,ignore
use waterui::*;
use nami::*;

fn accessible_structure_demo() -> impl View {
    vstack((
        // Proper heading hierarchy
        text("Main Application Title")
            .accessibility_role(AccessibilityRole::Heading)
            .accessibility_level(1)
            .font_size(24.0),
            
        text("Dashboard Section")
            .accessibility_role(AccessibilityRole::Heading)
            .accessibility_level(2)
            .font_size(20.0),
            
        text("User Statistics")
            .accessibility_role(AccessibilityRole::Heading)
            .accessibility_level(3)
            .font_size(16.0),
            
        // Landmark regions
        vstack((
            navigation_section(),
            main_content_section(),
            complementary_section(),
        ))
        .accessibility_role(AccessibilityRole::Main),
    ))
    .spacing(20.0)
    .padding(20.0)
}

fn navigation_section() -> impl View {
    hstack((
        link_button("Home", "/"),
        link_button("Dashboard", "/dashboard"),
        link_button("Profile", "/profile"),
        link_button("Settings", "/settings"),
    ))
    .spacing(15.0)
    .accessibility_role(AccessibilityRole::Navigation)
    .accessibility_label("Main navigation")
}

fn main_content_section() -> impl View {
    vstack((
        text("Welcome to your dashboard")
            .accessibility_role(AccessibilityRole::Heading)
            .accessibility_level(2),
            
        article_content(),
        data_table(),
    ))
    .spacing(20.0)
    .accessibility_role(AccessibilityRole::Main)
    .accessibility_label("Dashboard content")
}

fn article_content() -> impl View {
    vstack((
        text("Recent Activity")
            .accessibility_role(AccessibilityRole::Heading)
            .accessibility_level(3),
            
        text("You have completed 5 tasks this week. Great progress!")
            .accessibility_role(AccessibilityRole::Article),
    ))
    .spacing(10.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(8.0)
}

fn complementary_section() -> impl View {
    vstack((
        text("Quick Actions")
            .accessibility_role(AccessibilityRole::Heading)
            .accessibility_level(3),
            
        vstack((
            accessible_button("Create New Task", "Creates a new task in your task list"),
            accessible_button("View Reports", "Opens the reports dashboard"),
            accessible_button("Manage Team", "Access team management features"),
        ))
        .spacing(8.0),
    ))
    .spacing(10.0)
    .accessibility_role(AccessibilityRole::Complementary)
    .accessibility_label("Quick actions sidebar")
}

fn accessible_button(label: &str, description: &str) -> impl View {
    button(label)
        .accessibility_label(label)
        .accessibility_hint(description)
        .action(move |_| {
            println!("Button pressed: {}", label);
        })
}
```

### Form Accessibility

```rust,ignore
fn accessible_form_demo() -> impl View {
    let first_name = binding(String::new());
    let last_name = binding(String::new());
    let email = binding(String::new());
    let password = binding(String::new());
    let newsletter = binding(false);
    let country = binding("US".to_string());
    
    // Validation states
    let email_valid = s!(validate_email(&email));
    let password_valid = s!(validate_password(&password));
    let form_valid = s!(
        !first_name.trim().is_empty() &&
        !last_name.trim().is_empty() &&
        email_valid &&
        password_valid
    );
    
    vstack((
        text("Create Account")
            .accessibility_role(AccessibilityRole::Heading)
            .accessibility_level(1)
            .font_size(24.0),
            
        form((
            fieldset((
                legend("Personal Information"),
                
                accessible_text_field(
                    first_name.clone(),
                    "first-name",
                    "First Name",
                    "Enter your first name",
                    true,
                    None
                ),
                
                accessible_text_field(
                    last_name.clone(),
                    "last-name", 
                    "Last Name",
                    "Enter your last name",
                    true,
                    None
                ),
            ))
            .spacing(15.0),
            
            fieldset((
                legend("Account Details"),
                
                accessible_text_field(
                    email.clone(),
                    "email",
                    "Email Address",
                    "Enter your email address",
                    true,
                    s!(if email_valid { None } else { Some("Please enter a valid email address".to_string()) })
                ),
                
                accessible_password_field(
                    password.clone(),
                    "password",
                    "Password",
                    "Enter a secure password (minimum 8 characters)",
                    true,
                    s!(if password_valid { None } else { Some("Password must be at least 8 characters with uppercase, lowercase, and number".to_string()) })
                ),
                
                accessible_select(
                    country.clone(),
                    "country",
                    "Country",
                    "Select your country",
                    true,
                    vec![
                        ("US".to_string(), "United States"),
                        ("CA".to_string(), "Canada"),
                        ("UK".to_string(), "United Kingdom"),
                        ("DE".to_string(), "Germany"),
                        ("FR".to_string(), "France"),
                    ]
                ),
            ))
            .spacing(15.0),
            
            fieldset((
                legend("Preferences"),
                
                accessible_checkbox(
                    newsletter.clone(),
                    "newsletter",
                    "Subscribe to newsletter",
                    "Receive weekly updates and news about our products",
                    false
                ),
            )),
            
            // Form submission
            button("Create Account")
                .disabled(!form_valid.get())
                .accessibility_hint(s!(if form_valid {
                    "Submit the registration form".to_string()
                } else {
                    "Please fill in all required fields correctly before submitting".to_string()
                }))
                .action(move |_| {
                    println!("Form submitted");
                }),
        ))
        .spacing(20.0)
        .accessibility_role(AccessibilityRole::Form)
        .accessibility_label("Account registration form"),
    ))
    .spacing(20.0)
    .padding(20.0)
}

fn accessible_text_field(
    binding: Binding<String>,
    id: &str,
    label: &str,
    description: &str,
    required: bool,
    error: Signal<Option<String>>
) -> impl View {
    vstack((
        text!(label)
            .accessibility_role(AccessibilityRole::Label)
            .accessibility_for(id)
            .font_weight(FontWeight::Medium),
            
        text_field(binding.clone())
            .accessibility_id(id)
            .accessibility_label(label)
            .accessibility_hint(description)
            .accessibility_required(required)
            .accessibility_invalid(s!(error.is_some()))
            .accessibility_described_by(s!(if error.is_some() { 
                Some(format!("{}-error", id)) 
            } else { 
                None 
            }))
            .placeholder(if required { 
                format!("{} *", description) 
            } else { 
                description.to_string() 
            }),
            
        s!(if let Some(err) = &error {
            Some(
                text!("{}", err)
                    .color(Color::red())
                    .font_size(14.0)
                    .accessibility_id(&format!("{}-error", id))
                    .accessibility_role(AccessibilityRole::Alert)
                    .accessibility_live_region(LiveRegion::Polite)
            )
        } else {
            None
        }),
    ))
    .spacing(5.0)
}

fn accessible_password_field(
    binding: Binding<String>,
    id: &str,
    label: &str,
    description: &str,
    required: bool,
    error: Signal<Option<String>>
) -> impl View {
    vstack((
        text!(label)
            .accessibility_role(AccessibilityRole::Label)
            .accessibility_for(id)
            .font_weight(FontWeight::Medium),
            
        secure_field(binding.clone())
            .accessibility_id(id)
            .accessibility_label(label)
            .accessibility_hint(description)
            .accessibility_required(required)
            .accessibility_invalid(s!(error.is_some()))
            .placeholder(if required { 
                format!("{} *", description) 
            } else { 
                description.to_string() 
            }),
            
        password_strength_indicator(binding.clone()),
            
        s!(if let Some(err) = &error {
            Some(
                text!("{}", err)
                    .color(Color::red())
                    .font_size(14.0)
                    .accessibility_role(AccessibilityRole::Alert)
                    .accessibility_live_region(LiveRegion::Polite)
            )
        } else {
            None
        }),
    ))
    .spacing(5.0)
}

fn password_strength_indicator(password: Binding<String>) -> impl View {
    let strength = s!(calculate_password_strength(&password));
    
    hstack((
        text("Password strength:"),
        progress_bar(s!(strength.score / 4.0))
            .width(100.0)
            .accessibility_label("Password strength indicator")
            .accessibility_value(s!(format!("{} out of 4", strength.score))),
        text!(s!(strength.label.clone()))
            .color(s!(match strength.score {
                0..=1 => Color::red(),
                2 => Color::orange(),
                3 => Color::yellow(),
                4 => Color::green(),
                _ => Color::gray(),
            })),
    ))
    .spacing(10.0)
}

#[derive(Clone)]
struct PasswordStrength {
    score: i32,
    label: String,
}

fn calculate_password_strength(password: &str) -> PasswordStrength {
    let mut score = 0;
    
    if password.len() >= 8 { score += 1; }
    if password.chars().any(|c| c.is_uppercase()) { score += 1; }
    if password.chars().any(|c| c.is_lowercase()) { score += 1; }
    if password.chars().any(|c| c.is_numeric()) { score += 1; }
    if password.chars().any(|c| !c.is_alphanumeric()) { score += 1; }
    
    let label = match score {
        0..=1 => "Very Weak",
        2 => "Weak",
        3 => "Moderate",
        4 => "Strong",
        5 => "Very Strong",
        _ => "Unknown",
    }.to_string();
    
    PasswordStrength { 
        score: score.min(4), 
        label 
    }
}

fn accessible_select(
    binding: Binding<String>,
    id: &str,
    label: &str,
    description: &str,
    required: bool,
    options: Vec<(String, &str)>
) -> impl View {
    vstack((
        text!(label)
            .accessibility_role(AccessibilityRole::Label)
            .accessibility_for(id)
            .font_weight(FontWeight::Medium),
            
        picker(binding, options)
            .accessibility_id(id)
            .accessibility_label(label)
            .accessibility_hint(description)
            .accessibility_required(required),
    ))
    .spacing(5.0)
}

fn accessible_checkbox(
    binding: Binding<bool>,
    id: &str,
    label: &str,
    description: &str,
    required: bool
) -> impl View {
    hstack((
        checkbox(binding.clone())
            .accessibility_id(id)
            .accessibility_label(label)
            .accessibility_hint(description)
            .accessibility_required(required),
            
        text!(label)
            .accessibility_role(AccessibilityRole::Label)
            .accessibility_for(id)
            .on_tap({
                let binding = binding.clone();
                move |_| binding.update(|checked| !checked)
            }),
    ))
    .spacing(10.0)
}

fn validate_email(email: &str) -> bool {
    email.contains('@') && email.contains('.') && email.len() > 5
}

fn validate_password(password: &str) -> bool {
    password.len() >= 8 &&
    password.chars().any(|c| c.is_uppercase()) &&
    password.chars().any(|c| c.is_lowercase()) &&
    password.chars().any(|c| c.is_numeric())
}
```

## Keyboard Navigation

### Focus Management

```rust,ignore
fn keyboard_navigation_demo() -> impl View {
    let current_focus = binding(None::<String>);
    let focus_visible = binding(false);
    
    vstack((
        text("Keyboard Navigation Demo")
            .accessibility_role(AccessibilityRole::Heading)
            .accessibility_level(1)
            .font_size(20.0),
            
        text!("Currently focused: {}", s!(current_focus.clone().unwrap_or("None".to_string())))
            .accessibility_live_region(LiveRegion::Polite),
            
        // Focus trap example
        focus_trap((
            text("Focus Trap Area")
                .accessibility_role(AccessibilityRole::Heading)
                .accessibility_level(2),
                
            focus_group((
                focusable_button("Button 1", "button-1", current_focus.clone()),
                focusable_button("Button 2", "button-2", current_focus.clone()),
                focusable_button("Button 3", "button-3", current_focus.clone()),
            ))
            .orientation(FocusOrientation::Horizontal)
            .wrap(true),
            
            focus_group((
                focusable_input("Input 1", "input-1", current_focus.clone()),
                focusable_input("Input 2", "input-2", current_focus.clone()),
            ))
            .orientation(FocusOrientation::Vertical),
        ))
        .padding(20.0)
        .border_width(2.0)
        .border_color(s!(if focus_visible { Color::primary() } else { Color::light_gray() })),
        
        // Skip links
        skip_links(),
        
        // Keyboard shortcuts
        keyboard_shortcuts_help(),
    ))
    .spacing(20.0)
    .padding(20.0)
    .on_key_down(handle_global_keyboard_shortcuts)
}

fn focusable_button(label: &str, id: &str, current_focus: Binding<Option<String>>) -> impl View {
    let id_owned = id.to_string();
    
    button(label)
        .accessibility_id(id)
        .accessibility_hint(&format!("Press Enter or Space to activate {}", label))
        .focusable(true)
        .on_focus({
            let current_focus = current_focus.clone();
            let id = id_owned.clone();
            move || {
                current_focus.set(Some(id.clone()));
            }
        })
        .on_blur({
            let current_focus = current_focus.clone();
            move || {
                current_focus.set(None);
            }
        })
        .action(move |_| {
            println!("Activated: {}", label);
        })
}

fn focusable_input(placeholder: &str, id: &str, current_focus: Binding<Option<String>>) -> impl View {
    let id_owned = id.to_string();
    let value = binding(String::new());
    
    text_field(value)
        .accessibility_id(id)
        .accessibility_label(placeholder)
        .placeholder(placeholder)
        .focusable(true)
        .on_focus({
            let current_focus = current_focus.clone();
            let id = id_owned.clone();
            move || {
                current_focus.set(Some(id.clone()));
            }
        })
        .on_blur({
            let current_focus = current_focus.clone();
            move || {
                current_focus.set(None);
            }
        })
}

fn skip_links() -> impl View {
    hstack((
        skip_link("Skip to main content", "#main"),
        skip_link("Skip to navigation", "#nav"),
        skip_link("Skip to footer", "#footer"),
    ))
    .spacing(10.0)
    .accessibility_role(AccessibilityRole::Navigation)
    .accessibility_label("Skip links")
}

fn skip_link(text: &str, target: &str) -> impl View {
    button(text)
        .accessibility_hint(&format!("Navigate to {}", target))
        .focusable(true)
        .visibility(FocusVisibility::FocusOnly) // Only visible when focused
        .action(move |_| {
            // Focus the target element
            focus_element(target);
        })
}

fn focus_element(target: &str) {
    // Implementation would focus the element with the given ID
    println!("Focusing element: {}", target);
}

fn keyboard_shortcuts_help() -> impl View {
    let shortcuts_visible = binding(false);
    
    vstack((
        button(s!(if shortcuts_visible { "Hide Shortcuts" } else { "Show Keyboard Shortcuts" }))
            .accessibility_hint("Toggle keyboard shortcuts help")
            .keyboard_shortcut(KeyboardShortcut::new(Key::F1))
            .action({
                let shortcuts_visible = shortcuts_visible.clone();
                move |_| {
                    shortcuts_visible.update(|visible| !visible);
                }
            }),
            
        s!(if shortcuts_visible {
            Some(shortcuts_panel())
        } else {
            None
        }),
    ))
    .spacing(10.0)
}

fn shortcuts_panel() -> impl View {
    vstack((
        text("Keyboard Shortcuts")
            .accessibility_role(AccessibilityRole::Heading)
            .accessibility_level(3)
            .font_weight(FontWeight::Bold),
            
        shortcut_item("Tab", "Move to next focusable element"),
        shortcut_item("Shift + Tab", "Move to previous focusable element"),
        shortcut_item("Enter / Space", "Activate button or link"),
        shortcut_item("Arrow Keys", "Navigate within groups"),
        shortcut_item("Escape", "Close dialog or cancel operation"),
        shortcut_item("F1", "Toggle this help panel"),
        shortcut_item("Ctrl/Cmd + S", "Save current work"),
        shortcut_item("Ctrl/Cmd + Z", "Undo last action"),
    ))
    .spacing(8.0)
    .padding(15.0)
    .background(Color::surface())
    .border_width(1.0)
    .border_color(Color::border())
    .corner_radius(8.0)
    .accessibility_role(AccessibilityRole::Dialog)
    .accessibility_label("Keyboard shortcuts help")
}

fn shortcut_item(keys: &str, description: &str) -> impl View {
    hstack((
        text!(keys)
            .font_family("monospace")
            .padding(EdgeInsets::symmetric(6.0, 4.0))
            .background(Color::light_gray())
            .corner_radius(4.0)
            .min_width(100.0),
            
        text!(description)
            .flex(1),
    ))
    .spacing(10.0)
}

fn handle_global_keyboard_shortcuts(key_event: KeyEvent) {
    match key_event {
        KeyEvent { 
            key: Key::S, 
            modifiers: KeyModifiers::CMD, 
            .. 
        } => {
            println!("Save shortcut pressed");
            // Handle save
        },
        KeyEvent { 
            key: Key::Z, 
            modifiers: KeyModifiers::CMD, 
            .. 
        } => {
            println!("Undo shortcut pressed");
            // Handle undo
        },
        KeyEvent { 
            key: Key::F1, 
            .. 
        } => {
            println!("Help shortcut pressed");
            // Toggle help
        },
        _ => {}
    }
}
```

## Screen Reader Support

### ARIA Attributes and Live Regions

```rust,ignore
fn screen_reader_demo() -> impl View {
    let notification_count = binding(0);
    let loading_state = binding(false);
    let progress = binding(0.0);
    let status_message = binding(String::new());
    
    vstack((
        text("Screen Reader Demo")
            .accessibility_role(AccessibilityRole::Heading)
            .accessibility_level(1)
            .font_size(20.0),
            
        // Live regions for dynamic content
        live_region_examples(notification_count.clone(), status_message.clone()),
        
        // Progress indicators
        progress_section(loading_state.clone(), progress.clone()),
        
        // Complex widgets
        custom_widget_example(),
        
        // Interactive controls
        interactive_controls(
            notification_count.clone(),
            loading_state.clone(),
            progress.clone(),
            status_message.clone()
        ),
    ))
    .spacing(20.0)
    .padding(20.0)
}

fn live_region_examples(
    notification_count: Binding<i32>,
    status_message: Binding<String>
) -> impl View {
    vstack((
        text("Live Regions").font_size(16.0),
        
        // Polite live region - doesn't interrupt
        text!("Notifications: {}", notification_count)
            .accessibility_live_region(LiveRegion::Polite)
            .accessibility_label("Notification count"),
            
        // Assertive live region - interrupts screen reader
        text!("{}", status_message)
            .accessibility_live_region(LiveRegion::Assertive)
            .accessibility_role(AccessibilityRole::Alert)
            .color(s!(if status_message.contains("Error") { 
                Color::red() 
            } else { 
                Color::text() 
            })),
            
        // Status region for non-urgent updates
        text("System ready")
            .accessibility_live_region(LiveRegion::Status)
            .accessibility_label("System status"),
    ))
    .spacing(10.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(8.0)
}

fn progress_section(
    loading_state: Binding<bool>,
    progress: Binding<f64>
) -> impl View {
    vstack((
        text("Progress Indicators").font_size(16.0),
        
        s!(if loading_state {
            Some(
                vstack((
                    progress_indicator()
                        .accessibility_label("Loading content")
                        .accessibility_hint("Please wait while content loads"),
                        
                    text("Loading content, please wait...")
                        .accessibility_live_region(LiveRegion::Polite),
                ))
                .spacing(10.0)
            )
        } else {
            None
        }),
        
        vstack((
            text!("Progress: {:.0}%", s!(progress * 100.0)),
            
            progress_bar(progress.clone())
                .accessibility_label("Task completion progress")
                .accessibility_value(s!(format!("{:.0} percent complete", progress * 100.0)))
                .accessibility_value_min(0.0)
                .accessibility_value_max(100.0)
                .accessibility_value_now(s!(progress * 100.0)),
        ))
        .spacing(8.0),
    ))
    .spacing(15.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(8.0)
}

fn custom_widget_example() -> impl View {
    let selected_item = binding(0);
    let items = vec!["Option A", "Option B", "Option C", "Option D"];
    
    vstack((
        text("Custom Widget Example").font_size(16.0),
        
        // Custom listbox widget
        vstack(
            items.into_iter().enumerate().map(|(index, item)| {
                listbox_option(
                    item.to_string(),
                    index,
                    selected_item.clone()
                )
            })
        )
        .accessibility_role(AccessibilityRole::ListBox)
        .accessibility_label("Options list")
        .accessibility_multi_selectable(false)
        .accessibility_orientation(AccessibilityOrientation::Vertical)
        .spacing(2.0)
        .border_width(1.0)
        .border_color(Color::border())
        .corner_radius(6.0),
        
        text!("Selected: {}", s!(match selected_item {
            0 => "Option A",
            1 => "Option B", 
            2 => "Option C",
            3 => "Option D",
            _ => "None",
        }))
        .accessibility_live_region(LiveRegion::Polite),
    ))
    .spacing(10.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(8.0)
}

fn listbox_option(
    label: String,
    index: usize,
    selected_item: Binding<usize>
) -> impl View {
    let is_selected = s!(selected_item == index);
    
    text!(label)
        .padding(8.0)
        .background(s!(if is_selected { 
            Color::primary().opacity(0.2) 
        } else { 
            Color::transparent() 
        }))
        .accessibility_role(AccessibilityRole::Option)
        .accessibility_label(&label)
        .accessibility_selected(is_selected.get())
        .focusable(true)
        .on_tap({
            let selected_item = selected_item.clone();
            move |_| {
                selected_item.set(index);
            }
        })
        .on_key_down({
            let selected_item = selected_item.clone();
            move |key| {
                match key.key {
                    Key::ArrowUp => {
                        if index > 0 {
                            selected_item.set(index - 1);
                        }
                    },
                    Key::ArrowDown => {
                        if index < 3 {
                            selected_item.set(index + 1);
                        }
                    },
                    Key::Enter | Key::Space => {
                        selected_item.set(index);
                    },
                    _ => {}
                }
            }
        })
}

fn interactive_controls(
    notification_count: Binding<i32>,
    loading_state: Binding<bool>,
    progress: Binding<f64>,
    status_message: Binding<String>
) -> impl View {
    vstack((
        text("Controls").font_size(16.0),
        
        hstack((
            button("Add Notification")
                .accessibility_hint("Increases notification count by 1")
                .action({
                    let notification_count = notification_count.clone();
                    let status_message = status_message.clone();
                    move |_| {
                        notification_count.update(|count| count + 1);
                        status_message.set(format!("Notification added. Total: {}", notification_count.get()));
                    }
                }),
                
            button("Clear Notifications")
                .accessibility_hint("Resets notification count to zero")
                .disabled(s!(notification_count == 0))
                .action({
                    let notification_count = notification_count.clone();
                    let status_message = status_message.clone();
                    move |_| {
                        notification_count.set(0);
                        status_message.set("All notifications cleared".to_string());
                    }
                }),
        ))
        .spacing(10.0),
        
        hstack((
            button(s!(if loading_state { "Stop Loading" } else { "Start Loading" }))
                .accessibility_hint(s!(if loading_state { 
                    "Stop the loading process".to_string() 
                } else { 
                    "Start a loading process".to_string() 
                }))
                .action({
                    let loading_state = loading_state.clone();
                    let status_message = status_message.clone();
                    move |_| {
                        let new_state = !loading_state.get();
                        loading_state.set(new_state);
                        status_message.set(if new_state {
                            "Loading started".to_string()
                        } else {
                            "Loading stopped".to_string()
                        });
                    }
                }),
                
            button("Increase Progress")
                .accessibility_hint("Increases progress by 10%")
                .disabled(s!(progress >= 1.0))
                .action({
                    let progress = progress.clone();
                    let status_message = status_message.clone();
                    move |_| {
                        progress.update(|p| (p + 0.1).min(1.0));
                        status_message.set(format!("Progress updated to {:.0}%", progress.get() * 100.0));
                    }
                }),
                
            button("Reset Progress")
                .accessibility_hint("Resets progress to 0%")
                .action({
                    let progress = progress.clone();
                    let status_message = status_message.clone();
                    move |_| {
                        progress.set(0.0);
                        status_message.set("Progress reset to 0%".to_string());
                    }
                }),
        ))
        .spacing(10.0),
    ))
    .spacing(15.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(8.0)
}
```

## Color and Contrast

### High Contrast and Color Accessibility

```rust,ignore
fn color_accessibility_demo() -> impl View {
    let high_contrast_mode = binding(false);
    let theme = s!(if high_contrast_mode { 
        AccessibleTheme::high_contrast() 
    } else { 
        AccessibleTheme::default() 
    });
    
    theme_provider(theme.clone(),
        vstack((
            text("Color and Contrast Demo")
                .accessibility_role(AccessibilityRole::Heading)
                .accessibility_level(1)
                .font_size(20.0)
                .color(s!(theme.text_primary)),
                
            toggle(high_contrast_mode.clone())
                .label("High Contrast Mode")
                .accessibility_hint("Toggle high contrast colors for better visibility"),
                
            color_contrast_examples(theme.clone()),
            semantic_colors_section(theme.clone()),
            interactive_elements_section(theme.clone()),
        ))
        .spacing(20.0)
        .padding(20.0)
        .background(s!(theme.background))
    )
}

#[derive(Clone)]
struct AccessibleTheme {
    background: Color,
    surface: Color,
    text_primary: Color,
    text_secondary: Color,
    primary: Color,
    secondary: Color,
    success: Color,
    warning: Color,
    error: Color,
    border: Color,
    focus: Color,
}

impl AccessibleTheme {
    fn default() -> Self {
        Self {
            background: Color::rgb(1.0, 1.0, 1.0),
            surface: Color::rgb(0.98, 0.98, 0.98),
            text_primary: Color::rgb(0.13, 0.13, 0.13),
            text_secondary: Color::rgb(0.46, 0.46, 0.46),
            primary: Color::rgb(0.0, 0.48, 1.0),
            secondary: Color::rgb(0.34, 0.34, 0.36),
            success: Color::rgb(0.2, 0.7, 0.2),
            warning: Color::rgb(0.9, 0.6, 0.0),
            error: Color::rgb(0.9, 0.2, 0.2),
            border: Color::rgb(0.9, 0.9, 0.9),
            focus: Color::rgb(0.0, 0.48, 1.0),
        }
    }
    
    fn high_contrast() -> Self {
        Self {
            background: Color::rgb(1.0, 1.0, 1.0),
            surface: Color::rgb(1.0, 1.0, 1.0),
            text_primary: Color::rgb(0.0, 0.0, 0.0),
            text_secondary: Color::rgb(0.0, 0.0, 0.0),
            primary: Color::rgb(0.0, 0.0, 1.0),
            secondary: Color::rgb(0.0, 0.0, 0.0),
            success: Color::rgb(0.0, 0.5, 0.0),
            warning: Color::rgb(0.8, 0.4, 0.0),
            error: Color::rgb(0.8, 0.0, 0.0),
            border: Color::rgb(0.0, 0.0, 0.0),
            focus: Color::rgb(1.0, 1.0, 0.0), // High contrast focus indicator
        }
    }
}

fn color_contrast_examples(theme: Signal<AccessibleTheme>) -> impl View {
    vstack((
        text("Color Contrast Examples")
            .accessibility_role(AccessibilityRole::Heading)
            .accessibility_level(2)
            .font_size(16.0)
            .color(s!(theme.text_primary)),
            
        // Good contrast examples
        contrast_example(
            "Good Contrast (AA)",
            s!(theme.text_primary),
            s!(theme.background),
            "21:1", // Approximate contrast ratio
            true
        ),
        
        contrast_example(
            "Good Contrast (AAA)",
            s!(theme.text_secondary),
            s!(theme.background),
            "7:1",
            true
        ),
        
        // Interactive element contrast
        hstack((
            button("Primary Button")
                .background_color(s!(theme.primary))
                .color(Color::white())
                .accessibility_hint("High contrast primary button"),
                
            button("Secondary Button")
                .background_color(s!(theme.secondary))
                .color(Color::white())
                .accessibility_hint("High contrast secondary button"),
        ))
        .spacing(10.0),
    ))
    .spacing(15.0)
    .padding(15.0)
    .background(s!(theme.surface))
    .border_width(1.0)
    .border_color(s!(theme.border))
    .corner_radius(8.0)
}

fn contrast_example(
    label: &str,
    foreground: Signal<Color>,
    background: Signal<Color>,
    ratio: &str,
    is_compliant: bool
) -> impl View {
    hstack((
        text!(label)
            .color(foreground)
            .padding(10.0)
            .background(background)
            .min_width(150.0),
            
        text!("Ratio: {}", ratio)
            .font_size(14.0),
            
        text(if is_compliant { "✓ WCAG AA" } else { "✗ Insufficient" })
            .color(if is_compliant { Color::green() } else { Color::red() })
            .font_size(14.0),
    ))
    .spacing(10.0)
}

fn semantic_colors_section(theme: Signal<AccessibleTheme>) -> impl View {
    vstack((
        text("Semantic Colors")
            .accessibility_role(AccessibilityRole::Heading)
            .accessibility_level(2)
            .font_size(16.0)
            .color(s!(theme.text_primary)),
            
        hstack((
            status_indicator("Success", s!(theme.success), "Operation completed successfully"),
            status_indicator("Warning", s!(theme.warning), "Attention required"),
            status_indicator("Error", s!(theme.error), "Critical issue detected"),
        ))
        .spacing(15.0),
    ))
    .spacing(10.0)
}

fn status_indicator(label: &str, color: Signal<Color>, description: &str) -> impl View {
    hstack((
        circle()
            .width(16.0)
            .height(16.0)
            .color(color)
            .accessibility_hidden(true), // Decorative, meaning conveyed by text
            
        vstack((
            text!(label)
                .font_weight(FontWeight::Medium)
                .color(color),
            text!(description)
                .font_size(12.0)
                .color(s!(theme.text_secondary)),
        ))
        .spacing(2.0),
    ))
    .spacing(8.0)
}

fn interactive_elements_section(theme: Signal<AccessibleTheme>) -> impl View {
    let button_focused = binding(false);
    let link_focused = binding(false);
    
    vstack((
        text("Interactive Elements")
            .accessibility_role(AccessibilityRole::Heading)
            .accessibility_level(2)
            .font_size(16.0)
            .color(s!(theme.text_primary)),
            
        // Focus indicators
        hstack((
            button("Focusable Button")
                .color(s!(theme.text_primary))
                .background_color(s!(theme.surface))
                .border_width(2.0)
                .border_color(s!(if button_focused { 
                    theme.focus 
                } else { 
                    theme.border 
                }))
                .on_focus({
                    let button_focused = button_focused.clone();
                    move || button_focused.set(true)
                })
                .on_blur({
                    let button_focused = button_focused.clone();
                    move || button_focused.set(false)
                })
                .accessibility_hint("Button with high contrast focus indicator"),
                
            link_button("Focusable Link", "#")
                .color(s!(theme.primary))
                .underline(true)
                .border_width(s!(if link_focused { 2.0 } else { 0.0 }))
                .border_color(s!(theme.focus))
                .on_focus({
                    let link_focused = link_focused.clone();
                    move || link_focused.set(true)
                })
                .on_blur({
                    let link_focused = link_focused.clone();
                    move || link_focused.set(false)
                })
                .accessibility_hint("Link with high contrast focus indicator"),
        ))
        .spacing(15.0),
    ))
    .spacing(10.0)
}
```

## Summary

WaterUI's accessibility features provide:

- **Semantic Structure**: Proper heading hierarchy, landmarks, and ARIA roles
- **Keyboard Navigation**: Focus management, skip links, and keyboard shortcuts
- **Screen Reader Support**: ARIA attributes, live regions, and descriptive labels
- **Form Accessibility**: Proper labeling, validation messages, and error handling
- **Color Accessibility**: High contrast themes, focus indicators, and semantic colors
- **Custom Widgets**: Accessible custom components with proper ARIA implementation

Key best practices:
- Use semantic HTML/ARIA roles and proper heading hierarchy
- Provide descriptive labels and hints for all interactive elements
- Implement proper focus management and keyboard navigation
- Ensure sufficient color contrast (WCAG AA: 4.5:1, AAA: 7:1)
- Test with screen readers and keyboard-only navigation
- Use live regions for dynamic content updates
- Provide alternative ways to access functionality
- Consider reduced motion and high contrast preferences

Next: [Internationalization](19-internationalization.md)