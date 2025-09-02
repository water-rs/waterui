# Deployment

Deploying WaterUI applications across different platforms requires understanding each target environment's specific requirements and optimization strategies. This chapter covers deployment for desktop, web, mobile, and embedded platforms.

## Deployment Overview

### Target Platforms

WaterUI supports multiple deployment targets:

- **Desktop**: Linux, macOS, Windows (via GTK4 backend)
- **Web**: Browser-based applications (via WebAssembly)
- **Mobile**: iOS and Android (future backends)
- **Embedded**: IoT and embedded systems (no-std support)

### Build Configuration

```toml
# Cargo.toml
[package]
name = "my-waterui-app"
version = "0.1.0"
edition = "2021"

[dependencies]
waterui = { version = "0.1", features = ["gtk4"] }
nami = "0.1"
tokio = { version = "1.0", features = ["rt-multi-thread"] }

# Platform-specific dependencies
[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen = "0.2"
web-sys = "0.3"
console_error_panic_hook = "0.1"

# Desktop-specific dependencies
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
gtk4 = "0.5"

# Build profiles
[profile.release]
lto = true
codegen-units = 1
panic = "abort"
strip = true

[profile.wasm-release]
inherits = "release"
opt-level = "s"
```

## Desktop Deployment

### GTK4 Backend

```rust,ignore
// src/main.rs - Desktop application entry point
use waterui::*;
use nami::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize GTK
    gtk4::init()?;
    
    // Create app
    let app = gtk4::Application::new(
        Some("com.example.waterui-app"), 
        gtk4::gio::ApplicationFlags::DEFAULT_FLAGS
    );
    
    // Connect activate signal
    app.connect_activate(|app| {
        let window = gtk4::ApplicationWindow::new(app);
        window.set_title(Some("WaterUI App"));
        window.set_default_size(800, 600);
        
        // Mount WaterUI app
        let root_view = app_root();
        window.set_child(Some(&root_view.into_gtk_widget()));
        
        window.show();
    });
    
    // Run app
    app.run();
    Ok(())
}

fn app_root() -> impl View {
    let app_state = create_app_state();
    
    window(
        vstack((
            app_header(),
            main_content(app_state),
            app_footer(),
        ))
    )
    .title("My WaterUI App")
    .size(800, 600)
}
```

### Building for Desktop

```bash
# Build for current platform
cargo build --release

# Cross-compile for different platforms
cargo build --release --target x86_64-pc-windows-gnu
cargo build --release --target x86_64-apple-darwin
cargo build --release --target x86_64-unknown-linux-gnu

# Create distributable package
cargo install cargo-bundle
cargo bundle --release
```

### Desktop Packaging

#### Linux AppImage

```bash
# Install linuxdeploy
wget https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
chmod +x linuxdeploy-x86_64.AppImage

# Create AppDir structure
mkdir -p AppDir/usr/bin
cp target/release/my-waterui-app AppDir/usr/bin/

# Create desktop entry
cat > AppDir/my-waterui-app.desktop << EOF
[Desktop Entry]
Name=My WaterUI App
Exec=my-waterui-app
Icon=my-waterui-app
Type=Application
Categories=Utility;
EOF

# Create AppImage
./linuxdeploy-x86_64.AppImage --appdir AppDir --output appimage
```

#### macOS App Bundle

```toml
# Bundle.toml
[bundle]
name = "My WaterUI App"
identifier = "com.example.my-waterui-app"
icon = ["assets/icon.icns"]
version = "1.0.0"
copyright = "Copyright (c) 2024 Your Name"
category = "Utility"
short_description = "A WaterUI application"
long_description = "A cross-platform application built with WaterUI"

[bundle.osx]
frameworks = []
minimum_system_version = "10.15"
```

```bash
# Build macOS bundle
cargo bundle --release --target x86_64-apple-darwin

# Sign for distribution (requires Apple Developer account)
codesign --force --deep --sign "Developer ID Application: Your Name" \
  target/x86_64-apple-darwin/release/bundle/osx/My\ WaterUI\ App.app

# Create DMG
hdiutil create -volname "My WaterUI App" -srcfolder \
  "target/x86_64-apple-darwin/release/bundle/osx" -ov \
  "my-waterui-app.dmg"
```

