# Migration Guide

This chapter provides guidance for migrating existing applications to WaterUI, upgrading between WaterUI versions, and transitioning from other UI frameworks.

## Migrating from Other Frameworks

### From React/Web Frameworks

If you're coming from React or other web frameworks, you'll find many familiar concepts in WaterUI with Rust-specific improvements.

#### Conceptual Mapping

| React Concept | WaterUI Equivalent | Notes |
|---------------|-------------------|-------|
| Component | `impl View` or `struct View` | Function or struct-based |
| useState | `binding()` | Reactive state with `s!` macro |
| useEffect | `effect()` | Side effects with cleanup |
| useMemo | `s!(computation)` | Automatic memoization |
| Props | Function parameters | Type-safe parameters |
| Context | Environment system | Type-based DI |
| JSX | Rust macros | `text!()`, component calls |

#### Migration Example

```javascript
// React component
function Counter({ initialValue = 0 }) {
  const [count, setCount] = useState(initialValue);
  const doubled = useMemo(() => count * 2, [count]);
  
  useEffect(() => {
    document.title = `Count: ${count}`;
  }, [count]);
  
  return (
    <div>
      <h1>Count: {count}</h1>
      <p>Doubled: {doubled}</p>
      <button onClick={() => setCount(count + 1)}>+1</button>
      <button onClick={() => setCount(0)}>Reset</button>
    </div>
  );
}
```

```rust,ignore
// WaterUI equivalent
use waterui::*;
use nami::*;

fn counter(initial_value: i32) -> impl View {
    let count = binding(initial_value);
    let doubled = s!(count * 2);
    
    // Effect for title update
    effect({
        let count = count.clone();
        move || {
            set_window_title(&format!("Count: {}", count.get()));
        }
    });
    
    vstack((
        text!("Count: {}", count),
        text!("Doubled: {}", doubled),
        hstack((
            button("+1", {
                let count = count.clone();
                move || count.update(|c| *c += 1)
            }),
            button("Reset", {
                let count = count.clone();
                move || count.set(0)
            }),
        )),
    ))
}
```

#### Key Differences

1. **Type Safety**: WaterUI provides compile-time type checking
2. **Memory Safety**: Rust's ownership prevents memory leaks
3. **Performance**: Zero-cost abstractions and efficient reactivity
4. **Explicit State**: No hidden re-renders or state mutations

### From Flutter

Flutter developers will find WaterUI's widget composition familiar.

#### Widget Migration

```dart
// Flutter
class ProfileCard extends StatelessWidget {
  final String name;
  final String email;
  final String? avatarUrl;
  
  const ProfileCard({
    required this.name,
    required this.email,
    this.avatarUrl,
  });
  
  @override
  Widget build(BuildContext context) {
    return Card(
      child: Padding(
        padding: EdgeInsets.all(16),
        child: Row(
          children: [
            CircleAvatar(
              backgroundImage: avatarUrl != null 
                ? NetworkImage(avatarUrl!) 
                : null,
              child: avatarUrl == null ? Icon(Icons.person) : null,
            ),
            SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    name,
                    style: Theme.of(context).textTheme.titleMedium,
                  ),
                  Text(
                    email,
                    style: Theme.of(context).textTheme.bodyMedium,
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}
```

```rust,ignore
// WaterUI equivalent
use waterui::*;
use nami::*;

fn profile_card(name: &str, email: &str, avatar_url: Option<&str>) -> impl View {
    card(
        hstack((
            s!(if let Some(url) = avatar_url {
                Some(circle_avatar(image(url)))
            } else {
                Some(circle_avatar(icon("person")))
            }),
            spacer().frame_width(12.0),
            vstack((
                text!(name)
                    .font_weight(FontWeight::Medium)
                    .font_size(16.0),
                text!(email)
                    .color(Color::gray())
                    .font_size(14.0),
            ))
            .alignment(HorizontalAlignment::Leading)
            .expand(),
        ))
        .padding(16.0)
    )
}

fn circle_avatar<V: View>(content: V) -> impl View {
    content
        .frame_size(40.0, 40.0)
        .corner_radius(20.0)
        .background(Color::gray().opacity(0.1))
}

fn card<V: View>(content: V) -> impl View {
    content
        .background(Color::white())
        .corner_radius(8.0)
        .shadow(Color::black().opacity(0.1), 2.0, (0.0, 1.0))
}
```

