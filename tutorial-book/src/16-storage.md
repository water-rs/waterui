# Storage

Persistent storage is essential for most applications. WaterUI provides comprehensive storage solutions including local storage, databases, and file system access with reactive integration.

## Local Storage

### Browser Storage

```rust,ignore
use waterui::*;
use nami::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct AppPreferences {
    theme: String,
    language: String,
    auto_save: bool,
    recent_files: Vec<String>,
}

fn local_storage_demo() -> impl View {
    let preferences = binding(AppPreferences::default());
    let storage_status = binding(String::new());
    
    // Load preferences on startup
    {
        let preferences = preferences.clone();
        let storage_status = storage_status.clone();
        task::spawn(async move {
            match load_preferences().await {
                Ok(prefs) => {
                    preferences.set(prefs);
                    storage_status.set("Preferences loaded successfully".to_string());
                }
                Err(e) => {
                    storage_status.set(format!("Failed to load preferences: {}", e));
                }
            }
        });
    }
    
    vstack((
        text("Local Storage Demo").font_size(20.0),
        
        // Status display
        s!(if !storage_status.is_empty() {
            Some(
                text!("{}", storage_status)
                    .padding(10.0)
                    .background(Color::light_gray())
                    .corner_radius(8.0)
            )
        } else {
            None
        }),
        
        // Preferences editor
        preferences_editor(preferences.clone()),
        
        // Save/Load controls
        storage_controls(preferences.clone(), storage_status.clone()),
        
        // Recent files
        recent_files_section(preferences.clone()),
    ))
    .spacing(15.0)
    .padding(20.0)
}

fn preferences_editor(preferences: Binding<AppPreferences>) -> impl View {
    vstack((
        text("Preferences").font_size(16.0),
        
        picker(s!(preferences.theme.clone()), vec![
            ("light".to_string(), "Light Theme"),
            ("dark".to_string(), "Dark Theme"),
            ("auto".to_string(), "Auto"),
        ))
        .label("Theme")
        .on_change({
            let preferences = preferences.clone();
            move |theme| {
                preferences.update(|mut prefs| {
                    prefs.theme = theme;
                    prefs
                });
            }
        }),
        
        picker(s!(preferences.language.clone()), vec![
            ("en".to_string(), "English"),
            ("es".to_string(), "Spanish"),
            ("fr".to_string(), "French"),
            ("de".to_string(), "German"),
        ))
        .label("Language")
        .on_change({
            let preferences = preferences.clone();
            move |language| {
                preferences.update(|mut prefs| {
                    prefs.language = language;
                    prefs
                });
            }
        }),
        
        toggle(s!(preferences.auto_save))
            .label("Auto-save documents")
            .on_change({
                let preferences = preferences.clone();
                move |auto_save| {
                    preferences.update(|mut prefs| {
                        prefs.auto_save = auto_save;
                        prefs
                    });
                }
            }),
    ))
    .spacing(10.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(10.0)
}

fn storage_controls(
    preferences: Binding<AppPreferences>,
    status: Binding<String>
) -> impl View {
    hstack((
        button("Save Preferences")
            .action({
                let preferences = preferences.clone();
                let status = status.clone();
                move |_| {
                    let prefs = preferences.get();
                    let status = status.clone();
                    task::spawn(async move {
                        match save_preferences(&prefs).await {
                            Ok(_) => status.set("Preferences saved successfully".to_string()),
                            Err(e) => status.set(format!("Failed to save: {}", e)),
                        }
                    });
                }
            }),
            
        button("Load Preferences")
            .action({
                let preferences = preferences.clone();
                let status = status.clone();
                move |_| {
                    let preferences = preferences.clone();
                    let status = status.clone();
                    task::spawn(async move {
                        match load_preferences().await {
                            Ok(prefs) => {
                                preferences.set(prefs);
                                status.set("Preferences loaded successfully".to_string());
                            }
                            Err(e) => {
                                status.set(format!("Failed to load: {}", e));
                            }
                        }
                    });
                }
            }),
            
        button("Clear Storage")
            .style(ButtonStyle::Destructive)
            .action({
                let preferences = preferences.clone();
                let status = status.clone();
                move |_| {
                    let preferences = preferences.clone();
                    let status = status.clone();
                    task::spawn(async move {
                        match clear_storage().await {
                            Ok(_) => {
                                preferences.set(AppPreferences::default());
                                status.set("Storage cleared".to_string());
                            }
                            Err(e) => {
                                status.set(format!("Failed to clear storage: {}", e));
                            }
                        }
                    });
                }
            }),
    ))
    .spacing(10.0)
}

async fn save_preferences(preferences: &AppPreferences) -> Result<(), String> {
    let json = serde_json::to_string(preferences)
        .map_err(|e| format!("Serialization error: {}", e))?;
        
    // Browser localStorage
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .ok_or("LocalStorage not available")?
            .set_item("app_preferences", &json)
            .map_err(|_| "Failed to save to localStorage")?;
    }
    
    // Desktop file storage
    #[cfg(not(target_arch = "wasm32"))]
    {
        let config_dir = dirs::config_dir()
            .ok_or("Config directory not found")?
            .join("waterui-app");
        
        tokio::fs::create_dir_all(&config_dir).await
            .map_err(|e| format!("Failed to create config directory: {}", e))?;
            
        let config_file = config_dir.join("preferences.json");
        tokio::fs::write(&config_file, json).await
            .map_err(|e| format!("Failed to write config file: {}", e))?;
    }
    
    Ok(())
}

async fn load_preferences() -> Result<AppPreferences, String> {
    // Browser localStorage
    #[cfg(target_arch = "wasm32")]
    {
        let json = web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .ok_or("LocalStorage not available")?
            .get_item("app_preferences")
            .map_err(|_| "Failed to read from localStorage")?
            .unwrap_or_default();
            
        if json.is_empty() {
            return Ok(AppPreferences::default());
        }
        
        serde_json::from_str(&json)
            .map_err(|e| format!("Deserialization error: {}", e))
    }
    
    // Desktop file storage
    #[cfg(not(target_arch = "wasm32"))]
    {
        let config_file = dirs::config_dir()
            .ok_or("Config directory not found")?
            .join("waterui-app")
            .join("preferences.json");
            
        if !config_file.exists() {
            return Ok(AppPreferences::default());
        }
        
        let json = tokio::fs::read_to_string(&config_file).await
            .map_err(|e| format!("Failed to read config file: {}", e))?;
            
        serde_json::from_str(&json)
            .map_err(|e| format!("Deserialization error: {}", e))
    }
}

async fn clear_storage() -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .ok_or("LocalStorage not available")?
            .remove_item("app_preferences")
            .map_err(|_| "Failed to clear localStorage")?;
    }
    
    #[cfg(not(target_arch = "wasm32"))]
    {
        let config_file = dirs::config_dir()
            .ok_or("Config directory not found")?
            .join("waterui-app")
            .join("preferences.json");
            
        if config_file.exists() {
            tokio::fs::remove_file(&config_file).await
                .map_err(|e| format!("Failed to delete config file: {}", e))?;
        }
    }
    
    Ok(())
}

fn recent_files_section(preferences: Binding<AppPreferences>) -> impl View {
    let new_file = binding(String::new());
    
    vstack((
        text("Recent Files").font_size(16.0),
        
        hstack((
            text_field(new_file.clone())
                .placeholder("Add file path...")
                .flex(1)
                .on_submit({
                    let preferences = preferences.clone();
                    let new_file = new_file.clone();
                    move || {
                        add_recent_file(preferences.clone(), new_file.clone());
                    }
                }),
                
            button("Add")
                .disabled(s!(new_file.trim().is_empty()))
                .action({
                    let preferences = preferences.clone();
                    let new_file = new_file.clone();
                    move |_| {
                        add_recent_file(preferences.clone(), new_file.clone());
                    }
                }),
        ))
        .spacing(10.0),
        
        scroll(
            vstack(
                s!(preferences.recent_files.clone()).map(|files| {
                    files.into_iter().enumerate().map(|(index, file)| {
                        recent_file_item(file, index, preferences.clone())
                    })
                })
            )
            .spacing(5.0)
        )
        .max_height(200.0),
    ))
    .spacing(10.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(10.0)
}

fn add_recent_file(preferences: Binding<AppPreferences>, new_file: Binding<String>) {
    let file_path = new_file.get().trim().to_string();
    if !file_path.is_empty() {
        preferences.update(|mut prefs| {
            // Remove if already exists
            prefs.recent_files.retain(|f| f != &file_path);
            // Add to beginning
            prefs.recent_files.insert(0, file_path);
            // Keep only last 10
            prefs.recent_files.truncate(10);
            prefs
        });
        new_file.set(String::new());
    }
}

fn recent_file_item(
    file: String,
    index: usize,
    preferences: Binding<AppPreferences>
) -> impl View {
    hstack((
        text!("{}", file)
            .flex(1)
            .font_size(14.0),
            
        button("×")
            .style(ButtonStyle::Destructive)
            .action({
                let preferences = preferences.clone();
                move |_| {
                    preferences.update(|mut prefs| {
                        prefs.recent_files.remove(index);
                        prefs
                    });
                }
            }),
    ))
    .spacing(10.0)
    .padding(8.0)
    .background(Color::light_gray())
    .corner_radius(6.0)
}
```