#### Windows Installer

```toml
# Cargo.toml
[package.metadata.wix]
upgrade-guid = "12345678-1234-1234-1234-123456789012"
path-guid = "87654321-4321-4321-4321-210987654321"
license = "assets/license.rtf"
banner = "assets/banner.bmp"
dialog = "assets/dialog.bmp"
```

```bash
# Install cargo-wix
cargo install cargo-wix

# Build MSI installer
cargo wix --target x86_64-pc-windows-gnu --output target/wix/
```

## Web Deployment

### WebAssembly Backend

```rust,ignore
// src/lib.rs - WASM entry point
use wasm_bindgen::prelude::*;
use waterui::*;
use nami::*;

#[wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    
    let root_element = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .get_element_by_id("app")
        .unwrap();
    
    let app = web_app();
    app.mount_to_element(&root_element);
}

fn web_app() -> impl View {
    let app_state = create_app_state();
    
    vstack((
        web_header(),
        main_content(app_state),
        web_footer(),
    ))
    .style(css!("
        width: 100vw;
        height: 100vh;
        font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    "))
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

macro_rules! console_log {
    ($($t:tt)*) => (log(&format_args!($($t)*).to_string()))
}
```

### HTML Template

```html
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>My WaterUI App</title>
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <style>
        body {
            margin: 0;
            padding: 0;
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
        }
        
        #app {
            width: 100vw;
            height: 100vh;
        }
        
        .loading {
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            font-size: 18px;
            color: #666;
        }
    </style>
</head>
<body>
    <div id="app">
        <div class="loading">Loading WaterUI App...</div>
    </div>
    
    <script type="module">
        import init from './pkg/my_waterui_app.js';
        
        async function run() {
            await init();
        }
        
        run();
    </script>
</body>
</html>
```

### Building for Web

```bash
# Install wasm-pack
curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh

# Build for web
wasm-pack build --target web --out-dir pkg

# Optimize WASM size
wasm-opt -Oz pkg/my_waterui_app_bg.wasm -o pkg/my_waterui_app_bg.wasm

# Create production build
mkdir dist
cp index.html dist/
cp -r pkg dist/

# Serve locally for testing
python -m http.server 8000 --directory dist
```

### Web Optimization

```toml
# Cargo.toml - WASM-specific optimizations
[profile.release]
opt-level = "s"
lto = true
debug = false
panic = "abort"

[dependencies]
# Reduce bundle size
waterui = { version = "0.1", default-features = false, features = ["web"] }

# Tree-shaking friendly imports
[dependencies.web-sys]
version = "0.3"
features = [
  "console",
  "Document",
  "Element",
  "HtmlElement",
  "Window",
]
```

### Progressive Web App

```json
// manifest.json
{
  "name": "My WaterUI App",
  "short_name": "WaterUI App",
  "description": "A cross-platform application built with WaterUI",
  "start_url": "/",
  "display": "standalone",
  "background_color": "#ffffff",
  "theme_color": "#000000",
  "icons": [
    {
      "src": "icons/icon-192.png",
      "sizes": "192x192",
      "type": "image/png"
    },
    {
      "src": "icons/icon-512.png",
      "sizes": "512x512",
      "type": "image/png"
    }
  ]
}
```

```javascript
// service-worker.js
const CACHE_NAME = 'waterui-app-v1';
const urlsToCache = [
  '/',
  '/index.html',
  '/pkg/my_waterui_app.js',
  '/pkg/my_waterui_app_bg.wasm',
  '/manifest.json'
];

self.addEventListener('install', event => {
  event.waitUntil(
    caches.open(CACHE_NAME)
      .then(cache => cache.addAll(urlsToCache))
  );
});

self.addEventListener('fetch', event => {
  event.respondWith(
    caches.match(event.request)
      .then(response => {
        if (response) {
          return response;
        }
        return fetch(event.request);
      }
    )
  );
});
```

