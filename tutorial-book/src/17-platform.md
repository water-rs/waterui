# Platform Integration

WaterUI provides excellent platform integration capabilities, allowing applications to access native features and APIs across different platforms while maintaining a consistent codebase.

## Platform Detection

### Runtime Platform Detection

```rust,ignore
use waterui::*;
use nami::*;

#[derive(Clone, Debug, PartialEq)]
enum Platform {
    Web,
    Desktop(DesktopPlatform),
    Mobile(MobilePlatform),
}

#[derive(Clone, Debug, PartialEq)]
enum DesktopPlatform {
    Windows,
    MacOS,
    Linux,
}

#[derive(Clone, Debug, PartialEq)]
enum MobilePlatform {
    iOS,
    Android,
}

fn platform_detection_demo() -> impl View {
    let current_platform = binding(detect_platform());
    let platform_info = binding(get_platform_info());
    
    vstack((
        text("Platform Integration Demo").font_size(20.0),
        
        // Platform information
        platform_info_display(current_platform.clone(), platform_info.clone()),
        
        // Platform-specific features
        platform_features(current_platform.clone()),
        
        // Adaptive UI based on platform
        adaptive_ui(current_platform.clone()),
    ))
    .spacing(15.0)
    .padding(20.0)
}

fn detect_platform() -> Platform {
    #[cfg(target_arch = "wasm32")]
    {
        Platform::Web
    }
    
    #[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
    {
        Platform::Desktop(DesktopPlatform::Windows)
    }
    
    #[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
    {
        Platform::Desktop(DesktopPlatform::MacOS)
    }
    
    #[cfg(all(not(target_arch = "wasm32"), target_os = "linux"))]
    {
        Platform::Desktop(DesktopPlatform::Linux)
    }
    
    #[cfg(all(not(target_arch = "wasm32"), target_os = "ios"))]
    {
        Platform::Mobile(MobilePlatform::iOS)
    }
    
    #[cfg(all(not(target_arch = "wasm32"), target_os = "android"))]
    {
        Platform::Mobile(MobilePlatform::Android)
    }
    
    #[cfg(not(any(
        target_arch = "wasm32",
        target_os = "windows",
        target_os = "macos", 
        target_os = "linux",
        target_os = "ios",
        target_os = "android"
    )))]
    {
        Platform::Desktop(DesktopPlatform::Linux) // fallback
    }
}

#[derive(Clone, Debug)]
struct PlatformInfo {
    os_version: String,
    arch: String,
    user_agent: Option<String>,
    screen_size: (f64, f64),
    device_pixel_ratio: f64,
}

fn get_platform_info() -> PlatformInfo {
    #[cfg(target_arch = "wasm32")]
    {
        let window = web_sys::window().unwrap();
        let navigator = window.navigator();
        let screen = window.screen().unwrap();
        
        PlatformInfo {
            os_version: navigator.platform().unwrap_or_default(),
            arch: "wasm32".to_string(),
            user_agent: Some(navigator.user_agent().unwrap_or_default()),
            screen_size: (
                screen.width().unwrap_or_default() as f64,
                screen.height().unwrap_or_default() as f64,
            ),
            device_pixel_ratio: window.device_pixel_ratio(),
        }
    }
    
    #[cfg(not(target_arch = "wasm32"))]
    {
        use sysinfo::{System, SystemExt};
        let sys = System::new_all();
        
        PlatformInfo {
            os_version: format!("{} {}", 
                sys.name().unwrap_or_default(),
                sys.os_version().unwrap_or_default()
            ),
            arch: std::env::consts::ARCH.to_string(),
            user_agent: None,
            screen_size: get_screen_size(),
            device_pixel_ratio: 1.0,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn get_screen_size() -> (f64, f64) {
    // Platform-specific screen size detection
    #[cfg(target_os = "windows")]
    {
        use winapi::um::winuser::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        unsafe {
            (
                GetSystemMetrics(SM_CXSCREEN) as f64,
                GetSystemMetrics(SM_CYSCREEN) as f64,
            )
        }
    }
    
    #[cfg(target_os = "macos")]
    {
        // macOS screen size detection would go here
        (1920.0, 1080.0) // fallback
    }
    
    #[cfg(target_os = "linux")]
    {
        // Linux screen size detection would go here
        (1920.0, 1080.0) // fallback
    }
    
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        (1920.0, 1080.0) // fallback
    }
}

fn platform_info_display(
    platform: Binding<Platform>,
    info: Binding<PlatformInfo>
) -> impl View {
    vstack((
        text("Platform Information").font_size(16.0),
        
        info_row("Platform", s!(format!("{:?}", platform))),
        info_row("OS Version", s!(info.os_version.clone())),
        info_row("Architecture", s!(info.arch.clone())),
        info_row("Screen Size", s!(format!("{:.0}×{:.0}", info.screen_size.0, info.screen_size.1))),
        info_row("Device Pixel Ratio", s!(format!("{:.2}", info.device_pixel_ratio))),
        
        s!(if let Some(ua) = &info.user_agent {
            Some(info_row("User Agent", ua.clone()))
        } else {
            None
        }),
    ))
    .spacing(8.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(10.0)
}

fn info_row(label: &str, value: Signal<String>) -> impl View {
    hstack((
        text!(label)
            .font_weight(FontWeight::Medium)
            .min_width(120.0),
        text!("{}", value)
            .font_family("monospace")
            .color(Color::secondary()),
    ))
    .spacing(10.0)
}
```