## Database Integration

### SQLite Database

```rust,ignore
use sqlx::{SqlitePool, Row};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Task {
    id: Option<i64>,
    title: String,
    description: String,
    completed: bool,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

struct TaskDatabase {
    pool: SqlitePool,
}

impl TaskDatabase {
    async fn new(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = SqlitePool::connect(database_url).await?;
        
        // Create tables
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                completed BOOLEAN NOT NULL DEFAULT FALSE,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        "#)
        .execute(&pool)
        .await?;
        
        Ok(Self { pool })
    }
    
    async fn create_task(&self, task: &Task) -> Result<i64, sqlx::Error> {
        let result = sqlx::query(r#"
            INSERT INTO tasks (title, description, completed)
            VALUES (?1, ?2, ?3)
        "#)
        .bind(&task.title)
        .bind(&task.description)
        .bind(task.completed)
        .execute(&self.pool)
        .await?;
        
        Ok(result.last_insert_rowid())
    }
    
    async fn get_all_tasks(&self) -> Result<Vec<Task>, sqlx::Error> {
        let rows = sqlx::query(r#"
            SELECT id, title, description, completed, created_at, updated_at
            FROM tasks
            ORDER BY created_at DESC
        "#)
        .fetch_all(&self.pool)
        .await?;
        
        let tasks = rows.into_iter().map(|row| Task {
            id: Some(row.get("id")),
            title: row.get("title"),
            description: row.get("description"),
            completed: row.get("completed"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        }).collect();
        
        Ok(tasks)
    }
    
    async fn update_task(&self, task: &Task) -> Result<(), sqlx::Error> {
        sqlx::query(r#"
            UPDATE tasks
            SET title = ?1, description = ?2, completed = ?3, updated_at = CURRENT_TIMESTAMP
            WHERE id = ?4
        "#)
        .bind(&task.title)
        .bind(&task.description)
        .bind(task.completed)
        .bind(task.id)
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
    
    async fn delete_task(&self, id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM tasks WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
            
        Ok(())
    }
}

fn database_demo() -> impl View {
    let db = use_memo(|| {
        task::spawn(async {
            TaskDatabase::new("sqlite:tasks.db").await
        })
    });
    
    let tasks = binding(Vec::<Task>::new());
    let loading = binding(false);
    let error = binding(None::<String>);
    
    // Load tasks on startup
    {
        let tasks = tasks.clone();
        let loading = loading.clone();
        let error = error.clone();
        let db = db.clone();
        
        task::spawn(async move {
            if let Ok(database) = db.await {
                load_tasks(&database, tasks, loading, error).await;
            }
        });
    }
    
    vstack((
        text("Database Demo").font_size(20.0),
        
        // Add task form
        add_task_form(db.clone(), tasks.clone(), error.clone()),
        
        // Error display
        s!(if let Some(err) = &error {
            Some(
                text!("Error: {}", err)
                    .color(Color::red())
                    .padding(10.0)
                    .background(Color::red().opacity(0.1))
                    .corner_radius(8.0)
            )
        } else {
            None
        }),
        
        // Loading indicator
        s!(if loading {
            Some(loading_indicator())
        } else {
            None
        }),
        
        // Tasks list
        tasks_list(db.clone(), tasks.clone(), error.clone()),
    ))
    .spacing(15.0)
    .padding(20.0)
}

fn add_task_form(
    db: impl Future<Output = Result<TaskDatabase, sqlx::Error>> + Clone + 'static,
    tasks: Binding<Vec<Task>>,
    error: Binding<Option<String>>
) -> impl View {
    let title = binding(String::new());
    let description = binding(String::new());
    let adding = binding(false);
    
    vstack((
        text("Add New Task").font_size(16.0),
        
        text_field(title.clone())
            .placeholder("Task title")
            .disabled(adding.get()),
            
        text_area(description.clone())
            .placeholder("Task description (optional)")
            .disabled(adding.get())
            .rows(3),
            
        button("Add Task")
            .disabled(s!(title.trim().is_empty() || adding))
            .action({
                let db = db.clone();
                let title = title.clone();
                let description = description.clone();
                let adding = adding.clone();
                let tasks = tasks.clone();
                let error = error.clone();
                
                move |_| {
                    let task = Task {
                        id: None,
                        title: title.get(),
                        description: description.get(),
                        completed: false,
                        created_at: None,
                        updated_at: None,
                    };
                    
                    create_task(
                        db.clone(),
                        task,
                        title.clone(),
                        description.clone(),
                        adding.clone(),
                        tasks.clone(),
                        error.clone()
                    );
                }
            }),
    ))
    .spacing(10.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(10.0)
}

async fn create_task(
    db: impl Future<Output = Result<TaskDatabase, sqlx::Error>>,
    task: Task,
    title: Binding<String>,
    description: Binding<String>,
    adding: Binding<bool>,
    tasks: Binding<Vec<Task>>,
    error: Binding<Option<String>>
) {
    adding.set(true);
    error.set(None);
    
    match db.await {
        Ok(database) => {
            match database.create_task(&task).await {
                Ok(_) => {
                    title.set(String::new());
                    description.set(String::new());
                    load_tasks(&database, tasks, binding(false), error).await;
                }
                Err(e) => {
                    error.set(Some(format!("Failed to create task: {}", e)));
                }
            }
        }
        Err(e) => {
            error.set(Some(format!("Database connection failed: {}", e)));
        }
    }
    
    adding.set(false);
}

async fn load_tasks(
    db: &TaskDatabase,
    tasks: Binding<Vec<Task>>,
    loading: Binding<bool>,
    error: Binding<Option<String>>
) {
    loading.set(true);
    error.set(None);
    
    match db.get_all_tasks().await {
        Ok(task_list) => {
            tasks.set(task_list);
        }
        Err(e) => {
            error.set(Some(format!("Failed to load tasks: {}", e)));
        }
    }
    
    loading.set(false);
}

fn tasks_list(
    db: impl Future<Output = Result<TaskDatabase, sqlx::Error>> + Clone + 'static,
    tasks: Binding<Vec<Task>>,
    error: Binding<Option<String>>
) -> impl View {
    scroll(
        vstack(
            tasks.signal().map(|tasks| {
                tasks.into_iter().map(|task| {
                    task_item(task, db.clone(), tasks.clone(), error.clone())
                })
            })
        )
        .spacing(8.0)
    )
    .max_height(400.0)
}

fn task_item(
    task: Task,
    db: impl Future<Output = Result<TaskDatabase, sqlx::Error>> + Clone + 'static,
    tasks: Binding<Vec<Task>>,
    error: Binding<Option<String>>
) -> impl View {
    let completed = binding(task.completed);
    
    hstack((
        toggle(completed.clone())
            .on_change({
                let task = task.clone();
                let db = db.clone();
                let tasks = tasks.clone();
                let error = error.clone();
                
                move |is_completed| {
                    let mut updated_task = task.clone();
                    updated_task.completed = is_completed;
                    
                    update_task(db.clone(), updated_task, tasks.clone(), error.clone());
                }
            }),
            
        vstack((
            text!("{}", task.title)
                .font_weight(FontWeight::Medium)
                .strikethrough(completed.get()),
                
            s!(if !task.description.is_empty() {
                Some(
                    text!("{}", task.description)
                        .font_size(14.0)
                        .color(Color::secondary())
                        .strikethrough(completed.get())
                )
            } else {
                None
            }),
            
            s!(if let Some(created_at) = task.created_at {
                Some(
                    text!("Created: {}", created_at.format("%Y-%m-%d %H:%M"))
                        .font_size(12.0)
                        .color(Color::secondary())
                )
            } else {
                None
            }),
        ))
        .spacing(4.0)
        .flex(1),
        
        button("Delete")
            .style(ButtonStyle::Destructive)
            .action({
                let task_id = task.id.unwrap_or(0);
                let db = db.clone();
                let tasks = tasks.clone();
                let error = error.clone();
                
                move |_| {
                    delete_task(db.clone(), task_id, tasks.clone(), error.clone());
                }
            }),
    ))
    .spacing(15.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(8.0)
}

async fn update_task(
    db: impl Future<Output = Result<TaskDatabase, sqlx::Error>>,
    task: Task,
    tasks: Binding<Vec<Task>>,
    error: Binding<Option<String>>
) {
    match db.await {
        Ok(database) => {
            match database.update_task(&task).await {
                Ok(_) => {
                    load_tasks(&database, tasks, binding(false), error).await;
                }
                Err(e) => {
                    error.set(Some(format!("Failed to update task: {}", e)));
                }
            }
        }
        Err(e) => {
            error.set(Some(format!("Database connection failed: {}", e)));
        }
    }
}

async fn delete_task(
    db: impl Future<Output = Result<TaskDatabase, sqlx::Error>>,
    task_id: i64,
    tasks: Binding<Vec<Task>>,
    error: Binding<Option<String>>
) {
    match db.await {
        Ok(database) => {
            match database.delete_task(task_id).await {
                Ok(_) => {
                    load_tasks(&database, tasks, binding(false), error).await;
                }
                Err(e) => {
                    error.set(Some(format!("Failed to delete task: {}", e)));
                }
            }
        }
        Err(e) => {
            error.set(Some(format!("Database connection failed: {}", e)));
        }
    }
}
```

