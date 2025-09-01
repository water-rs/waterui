# WaterUI Data Management

Data persistence and management utilities for the WaterUI framework.

## Overview

`waterui-data` provides reactive data management capabilities including database integration, collection handling, and data persistence. It offers a reactive interface for working with databases and data sources.

## Core Components

### Data

Reactive data container for database-backed collections:

```rust
use waterui_data::{Data, Schema, Id};

#[derive(Serialize, Deserialize)]
struct User {
    id: Id,
    name: String,
    email: String,
    created_at: DateTime<Utc>,
}

impl Schema for User {}

let users: Data<User> = Data::new()
    .database(database)
    .table("users")
    .auto_sync(true);
```

### DataElement

Individual data items with reactive capabilities:

```rust
use waterui_data::DataElement;

let user_element: DataElement<User> = users.get(0).unwrap();
let user_data = user_element.data();
let user_id = user_element.id;

// Reactive updates
user_element.update(|user| {
    user.name = "Updated Name".to_string();
});
```

## Schema Definition

### Schema Trait

Define data structures for persistence:

```rust
use waterui_data::Schema;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Todo {
    pub id: Id,
    pub title: String,
    pub completed: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl Schema for Todo {}
```

### Relationships

Define relationships between data models:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Post {
    pub id: Id,
    pub title: String,
    pub content: String,
    pub author_id: Id,  // Foreign key
    pub tags: Vec<Id>,  // Many-to-many relationship
}

impl Schema for Post {
    fn relationships() -> Vec<Relationship> {
        vec![
            Relationship::belongs_to("author", "users", "author_id"),
            Relationship::has_many("comments", "comments", "post_id"),
            Relationship::many_to_many("tags", "tags", "post_tags"),
        ]
    }
}
```

## Database Integration

### Database Trait

Interface for different database backends:

```rust
use waterui_data::database::{Database, DatabaseError};

pub trait Database: Send + Sync {
    async fn insert<T: Schema>(&self, item: &T) -> Result<Id, DatabaseError>;
    async fn update<T: Schema>(&self, id: Id, item: &T) -> Result<(), DatabaseError>;
    async fn delete<T: Schema>(&self, id: Id) -> Result<(), DatabaseError>;
    async fn find<T: Schema>(&self, id: Id) -> Result<Option<T>, DatabaseError>;
    async fn find_all<T: Schema>(&self) -> Result<Vec<T>, DatabaseError>;
    async fn query<T: Schema>(&self, query: &str) -> Result<Vec<T>, DatabaseError>;
}
```

### Default Database

Built-in database implementation:

```rust
use waterui_data::database::DefaultDatabase;

let database = DefaultDatabase::new()
    .connection_string("sqlite:///app.db")
    .auto_migrate(true)
    .connection_pool_size(10)
    .connect()
    .await?;
```

## Reactive Collections

### Collection Trait

Interface for reactive data collections:

```rust
use nami::collection::Collection;

impl<T: Schema> Collection for Data<T> {
    type Item = DataElement<T>;

    fn get(&self, index: usize) -> Option<Self::Item>;
    fn len(&self) -> usize;
    fn add_watcher(&self, watcher: Watcher<()>) -> WatcherGuard;
}
```

### Reactive Queries

Dynamic, reactive database queries:

```rust
let active_todos = Data::<Todo>::new()
    .filter(|todo| !todo.completed)
    .order_by("created_at", Order::Desc)
    .limit(20);

// Automatically updates when data changes
let todo_list = ForEach::new(active_todos, |todo| {
    TodoItemView::new(todo)
});
```

## Data Operations

### CRUD Operations

Create, read, update, delete operations:

```rust
// Create
let new_user = User {
    id: 0,  // Will be assigned by database
    name: "John Doe".to_string(),
    email: "john@example.com".to_string(),
    created_at: Utc::now(),
};
let user_id = users.insert(new_user).await?;

// Read
let user = users.find(user_id).await?;

// Update
users.update(user_id, |user| {
    user.email = "newemail@example.com".to_string();
}).await?;

// Delete
users.delete(user_id).await?;
```

### Batch Operations

Efficient bulk operations:

```rust
// Batch insert
let new_todos = vec![
    Todo::new("Task 1"),
    Todo::new("Task 2"),
    Todo::new("Task 3"),
];
todos.insert_batch(new_todos).await?;

// Batch update
todos.update_where(
    |todo| todo.completed,
    |todo| todo.archived = true
).await?;

// Batch delete
todos.delete_where(|todo| todo.archived).await?;
```

## Change Tracking

### Change Events

Monitor data changes:

```rust
let _guard = users.on_change(|change_event| {
    match change_event {
        ChangeEvent::Insert(user) => {
            println!("New user added: {}", user.name);
        },
        ChangeEvent::Update(id, old_user, new_user) => {
            println!("User {} updated", id);
        },
        ChangeEvent::Delete(id) => {
            println!("User {} deleted", id);
        },
    }
});
```

### Reactive Sync

Automatic synchronization with external sources:

```rust
let synced_data = Data::<User>::new()
    .sync_with_api("https://api.example.com/users")
    .sync_interval(Duration::from_secs(30))
    .conflict_resolution(ConflictResolution::ServerWins);
```

## Performance Optimizations

### Lazy Loading

Load data on demand:

```rust
let lazy_users = Data::<User>::new()
    .lazy_loading(true)
    .page_size(20)
    .prefetch_threshold(5);
```

### Caching

Intelligent data caching:

```rust
let cached_data = Data::<Post>::new()
    .cache_policy(CachePolicy::LRU)
    .cache_size(100)
    .cache_ttl(Duration::from_secs(300));
```

## Dependencies

- `serde`: Serialization framework
- `futures`: Async utilities

## Example

```rust
use waterui_data::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BlogPost {
    pub id: Id,
    pub title: String,
    pub content: String,
    pub published: bool,
    pub created_at: DateTime<Utc>,
}

impl Schema for BlogPost {}

struct BlogView {
    posts: Data<BlogPost>,
    search_query: Binding<String>,
}

impl View for BlogView {
    fn body(self, env: &Environment) -> impl View {
        let filtered_posts = self.posts
            .filter(move |post| {
                post.title.contains(&self.search_query.get()) ||
                post.content.contains(&self.search_query.get())
            })
            .filter(|post| post.published);

        VStack::new([
            SearchBar::new(self.search_query.clone())
                .placeholder("Search posts..."),

            ForEach::new(filtered_posts, |post| {
                PostView::new(post)
            }),
        ])
    }
}

struct PostView {
    post: DataElement<BlogPost>,
}

impl View for PostView {
    fn body(self, env: &Environment) -> impl View {
        VStack::new([
            Text::new(self.post.data().title)
                .typography(Typography::HEADING_2),

            Text::new(self.post.data().content)
                .typography(Typography::BODY),

            Button::new("Toggle Published")
                .on_click(move || {
                    self.post.update(|post| {
                        post.published = !post.published;
                    });
                }),
        ])
        .padding(16.0)
    }
}
```