### From SwiftUI

SwiftUI and WaterUI share many design principles, making migration straightforward.

#### SwiftUI to WaterUI

```swift
// SwiftUI
struct ContentView: View {
    @State private var username = ""
    @State private var isLoggedIn = false
    
    var body: some View {
        VStack {
            if isLoggedIn {
                Text("Welcome, \(username)!")
                    .font(.title)
                
                Button("Logout") {
                    isLoggedIn = false
                    username = ""
                }
            } else {
                TextField("Username", text: $username)
                    .textFieldStyle(RoundedBorderTextFieldStyle())
                
                Button("Login") {
                    isLoggedIn = true
                }
                .disabled(username.isEmpty)
            }
        }
        .padding()
    }
}
```

```rust,ignore
// WaterUI equivalent
use waterui::*;
use nami::*;

fn content_view() -> impl View {
    let username = binding(String::new());
    let is_logged_in = binding(false);
    
    vstack(s!(if is_logged_in {
        Some(({
            let username_val = username.get();
            let is_logged_in = is_logged_in.clone();
            let username_clear = username.clone();
            
            vstack((
                text!("Welcome, {}!", username_val)
                    .font_size(24.0)
                    .font_weight(FontWeight::Bold),
                button("Logout", move || {
                    is_logged_in.set(false);
                    username_clear.set(String::new());
                }),
            ))
        }))
    } else {
        Some(({
            let username_clone = username.clone();
            let is_logged_in = is_logged_in.clone();
            let is_empty = s!(username.is_empty());
            
            vstack((
                text_field("Username", username.clone())
                    .border(Color::gray(), 1.0)
                    .corner_radius(4.0),
                button("Login", move || {
                    is_logged_in.set(true);
                })
                .disabled(is_empty),
            ))
        }))
    }))
    .padding(16.0)
}
```

## Version Migration

### Upgrading from 0.1.x to 0.2.x

#### Breaking Changes

1. **Macro Syntax Changes**
   ```rust,ignore
   // 0.1.x
   text(format!("Count: {}", count))
   
   // 0.2.x
   text!("Count: {}", count)
   ```

2. **Environment API Changes**
   ```rust,ignore
   // 0.1.x
   view.with_environment(theme)
   
   // 0.2.x
   view.environment(theme)
   ```

3. **Signal Creation**
   ```rust,ignore
   // 0.1.x
   let computed = signal(|| count.get() * 2);
   
   // 0.2.x
   let computed = s!(count * 2);
   ```

#### Migration Script

Create a migration script to automate common changes:

```bash
#!/bin/bash
# migrate_to_0_2.sh

echo "Migrating WaterUI project to 0.2.x..."

# Update Cargo.toml
sed -i 's/waterui = "0\.1"/waterui = "0.2"/g' Cargo.toml

# Update text formatting
find src -name "*.rs" -type f -exec sed -i 's/text(format!(\([^)]*\))/text!(\1)/g' {} +

# Update environment API
find src -name "*.rs" -type f -exec sed -i 's/\.with_environment(/.environment(/g' {} +

# Update signal creation (basic cases)
find src -name "*.rs" -type f -exec sed -i 's/signal(|| \([^)]*\))/s!(\1)/g' {} +

echo "Automated migration complete. Please review changes manually."
echo "Run 'cargo check' to identify remaining issues."
```

#### Manual Migration Steps

1. **Update Dependencies**
   ```toml
   [dependencies]
   waterui = "0.2"
   nami = "0.2"
   ```

2. **Fix Compilation Errors**
   ```bash
   cargo check
   # Fix errors one by one
   ```

3. **Update Tests**
   ```rust,ignore
   // Update test utilities
   #[cfg(test)]
   mod tests {
       use super::*;
       use waterui::testing::*;
       
       #[test]
       fn test_component() {
           let component = test_component();
           assert_view_contains!(component, "Expected text");
       }
   }
   ```