## Native API Access

### File System Integration

```rust,ignore
fn file_system_integration() -> impl View {
    let selected_file = binding(None::<String>);
    let file_content = binding(String::new());
    let save_status = binding(String::new());
    
    vstack((
        text("File System Integration").font_size(16.0),
        
        // File operations
        file_operations(selected_file.clone(), file_content.clone(), save_status.clone()),
        
        // File content editor
        s!(if selected_file.is_some() {
            Some(file_editor(file_content.clone(), save_status.clone()))
        } else {
            None
        }),
        
        // Status display
        s!(if !save_status.is_empty() {
            Some(
                text!("{}", save_status)
                    .padding(10.0)
                    .background(Color::light_gray())
                    .corner_radius(8.0)
            )
        } else {
            None
        }),
    ))
    .spacing(15.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(10.0)
}

fn file_operations(
    selected_file: Binding<Option<String>>,
    file_content: Binding<String>,
    save_status: Binding<String>
) -> impl View {
    hstack((
        button("Open File")
            .action({
                let selected_file = selected_file.clone();
                let file_content = file_content.clone();
                let save_status = save_status.clone();
                move |_| {
                    open_file_dialog(selected_file.clone(), file_content.clone(), save_status.clone());
                }
            }),
            
        button("Save File")
            .disabled(s!(selected_file.is_none()))
            .action({
                let selected_file = selected_file.clone();
                let file_content = file_content.clone();
                let save_status = save_status.clone();
                move |_| {
                    save_current_file(selected_file.clone(), file_content.clone(), save_status.clone());
                }
            }),
            
        button("Save As...")
            .action({
                let file_content = file_content.clone();
                let save_status = save_status.clone();
                move |_| {
                    save_file_as(file_content.clone(), save_status.clone());
                }
            }),
    ))
    .spacing(10.0)
}

fn open_file_dialog(
    selected_file: Binding<Option<String>>,
    file_content: Binding<String>,
    save_status: Binding<String>
) {
    #[cfg(target_arch = "wasm32")]
    {
        // Web file API
        let input = web_sys::HtmlInputElement::new().unwrap();
        input.set_type("file");
        input.set_accept(".txt,.md,.json,.rs");
        
        let selected_file = selected_file.clone();
        let file_content = file_content.clone();
        let save_status = save_status.clone();
        
        let closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
            let input = event.target().unwrap().dyn_into::<web_sys::HtmlInputElement>().unwrap();
            if let Some(files) = input.files() {
                if let Some(file) = files.get(0) {
                    let file_name = file.name();
                    selected_file.set(Some(file_name));
                    
                    // Read file content
                    let reader = web_sys::FileReader::new().unwrap();
                    let file_content = file_content.clone();
                    let save_status = save_status.clone();
                    
                    let onload = Closure::wrap(Box::new(move |event: web_sys::Event| {
                        let reader = event.target().unwrap().dyn_into::<web_sys::FileReader>().unwrap();
                        if let Ok(content) = reader.result() {
                            if let Some(text) = content.as_string() {
                                file_content.set(text);
                                save_status.set("File loaded successfully".to_string());
                            }
                        }
                    }) as Box<dyn Fn(_)>);
                    
                    reader.set_onload(Some(onload.as_ref().unchecked_ref()));
                    onload.forget();
                    reader.read_as_text(&file).unwrap();
                }
            }
        }) as Box<dyn Fn(_)>);
        
        input.set_onchange(Some(closure.as_ref().unchecked_ref()));
        closure.forget();
        input.click();
    }
    
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Native file dialog
        task::spawn(async move {
            use rfd::AsyncFileDialog;
            
            let file = AsyncFileDialog::new()
                .add_filter("Text files", &["txt", "md", "json", "rs"))
                .pick_file()
                .await;
                
            if let Some(file) = file {
                let path = file.path().to_string_lossy().to_string();
                selected_file.set(Some(path.clone()));
                
                match tokio::fs::read_to_string(&path).await {
                    Ok(content) => {
                        file_content.set(content);
                        save_status.set("File loaded successfully".to_string());
                    }
                    Err(e) => {
                        save_status.set(format!("Error loading file: {}", e));
                    }
                }
            }
        });
    }
}

fn save_current_file(
    selected_file: Binding<Option<String>>,
    file_content: Binding<String>,
    save_status: Binding<String>
) {
    if let Some(path) = selected_file.get() {
        let content = file_content.get();
        
        #[cfg(not(target_arch = "wasm32"))]
        {
            task::spawn(async move {
                match tokio::fs::write(&path, content).await {
                    Ok(_) => save_status.set("File saved successfully".to_string()),
                    Err(e) => save_status.set(format!("Error saving file: {}", e)),
                }
            });
        }
        
        #[cfg(target_arch = "wasm32")]
        {
            // Web download
            download_file(&path, &content);
            save_status.set("File downloaded".to_string());
        }
    }
}

fn save_file_as(
    file_content: Binding<String>,
    save_status: Binding<String>
) {
    let content = file_content.get();
    
    #[cfg(target_arch = "wasm32")]
    {
        download_file("untitled.txt", &content);
        save_status.set("File downloaded".to_string());
    }
    
    #[cfg(not(target_arch = "wasm32"))]
    {
        task::spawn(async move {
            use rfd::AsyncFileDialog;
            
            let file = AsyncFileDialog::new()
                .add_filter("Text files", &["txt"))
                .set_file_name("untitled.txt")
                .save_file()
                .await;
                
            if let Some(file) = file {
                match tokio::fs::write(file.path(), content).await {
                    Ok(_) => save_status.set("File saved successfully".to_string()),
                    Err(e) => save_status.set(format!("Error saving file: {}", e)),
                }
            }
        });
    }
}

#[cfg(target_arch = "wasm32")]
fn download_file(filename: &str, content: &str) {
    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();
    
    let blob = web_sys::Blob::new_with_str_sequence(
        &js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(content))
    ).unwrap();
    
    let url = web_sys::Url::create_object_url_with_blob(&blob).unwrap();
    
    let a = document.create_element("a").unwrap();
    let a = a.dyn_into::<web_sys::HtmlAnchorElement>().unwrap();
    a.set_href(&url);
    a.set_download(filename);
    a.click();
    
    web_sys::Url::revoke_object_url(&url).unwrap();
}

fn file_editor(
    file_content: Binding<String>,
    save_status: Binding<String>
) -> impl View {
    vstack((
        text("File Editor").font_size(14.0),
        
        text_area(file_content.clone())
            .min_height(300.0)
            .font_family("monospace")
            .on_change({
                let save_status = save_status.clone();
                move |_| {
                    save_status.set("Modified (unsaved)".to_string());
                }
            }),
    ))
    .spacing(10.0)
}
```