## File System Access

### File Operations

```rust,ignore
use std::path::PathBuf;

fn file_manager_demo() -> impl View {
    let current_directory = binding(std::env::current_dir().unwrap_or_default());
    let files = binding(Vec::<FileEntry>::new());
    let selected_file = binding(None::<PathBuf>);
    let file_content = binding(String::new());
    let loading = binding(false);
    let error = binding(None::<String>);
    
    // Load directory on startup
    {
        let current_directory = current_directory.clone();
        let files = files.clone();
        let loading = loading.clone();
        let error = error.clone();
        
        task::spawn(async move {
            load_directory(current_directory.get(), files, loading, error).await;
        });
    }
    
    vstack((
        text("File Manager Demo").font_size(20.0),
        
        // Directory navigation
        directory_navigation(current_directory.clone(), files.clone()),
        
        // File browser
        file_browser(files.clone(), selected_file.clone()),
        
        // File content viewer
        s!(if selected_file.is_some() {
            Some(file_viewer(selected_file.clone(), file_content.clone()))
        } else {
            None
        }),
        
        // Error display
        s!(if let Some(err) = &error {
            Some(error_display(err.clone()))
        } else {
            None
        }),
    ))
    .spacing(15.0)
    .padding(20.0)
}

#[derive(Clone, Debug)]
struct FileEntry {
    path: PathBuf,
    name: String,
    is_directory: bool,
    size: Option<u64>,
    modified: Option<std::time::SystemTime>,
}

fn directory_navigation(
    current_directory: Binding<PathBuf>,
    files: Binding<Vec<FileEntry>>
) -> impl View {
    vstack((
        text!("Current Directory: {}", s!(current_directory.display().to_string())),
        
        hstack((
            button("Parent Directory")
                .disabled(s!(current_directory.parent().is_none()))
                .action({
                    let current_directory = current_directory.clone();
                    let files = files.clone();
                    move |_| {
                        if let Some(parent) = current_directory.get().parent() {
                            let parent_path = parent.to_path_buf();
                            current_directory.set(parent_path.clone());
                            
                            let files = files.clone();
                            task::spawn(async move {
                                load_directory(parent_path, files, binding(false), binding(None)).await;
                            });
                        }
                    }
                }),
                
            button("Refresh")
                .action({
                    let current_directory = current_directory.clone();
                    let files = files.clone();
                    move |_| {
                        let dir = current_directory.get();
                        let files = files.clone();
                        task::spawn(async move {
                            load_directory(dir, files, binding(false), binding(None)).await;
                        });
                    }
                }),
        ))
        .spacing(10.0),
    ))
    .spacing(10.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(10.0)
}

async fn load_directory(
    directory: PathBuf,
    files: Binding<Vec<FileEntry>>,
    loading: Binding<bool>,
    error: Binding<Option<String>>
) {
    loading.set(true);
    error.set(None);
    
    match tokio::fs::read_dir(&directory).await {
        Ok(mut entries) => {
            let mut file_entries = Vec::new();
            
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let metadata = entry.metadata().await.ok();
                
                let file_entry = FileEntry {
                    name: path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    path: path.clone(),
                    is_directory: path.is_dir(),
                    size: metadata.as_ref().and_then(|m| {
                        if m.is_file() { Some(m.len()) } else { None }
                    }),
                    modified: metadata.and_then(|m| m.modified().ok()),
                };
                
                file_entries.push(file_entry);
            }
            
            // Sort: directories first, then files
            file_entries.sort_by(|a, b| {
                match (a.is_directory, b.is_directory) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.name.cmp(&b.name),
                }
            });
            
            files.set(file_entries);
        }
        Err(e) => {
            error.set(Some(format!("Failed to read directory: {}", e)));
        }
    }
    
    loading.set(false);
}

fn file_browser(
    files: Binding<Vec<FileEntry>>,
    selected_file: Binding<Option<PathBuf>>
) -> impl View {
    scroll(
        vstack(
            files.signal().map(|files| {
                files.into_iter().map(|file| {
                    file_entry_item(file, selected_file.clone())
                })
            })
        )
        .spacing(2.0)
    )
    .max_height(300.0)
}

fn file_entry_item(
    file: FileEntry,
    selected_file: Binding<Option<PathBuf>>
) -> impl View {
    let is_selected = s!(selected_file.as_ref() == Some(&file.path));
    
    hstack((
        text(if file.is_directory { "📁" } else { "📄" }),
        
        text!("{}", file.name)
            .flex(1)
            .font_weight(s!(if file.is_directory { 
                FontWeight::Medium 
            } else { 
                FontWeight::Regular 
            })),
            
        s!(if let Some(size) = file.size {
            Some(
                text!("{}", format_file_size(size))
                    .font_size(12.0)
                    .color(Color::secondary())
            )
        } else {
            None
        }),
        
        s!(if let Some(modified) = file.modified {
            Some(
                text!("{}", format_time(modified))
                    .font_size(12.0)
                    .color(Color::secondary())
            )
        } else {
            None
        }),
    ))
    .spacing(10.0)
    .padding(8.0)
    .background(s!(if is_selected { 
        Color::primary().opacity(0.1) 
    } else { 
        Color::transparent() 
    }))
    .corner_radius(4.0)
    .on_tap({
        let file = file.clone();
        let selected_file = selected_file.clone();
        move |_| {
            if file.is_directory {
                // Navigate to directory
            } else {
                // Select file
                selected_file.set(Some(file.path.clone()));
            }
        }
    })
}

fn file_viewer(
    selected_file: Binding<Option<PathBuf>>,
    file_content: Binding<String>
) -> impl View {
    let loading = binding(false);
    
    vstack((
        text!("File: {}", s!(selected_file.as_ref().map(|p| p.display().to_string()).unwrap_or_default())),
        
        hstack((
            button("Load Content")
                .disabled(loading.get())
                .action({
                    let selected_file = selected_file.clone();
                    let file_content = file_content.clone();
                    let loading = loading.clone();
                    
                    move |_| {
                        if let Some(path) = selected_file.get() {
                            load_file_content(path, file_content.clone(), loading.clone());
                        }
                    }
                }),
                
            button("Save Content")
                .action({
                    let selected_file = selected_file.clone();
                    let file_content = file_content.clone();
                    
                    move |_| {
                        if let Some(path) = selected_file.get() {
                            save_file_content(path, file_content.get());
                        }
                    }
                }),
        ))
        .spacing(10.0),
        
        text_area(file_content.clone())
            .placeholder("File content will appear here...")
            .min_height(200.0)
            .font_family("monospace"),
    ))
    .spacing(10.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(10.0)
}

fn load_file_content(
    path: PathBuf,
    content: Binding<String>,
    loading: Binding<bool>
) {
    task::spawn(async move {
        loading.set(true);
        
        match tokio::fs::read_to_string(&path).await {
            Ok(file_content) => {
                content.set(file_content);
            }
            Err(e) => {
                content.set(format!("Error reading file: {}", e));
            }
        }
        
        loading.set(false);
    });
}

fn save_file_content(path: PathBuf, content: String) {
    task::spawn(async move {
        if let Err(e) = tokio::fs::write(&path, content).await {
            eprintln!("Error saving file: {}", e);
        }
    });
}

fn format_file_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = size as f64;
    let mut unit_index = 0;
    
    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }
    
    format!("{:.1} {}", size, UNITS[unit_index))
}

fn format_time(time: std::time::SystemTime) -> String {
    use chrono::{DateTime, Utc};
    
    let datetime: DateTime<Utc> = time.into();
    datetime.format("%Y-%m-%d %H:%M").to_string()
}

fn error_display(error: String) -> impl View {
    text!("Error: {}", error)
        .color(Color::red())
        .padding(10.0)
        .background(Color::red().opacity(0.1))
        .corner_radius(8.0)
}
```

## Summary

WaterUI's storage capabilities provide:

- **Local Storage**: Browser localStorage and desktop file-based configuration
- **Database Integration**: SQLite and other databases with async operations
- **File System Access**: Directory browsing, file reading/writing
- **Reactive Integration**: Automatic UI updates with storage operations
- **Cross-Platform**: Works on web, desktop, and mobile platforms
- **Type Safety**: Serialization/deserialization with proper error handling

Key best practices:
- Use appropriate storage for data types (local storage for preferences, database for structured data)
- Handle storage errors gracefully with user-friendly messages
- Implement loading states for async operations
- Structure data with clear schemas and migrations
- Test storage operations across platforms
- Consider data privacy and security implications

Next: [Platform Integration](17-platform.md)