4. **Verify Functionality**
   ```bash
   cargo test
   cargo run
   ```

### Upgrading from 0.2.x to 0.3.x

#### New Features

1. **Async Components**
   ```rust,ignore
   // 0.3.x - New async support
   async fn async_data_view() -> impl View {
       let data = fetch_data().await;
       text!("Loaded: {}", data.len())
   }
   
   fn app() -> impl View {
       async_view(async_data_view())
           .loading(loading_spinner())
           .error(|err| text!("Error: {}", err))
   }
   ```

2. **Enhanced Animation API**
   ```rust,ignore
   // 0.3.x - Improved animations
   fn animated_button() -> impl View {
       let is_hovered = binding(false);
       
       button("Hover me", || {})
           .scale(s!(if is_hovered { 1.1 } else { 1.0 }))
           .animation(Animation::spring())
           .on_hover({
               let is_hovered = is_hovered.clone();
               move |hovered| is_hovered.set(hovered)
           })
   }
   ```

3. **Better Performance Monitoring**
   ```rust,ignore
   // 0.3.x - Built-in performance tools
   fn app() -> impl View {
       main_content()
           .performance_overlay(true)
           .memory_monitoring(true)
   }
   ```

## Framework-Specific Migration

### From Electron to WaterUI Native

#### Architecture Changes

```javascript
// Electron main process
const { app, BrowserWindow } = require('electron');

function createWindow() {
  const mainWindow = new BrowserWindow({
    width: 800,
    height: 600,
    webPreferences: {
      nodeIntegration: true
    }
  });
  
  mainWindow.loadFile('index.html');
}

app.whenReady().then(createWindow);
```

```rust,ignore
// WaterUI native equivalent
use waterui::*;
use nami::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = native_app(app_content())
        .title("My App")
        .size(800, 600)
        .resizable(true);
        
    app.run().await?;
    Ok(())
}

fn app_content() -> impl View {
    vstack((
        title_bar(),
        main_area(),
        status_bar(),
    ))
}
```

#### IPC Replacement

```javascript
// Electron IPC
const { ipcRenderer } = require('electron');

// Renderer process
ipcRenderer.invoke('get-user-data').then(data => {
  updateUI(data);
});

ipcRenderer.send('save-user-data', userData);
```

```rust,ignore
// WaterUI service communication
use waterui::*;
use nami::*;

#[derive(Clone)]
struct UserService {
    data: Binding<Option<UserData>>,
}

impl UserService {
    async fn load_user_data(&self) -> Result<UserData, ServiceError> {
        let data = tokio::fs::read_to_string("user_data.json").await?;
        let user_data: UserData = serde_json::from_str(&data)?;
        self.data.set(Some(user_data.clone()));
        Ok(user_data)
    }
    
    async fn save_user_data(&self, data: UserData) -> Result<(), ServiceError> {
        let json = serde_json::to_string_pretty(&data)?;
        tokio::fs::write("user_data.json", json).await?;
        self.data.set(Some(data));
        Ok(())
    }
}

fn app_with_service() -> impl View {
    let service = UserService {
        data: binding(None),
    };
    
    vstack((
        user_profile(service.data.clone()),
        data_controls(service.clone()),
    ))
    .environment(service)
}
```

### From Tauri to WaterUI

#### Command Migration

```rust,ignore
// Tauri command
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

// Frontend call
invoke('greet', { name: 'World' })
  .then((response) => console.log(response));
```

```rust,ignore
// WaterUI equivalent
use waterui::*;
use nami::*;

#[derive(Clone)]
struct GreetingService;

impl GreetingService {
    fn greet(&self, name: &str) -> String {
        format!("Hello, {}! You've been greeted from Rust!", name)
    }
}

fn greeting_component() -> impl View {
    let name = binding(String::new());
    let greeting = binding(String::new());
    let service = GreetingService;
    
    vstack((
        text_field("Enter name", name.clone()),
        button("Greet", {
            let name = name.clone();
            let greeting = greeting.clone();
            let service = service.clone();
            
            move || {
                let message = service.greet(&name.get());
                greeting.set(message);
            }
        }),
        text!(greeting),
    ))
}
```