### Notification System

```rust,ignore
fn notification_system_demo() -> impl View {
    let notification_permission = binding(NotificationPermission::Default);
    let notification_history = binding(Vec::<NotificationRecord>::new());
    
    vstack((
        text("Notification System").font_size(16.0),
        
        // Permission status
        permission_status(notification_permission.clone()),
        
        // Notification controls
        notification_controls(notification_permission.clone(), notification_history.clone()),
        
        // Notification history
        notification_history_display(notification_history.clone()),
    ))
    .spacing(15.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(10.0)
}

#[derive(Clone, Debug, PartialEq)]
enum NotificationPermission {
    Default,
    Granted,
    Denied,
}

#[derive(Clone, Debug)]
struct NotificationRecord {
    id: u32,
    title: String,
    body: String,
    timestamp: chrono::DateTime<chrono::Utc>,
}

fn permission_status(permission: Binding<NotificationPermission>) -> impl View {
    hstack((
        text("Notification Permission:"),
        text!(s!(match permission {
            NotificationPermission::Default => "Not requested",
            NotificationPermission::Granted => "Granted",
            NotificationPermission::Denied => "Denied",
        }))
        .color(s!(match permission {
            NotificationPermission::Default => Color::secondary(),
            NotificationPermission::Granted => Color::green(),
            NotificationPermission::Denied => Color::red(),
        })),
        
        s!(if permission == NotificationPermission::Default {
            Some(
                button("Request Permission")
                    .action({
                        let permission = permission.clone();
                        move |_| {
                            request_notification_permission(permission.clone());
                        }
                    })
            )
        } else {
            None
        }),
    ))
    .spacing(10.0)
}

fn request_notification_permission(permission: Binding<NotificationPermission>) {
    #[cfg(target_arch = "wasm32")]
    {
        let permission_clone = permission.clone();
        
        wasm_bindgen_futures::spawn_local(async move {
            let window = web_sys::window().unwrap();
            let notification = js_sys::Reflect::get(&window, &"Notification".into()).unwrap();
            
            let request_permission = js_sys::Reflect::get(&notification, &"requestPermission".into()).unwrap();
            let request_permission = request_permission.dyn_into::<js_sys::Function>().unwrap();
            
            let promise = request_permission.call0(&notification).unwrap();
            let promise = promise.dyn_into::<js_sys::Promise>().unwrap();
            
            let result = wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
            let permission_string = result.as_string().unwrap();
            
            let perm = match permission_string.as_str() {
                "granted" => NotificationPermission::Granted,
                "denied" => NotificationPermission::Denied,
                _ => NotificationPermission::Default,
            };
            
            permission_clone.set(perm);
        });
    }
    
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Desktop notification permission is usually granted by default
        permission.set(NotificationPermission::Granted);
    }
}

fn notification_controls(
    permission: Binding<NotificationPermission>,
    history: Binding<Vec<NotificationRecord>>
) -> impl View {
    let title = binding(String::new());
    let body = binding(String::new());
    let notification_counter = use_counter();
    
    vstack((
        text_field(title.clone())
            .placeholder("Notification title"),
            
        text_area(body.clone())
            .placeholder("Notification body")
            .rows(3),
            
        hstack((
            button("Send Notification")
                .disabled(s!(
                    permission != NotificationPermission::Granted || 
                    title.trim().is_empty()
                ))
                .action({
                    let title = title.clone();
                    let body = body.clone();
                    let history = history.clone();
                    let counter = notification_counter.clone();
                    
                    move |_| {
                        let notification = NotificationRecord {
                            id: counter.increment(),
                            title: title.get(),
                            body: body.get(),
                            timestamp: chrono::Utc::now(),
                        };
                        
                        send_notification(&notification);
                        
                        history.update(|mut hist| {
                            hist.insert(0, notification);
                            if hist.len() > 10 {
                                hist.truncate(10);
                            }
                            hist
                        });
                        
                        title.set(String::new());
                        body.set(String::new());
                    }
                }),
                
            button("Test Notification")
                .disabled(s!(permission != NotificationPermission::Granted))
                .action({
                    let history = history.clone();
                    let counter = notification_counter.clone();
                    
                    move |_| {
                        let notification = NotificationRecord {
                            id: counter.increment(),
                            title: "Test Notification".to_string(),
                            body: "This is a test notification from WaterUI".to_string(),
                            timestamp: chrono::Utc::now(),
                        };
                        
                        send_notification(&notification);
                        
                        history.update(|mut hist| {
                            hist.insert(0, notification);
                            hist
                        });
                    }
                }),
        ))
        .spacing(10.0),
    ))
    .spacing(10.0)
}

fn send_notification(notification: &NotificationRecord) {
    #[cfg(target_arch = "wasm32")]
    {
        let options = js_sys::Object::new();
        js_sys::Reflect::set(&options, &"body".into(), &notification.body.clone().into()).unwrap();
        js_sys::Reflect::set(&options, &"icon".into(), &"/icon.png".into()).unwrap();
        
        let window = web_sys::window().unwrap();
        let notification_constructor = js_sys::Reflect::get(&window, &"Notification".into()).unwrap();
        let notification_constructor = notification_constructor.dyn_into::<js_sys::Function>().unwrap();
        
        let _notification = js_sys::Reflect::construct(
            &notification_constructor,
            &js_sys::Array::of2(&notification.title.clone().into(), &options)
        ).unwrap();
    }
    
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Desktop notification using notify-rust or similar
        #[cfg(feature = "desktop-notifications")]
        {
            use notify_rust::Notification;
            
            let _ = Notification::new()
                .summary(&notification.title)
                .body(&notification.body)
                .show();
        }
        
        #[cfg(not(feature = "desktop-notifications"))]
        {
            println!("Notification: {} - {}", notification.title, notification.body);
        }
    }
}

struct Counter {
    value: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

impl Counter {
    fn new() -> Self {
        Self {
            value: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
        }
    }
    
    fn increment(&self) -> u32 {
        self.value.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
    }
}

fn use_counter() -> Counter {
    Counter::new()
}

fn notification_history_display(history: Binding<Vec<NotificationRecord>>) -> impl View {
    vstack((
        text("Notification History").font_size(14.0),
        
        scroll(
            vstack(
                history.get().map(|notifications| {
                    if notifications.is_empty() {
                        vec![text("No notifications sent yet").color(Color::secondary())]
                    } else {
                        notifications.into_iter().map(|notif| {
                            notification_item(notif)
                        }).collect()
                    }
                })
            )
            .spacing(8.0)
        )
        .max_height(200.0),
    ))
    .spacing(10.0)
}

fn notification_item(notification: NotificationRecord) -> impl View {
    vstack((
        hstack((
            text!("{}", notification.title)
                .font_weight(FontWeight::Medium),
            spacer(),
            text!("{}", notification.timestamp.format("%H:%M:%S"))
                .font_size(12.0)
                .color(Color::secondary()),
        )),
        
        s!(if !notification.body.is_empty() {
            Some(
                text!("{}", notification.body)
                    .font_size(14.0)
                    .color(Color::secondary())
            )
        } else {
            None
        }),
    ))
    .spacing(5.0)
    .padding(10.0)
    .background(Color::light_gray())
    .corner_radius(6.0)
}
```