## Mobile Deployment (Future)

### iOS Deployment

```rust,ignore
// Future iOS backend example
#[cfg(target_os = "ios")]
use waterui_ios::*;

#[cfg(target_os = "ios")]
#[no_mangle]
pub extern "C" fn main_ios() {
    let app = ios_application(app_root());
    app.run();
}

// App delegate
#[cfg(target_os = "ios")]
struct AppDelegate;

#[cfg(target_os = "ios")]
impl UIApplicationDelegate for AppDelegate {
    fn application_did_finish_launching(&self) -> bool {
        let window = UIWindow::new();
        let view_controller = WaterUIViewController::new(app_root());
        window.set_root_view_controller(view_controller);
        window.make_key_and_visible();
        true
    }
}
```

### Android Deployment

```rust,ignore
// Future Android backend example
#[cfg(target_os = "android")]
use waterui_android::*;

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_com_example_waterui_MainActivity_initWaterUI(
    env: JNIEnv,
    _: JClass,
    activity: JObject,
) {
    let app = android_application(app_root());
    app.attach_to_activity(activity);
}
```

## Embedded Deployment

### No-std Configuration

```toml
# Cargo.toml for embedded targets
[dependencies]
waterui = { version = "0.1", default-features = false, features = ["embedded"] }
nami = { version = "0.1", default-features = false }

# Embedded-specific dependencies
embedded-hal = "0.2"
cortex-m = "0.7"
cortex-m-rt = "0.7"
panic-halt = "0.2"

# Target-specific configuration
[profile.release]
codegen-units = 1
debug = true
lto = true
```

```rust,ignore
// src/main.rs for embedded target
#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;
use waterui::*;
use nami::*;

#[entry]
fn main() -> ! {
    // Initialize hardware
    let peripherals = init_hardware();
    
    // Create embedded app
    let app = embedded_app(peripherals);
    
    // Run main loop
    loop {
        app.update();
        app.render();
    }
}

fn embedded_app(peripherals: Peripherals) -> impl View {
    let sensor_data = binding(SensorData::default());
    let display_mode = binding(DisplayMode::Temperature);
    
    // Background task for sensor reading
    spawn_sensor_task(sensor_data.clone(), peripherals.sensors);
    
    // UI for embedded display
    vstack((
        text!("Sensor Monitor"),
        s!(match display_mode {
            DisplayMode::Temperature => {
                text!("Temp: {:.1}°C", sensor_data.temperature)
            },
            DisplayMode::Humidity => {
                text!("Humidity: {:.1}%", sensor_data.humidity)
            },
            DisplayMode::Pressure => {
                text!("Pressure: {:.0} hPa", sensor_data.pressure)
            },
        }),
        button("Mode", {
            let display_mode = display_mode.clone();
            move || {
                display_mode.update(|mode| {
                    *mode = match *mode {
                        DisplayMode::Temperature => DisplayMode::Humidity,
                        DisplayMode::Humidity => DisplayMode::Pressure,
                        DisplayMode::Pressure => DisplayMode::Temperature,
                    }
                });
            }
        }),
    ))
}
```

## CI/CD Pipeline

### GitHub Actions