## Migration Tools

### Code Analysis Tool

```rust,ignore
// migration_analyzer.rs
use syn::{visit::Visit, File, Item, ItemFn, Expr};
use std::fs;

struct MigrationAnalyzer {
    issues: Vec<MigrationIssue>,
}

#[derive(Debug)]
struct MigrationIssue {
    file: String,
    line: usize,
    issue_type: IssueType,
    suggestion: String,
}

#[derive(Debug)]
enum IssueType {
    DeprecatedMacro,
    OldEnvironmentApi,
    UnsafePattern,
}

impl<'ast> Visit<'ast> for MigrationAnalyzer {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        match expr {
            Expr::Macro(macro_expr) => {
                if let Some(path) = macro_expr.mac.path.get_ident() {
                    if path == "format_text" {
                        self.issues.push(MigrationIssue {
                            file: "current_file.rs".to_string(),
                            line: 0, // Would get from span
                            issue_type: IssueType::DeprecatedMacro,
                            suggestion: "Replace with text! macro".to_string(),
                        });
                    }
                }
            }
            _ => {}
        }
        
        syn::visit::visit_expr(self, expr);
    }
}

fn analyze_migration(file_path: &str) -> Result<Vec<MigrationIssue>, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(file_path)?;
    let syntax_tree: File = syn::parse_file(&content)?;
    
    let mut analyzer = MigrationAnalyzer {
        issues: Vec::new(),
    };
    
    analyzer.visit_file(&syntax_tree);
    Ok(analyzer.issues)
}
```

### Automated Refactoring

```rust,ignore
// refactor_tool.rs
use quote::quote;
use syn::{parse_quote, Expr, ExprMacro};

struct RefactoringTool;

impl RefactoringTool {
    fn refactor_text_macro(&self, old_expr: &ExprMacro) -> Expr {
        // Convert format_text!("Hello {}", name) to text!("Hello {}", name)
        let tokens = &old_expr.mac.tokens;
        parse_quote! {
            text!(#tokens)
        }
    }
    
    fn refactor_environment_call(&self, expr: &Expr) -> Expr {
        // Convert .with_environment(value) to .environment(value)
        // Implementation would use syn to parse and transform
        expr.clone() // Placeholder
    }
}
```

## Migration Checklist

### Pre-Migration

- [ ] **Backup your project**: Create a git branch or copy
- [ ] **Review changelog**: Check breaking changes
- [ ] **Update dependencies**: Check compatibility
- [ ] **Run tests**: Ensure current functionality works

### During Migration

- [ ] **Update Cargo.toml**: New version numbers
- [ ] **Fix compilation errors**: Address breaking changes
- [ ] **Update imports**: New module structure
- [ ] **Refactor deprecated APIs**: Use new equivalents
- [ ] **Update tests**: New testing utilities

### Post-Migration

- [ ] **Run all tests**: Verify functionality
- [ ] **Performance testing**: Check for regressions
- [ ] **Update documentation**: Reflect new APIs
- [ ] **Deploy to staging**: Test in environment
- [ ] **Monitor production**: Watch for issues

### Migration Timeline

For a typical medium-sized project (10,000-50,000 lines):

1. **Week 1**: Analysis and planning
2. **Week 2**: Dependency updates and basic migration
3. **Week 3**: Feature refactoring and testing
4. **Week 4**: Performance optimization and deployment

## Getting Help

### Migration Support

- **Documentation**: Check migration guides for your version
- **Community**: Join the WaterUI Discord for help
- **Issues**: Report migration problems on GitHub
- **Professional Support**: Consider consulting services for large migrations

### Common Pitfalls

1. **Rushing the migration**: Take time to understand changes
2. **Skipping tests**: Always verify functionality after changes
3. **Ignoring deprecation warnings**: Address warnings promptly
4. **Not reading changelogs**: Important changes are documented

Migration can seem daunting, but WaterUI's strong type system and helpful compiler errors make the process manageable. Take it step by step, and don't hesitate to ask for help when needed.