## Platform-Specific Features

### Desktop Integration

```rust,ignore
fn desktop_integration() -> impl View {
    vstack((
        text("Desktop Integration").font_size(16.0),
        
        // System tray
        system_tray_controls(),
        
        // Window management
        window_controls(),
        
        // Menu bar
        menu_bar_controls(),
        
        // Clipboard access
        clipboard_controls(),
    ))
    .spacing(15.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(10.0)
}

fn system_tray_controls() -> impl View {
    let tray_visible = binding(false);
    
    vstack((
        text("System Tray").font_size(14.0),
        
        toggle(tray_visible.clone())
            .label("Show in system tray")
            .on_change({
                let tray_visible = tray_visible.clone();
                move |visible| {
                    if visible {
                        create_system_tray();
                    } else {
                        remove_system_tray();
                    }
                    tray_visible.set(visible);
                }
            }),
    ))
    .spacing(8.0)
}

#[cfg(not(target_arch = "wasm32"))]
fn create_system_tray() {
    // System tray creation would be platform-specific
    #[cfg(target_os = "windows")]
    {
        // Windows system tray implementation
    }
    
    #[cfg(target_os = "macos")]
    {
        // macOS menu bar implementation
    }
    
    #[cfg(target_os = "linux")]
    {
        // Linux system tray implementation
    }
}

#[cfg(target_arch = "wasm32")]
fn create_system_tray() {
    // Not available on web
}

fn remove_system_tray() {
    // Remove system tray icon
}

fn window_controls() -> impl View {
    let always_on_top = binding(false);
    let window_title = binding("WaterUI App".to_string());
    
    vstack((
        text("Window Controls").font_size(14.0),
        
        text_field(window_title.clone())
            .label("Window Title")
            .on_change({
                let window_title = window_title.clone();
                move |title| {
                    set_window_title(&title);
                    window_title.set(title);
                }
            }),
            
        toggle(always_on_top.clone())
            .label("Always on top")
            .on_change({
                let always_on_top = always_on_top.clone();
                move |on_top| {
                    set_always_on_top(on_top);
                    always_on_top.set(on_top);
                }
            }),
            
        hstack((
            button("Minimize")
                .action(|_| minimize_window()),
            button("Maximize")
                .action(|_| maximize_window()),
            button("Center")
                .action(|_| center_window()),
        ))
        .spacing(10.0),
    ))
    .spacing(8.0)
}

fn set_window_title(title: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window().unwrap().document().unwrap().set_title(title);
    }
    
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Desktop window title setting would go here
        println!("Setting window title to: {}", title);
    }
}

fn set_always_on_top(on_top: bool) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Desktop always on top implementation
        println!("Setting always on top: {}", on_top);
    }
}

fn minimize_window() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Desktop window minimize implementation
        println!("Minimizing window");
    }
}

fn maximize_window() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Desktop window maximize implementation
        println!("Maximizing window");
    }
}

fn center_window() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Desktop window center implementation
        println!("Centering window");
    }
}

fn clipboard_controls() -> impl View {
    let clipboard_content = binding(String::new());
    let status = binding(String::new());
    
    vstack((
        text("Clipboard Access").font_size(14.0),
        
        text_area(clipboard_content.clone())
            .placeholder("Clipboard content will appear here...")
            .rows(3),
            
        hstack((
            button("Read Clipboard")
                .action({
                    let clipboard_content = clipboard_content.clone();
                    let status = status.clone();
                    move |_| {
                        read_clipboard(clipboard_content.clone(), status.clone());
                    }
                }),
                
            button("Write to Clipboard")
                .action({
                    let clipboard_content = clipboard_content.clone();
                    let status = status.clone();
                    move |_| {
                        write_clipboard(clipboard_content.get(), status.clone());
                    }
                }),
        ))
        .spacing(10.0),
        
        s!(if !status.is_empty() {
            Some(
                text!("{}", status)
                    .font_size(12.0)
                    .color(Color::secondary())
            )
        } else {
            None
        }),
    ))
    .spacing(8.0)
}

fn read_clipboard(content: Binding<String>, status: Binding<String>) {
    #[cfg(target_arch = "wasm32")]
    {
        let content = content.clone();
        let status = status.clone();
        
        wasm_bindgen_futures::spawn_local(async move {
            let window = web_sys::window().unwrap();
            let navigator = window.navigator();
            
            if let Some(clipboard) = navigator.clipboard() {
                match wasm_bindgen_futures::JsFuture::from(clipboard.read_text()).await {
                    Ok(text) => {
                        if let Some(text_str) = text.as_string() {
                            content.set(text_str);
                            status.set("Clipboard read successfully".to_string());
                        }
                    }
                    Err(_) => {
                        status.set("Failed to read clipboard".to_string());
                    }
                }
            } else {
                status.set("Clipboard API not available".to_string());
            }
        });
    }
    
    #[cfg(not(target_arch = "wasm32"))]
    {
        task::spawn(async move {
            #[cfg(feature = "clipboard")]
            {
                use arboard::Clipboard;
                let mut clipboard = Clipboard::new().unwrap();
                
                match clipboard.get_text() {
                    Ok(text) => {
                        content.set(text);
                        status.set("Clipboard read successfully".to_string());
                    }
                    Err(e) => {
                        status.set(format!("Failed to read clipboard: {}", e));
                    }
                }
            }
            
            #[cfg(not(feature = "clipboard"))]
            {
                status.set("Clipboard access not enabled".to_string());
            }
        });
    }
}

fn write_clipboard(text: String, status: Binding<String>) {
    #[cfg(target_arch = "wasm32")]
    {
        let status = status.clone();
        
        wasm_bindgen_futures::spawn_local(async move {
            let window = web_sys::window().unwrap();
            let navigator = window.navigator();
            
            if let Some(clipboard) = navigator.clipboard() {
                match wasm_bindgen_futures::JsFuture::from(
                    clipboard.write_text(&text)
                ).await {
                    Ok(_) => {
                        status.set("Text copied to clipboard".to_string());
                    }
                    Err(_) => {
                        status.set("Failed to write to clipboard".to_string());
                    }
                }
            } else {
                status.set("Clipboard API not available".to_string());
            }
        });
    }
    
    #[cfg(not(target_arch = "wasm32"))]
    {
        task::spawn(async move {
            #[cfg(feature = "clipboard")]
            {
                use arboard::Clipboard;
                let mut clipboard = Clipboard::new().unwrap();
                
                match clipboard.set_text(&text) {
                    Ok(_) => {
                        status.set("Text copied to clipboard".to_string());
                    }
                    Err(e) => {
                        status.set(format!("Failed to write clipboard: {}", e));
                    }
                }
            }
            
            #[cfg(not(feature = "clipboard"))]
            {
                status.set("Clipboard access not enabled".to_string());
            }
        });
    }
}
```

## Summary

WaterUI's platform integration provides:

- **Platform Detection**: Runtime detection of web, desktop, and mobile platforms
- **Native File Access**: File dialogs, reading/writing files across platforms
- **Notification System**: Cross-platform notifications with permission handling
- **Desktop Features**: System tray, window controls, clipboard access
- **Adaptive UI**: Platform-specific UI patterns and behaviors
- **API Abstraction**: Unified APIs that work across different platforms

Key best practices:
- Use conditional compilation for platform-specific code
- Provide fallbacks for unsupported features
- Handle permissions properly, especially on web platforms
- Test platform integrations on target platforms
- Use feature flags to enable/disable platform-specific functionality
- Consider user experience differences across platforms

Next: [Accessibility](18-accessibility.md)