```yaml
# .github/workflows/deploy.yml
name: Deploy

on:
  push:
    tags: ['v*']
  pull_request:
    branches: [main]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
    - uses: actions/checkout@v3
    - uses: actions-rs/toolchain@v1
      with:
        toolchain: stable
    - name: Run tests
      run: cargo test --all-features
    - name: Run clippy
      run: cargo clippy --all-targets --all-features -- -D warnings

  build-desktop:
    needs: test
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
    - uses: actions/checkout@v3
    - uses: actions-rs/toolchain@v1
      with:
        toolchain: stable
    - name: Install GTK (Linux)
      if: matrix.os == 'ubuntu-latest'
      run: sudo apt-get install libgtk-4-dev
    - name: Build
      run: cargo build --release
    - name: Upload artifacts
      uses: actions/upload-artifact@v3
      with:
        name: ${{ matrix.os }}-binary
        path: target/release/my-waterui-app*

  build-web:
    needs: test
    runs-on: ubuntu-latest
    steps:
    - uses: actions/checkout@v3
    - uses: actions-rs/toolchain@v1
      with:
        toolchain: stable
        target: wasm32-unknown-unknown
    - name: Install wasm-pack
      run: curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
    - name: Build WASM
      run: wasm-pack build --target web --out-dir pkg
    - name: Deploy to GitHub Pages
      if: startsWith(github.ref, 'refs/tags/')
      uses: peaceiris/actions-gh-pages@v3
      with:
        github_token: ${{ secrets.GITHUB_TOKEN }}
        publish_dir: ./dist

  release:
    needs: [build-desktop, build-web]
    if: startsWith(github.ref, 'refs/tags/')
    runs-on: ubuntu-latest
    steps:
    - name: Download artifacts
      uses: actions/download-artifact@v3
    - name: Create release
      uses: softprops/action-gh-release@v1
      with:
        files: '**/*'
      env:
        GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

## Deployment Best Practices

### Security

```rust,ignore
// Security considerations
use waterui::*;
use nami::*;

// Sanitize user inputs
fn safe_text_input(input: Binding<String>) -> impl View {
    let sanitized = s!(sanitize_html(input.as_str()));
    text_field("Input", input)
        .validation(|text| {
            if text.len() > 1000 {
                Some("Input too long".to_string())
            } else if contains_harmful_content(text) {
                Some("Invalid content".to_string())
            } else {
                None
            }
        })
}

// Secure API communication
async fn secure_api_call<T>(endpoint: &str, data: T) -> Result<Response, ApiError> 
where
    T: serde::Serialize,
{
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("WaterUI-App/1.0")
        .build()?;
        
    let response = client
        .post(endpoint)
        .header("Authorization", format!("Bearer {}", get_auth_token()?))
        .json(&data)
        .send()
        .await?;
        
    if response.status().is_success() {
        Ok(response.json().await?)
    } else {
        Err(ApiError::HttpError(response.status()))
    }
}
```

### Performance

```rust,ignore
// Production optimizations
use waterui::*;
use nami::*;

// Lazy loading
fn optimized_app() -> impl View {
    let current_route = binding(Route::Home);
    
    s!(match current_route {
        Route::Home => Some(lazy_load_home()),
        Route::Dashboard => Some(lazy_load_dashboard()),
        Route::Settings => Some(lazy_load_settings()),
    })
}

// Code splitting
mod home {
    use super::*;
    
    pub fn lazy_load_home() -> impl View {
        // Heavy component loaded only when needed
        home_component()
    }
}

// Asset optimization
fn optimized_image(url: &str) -> impl View {
    image(url)
        .lazy_loading(true)
        .responsive(true)
        .webp_fallback(true)
        .placeholder(loading_spinner())
}
```

### Monitoring

```rust,ignore
// Production monitoring
use waterui::*;
use nami::*;

// Error tracking
fn with_error_boundary<V: View>(view: V) -> impl View {
    error_boundary(
        view,
        |error: Box<dyn std::error::Error>| {
            // Log to monitoring service
            log_error_to_service(&error);
            
            // Show user-friendly error
            vstack((
                text!("Something went wrong"),
                text!("Please try again later"),
                button("Retry", || {
                    // Retry action
                }),
            ))
        }
    )
}

// Performance metrics
fn with_performance_monitoring<V: View>(name: &str, view: V) -> impl View {
    let start_time = std::time::Instant::now();
    let result = view;
    let render_time = start_time.elapsed();
    
    // Send metrics to monitoring service
    send_metric("render_time", render_time, [("component", name)));
    
    result
}
```

Deployment is a crucial phase that requires careful consideration of target platforms, performance optimization, security measures, and monitoring. WaterUI's cross-platform architecture makes it straightforward to deploy to multiple targets while maintaining code reuse and consistency.
