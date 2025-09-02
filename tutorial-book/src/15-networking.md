# Networking

Modern applications require robust networking capabilities. WaterUI provides excellent support for HTTP requests, API integration, and async data loading with reactive state management.

## HTTP Client Setup

### Basic HTTP Requests

```rust,ignore
use waterui::*;
use nami::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiResponse<T> {
    data: T,
    success: bool,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct User {
    id: u32,
    name: String,
    email: String,
    avatar_url: String,
}

fn basic_http_demo() -> impl View {
    let loading = binding(false);
    let users = binding(Vec::<User>::new());
    let error = binding(None::<String>);
    
    vstack((
        text("HTTP Requests Demo").font_size(20.0),
        
        button("Fetch Users")
            .disabled(loading.get())
            .action({
                let loading = loading.clone();
                let users = users.clone();
                let error = error.clone();
                move |_| {
                    fetch_users(loading.clone(), users.clone(), error.clone());
                }
            }),
            
        s!(if loading {
            Some(
                hstack((
                    progress_indicator(),
                    text("Loading users..."),
                ))
                .spacing(10.0)
            )
        } else {
            None
        }),
        
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
        
        scroll(
            vstack(
                users.get().map(|users| {
                    users.into_iter().map(|user| {
                        user_card(user)
                    })
                })
            )
            .spacing(10.0)
        )
        .max_height(400.0),
    ))
    .spacing(15.0)
    .padding(20.0)
}

async fn fetch_users(
    loading: Binding<bool>,
    users: Binding<Vec<User>>,
    error: Binding<Option<String>>,
) {
    loading.set(true);
    error.set(None);
    
    let result = http_client::get("https://jsonplaceholder.typicode.com/users")
        .await;
        
    match result {
        Ok(response) => {
            match response.json::<Vec<User>>().await {
                Ok(user_list) => {
                    users.set(user_list);
                }
                Err(e) => {
                    error.set(Some(format!("Failed to parse response: {}", e)));
                }
            }
        }
        Err(e) => {
            error.set(Some(format!("Request failed: {}", e)));
        }
    }
    
    loading.set(false);
}

fn user_card(user: User) -> impl View {
    hstack((
        async_image(user.avatar_url)
            .width(50.0)
            .height(50.0)
            .corner_radius(25.0),
            
        vstack((
            text!("{}", user.name)
                .font_weight(FontWeight::Medium),
            text!("{}", user.email)
                .font_size(14.0)
                .color(Color::secondary()),
        ))
        .spacing(5.0)
        .flex(1),
    ))
    .spacing(15.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(10.0)
}
```

### Advanced HTTP Client

```rust,ignore
struct HttpClient {
    base_url: String,
    headers: HashMap<String, String>,
    timeout: Duration,
}

impl HttpClient {
    fn new(base_url: String) -> Self {
        Self {
            base_url,
            headers: HashMap::new(),
            timeout: Duration::from_secs(30),
        }
    }
    
    fn with_header(mut self, key: String, value: String) -> Self {
        self.headers.insert(key, value);
        self
    }
    
    fn with_auth_token(self, token: String) -> Self {
        self.with_header("Authorization".to_string(), format!("Bearer {}", token))
    }
    
    fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    
    async fn get<T>(&self, endpoint: &str) -> Result<T, HttpError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let url = format!("{}{}", self.base_url, endpoint);
        
        let response = reqwest::Client::new()
            .get(&url)
            .headers(self.build_headers())
            .timeout(self.timeout)
            .send()
            .await
            .map_err(HttpError::Request)?;
            
        if response.status().is_success() {
            response.json().await.map_err(HttpError::Parse)
        } else {
            Err(HttpError::Status(response.status().as_u16()))
        }
    }
    
    async fn post<T, R>(&self, endpoint: &str, body: &T) -> Result<R, HttpError>
    where
        T: Serialize,
        R: for<'de> Deserialize<'de>,
    {
        let url = format!("{}{}", self.base_url, endpoint);
        
        let response = reqwest::Client::new()
            .post(&url)
            .headers(self.build_headers())
            .json(body)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(HttpError::Request)?;
            
        if response.status().is_success() {
            response.json().await.map_err(HttpError::Parse)
        } else {
            Err(HttpError::Status(response.status().as_u16()))
        }
    }
    
    fn build_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        
        for (key, value) in &self.headers {
            if let (Ok(name), Ok(val)) = (
                reqwest::header::HeaderName::from_str(key),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                headers.insert(name, val);
            }
        }
        
        headers
    }
}

#[derive(Debug)]
enum HttpError {
    Request(reqwest::Error),
    Parse(reqwest::Error),
    Status(u16),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Request(e) => write!(f, "Request error: {}", e),
            HttpError::Parse(e) => write!(f, "Parse error: {}", e),
            HttpError::Status(code) => write!(f, "HTTP error: {}", code),
        }
    }
}

fn advanced_http_demo() -> impl View {
    let client = HttpClient::new("https://api.github.com".to_string())
        .with_header("User-Agent".to_string(), "WaterUI-Demo".to_string())
        .with_timeout(Duration::from_secs(10));
        
    let loading = binding(false);
    let repos = binding(Vec::<GitHubRepo>::new());
    let error = binding(None::<String>);
    let username = binding("octocat".to_string());
    
    vstack((
        text("GitHub API Demo").font_size(20.0),
        
        hstack((
            text_field(username.clone())
                .placeholder("GitHub username")
                .on_submit({
                    let client = client.clone();
                    let loading = loading.clone();
                    let repos = repos.clone();
                    let error = error.clone();
                    let username = username.clone();
                    move || {
                        fetch_github_repos(
                            client.clone(), 
                            username.get(),
                            loading.clone(),
                            repos.clone(),
                            error.clone()
                        );
                    }
                }),
                
            button("Fetch Repos")
                .disabled(loading.get())
                .action({
                    let client = client.clone();
                    let loading = loading.clone();
                    let repos = repos.clone();
                    let error = error.clone();
                    let username = username.clone();
                    move |_| {
                        fetch_github_repos(
                            client.clone(),
                            username.get(),
                            loading.clone(),
                            repos.clone(),
                            error.clone()
                        );
                    }
                }),
        ))
        .spacing(10.0),
        
        s!(if loading {
            Some(loading_indicator())
        } else {
            None
        }),
        
        s!(if let Some(err) = &error {
            Some(error_display(err.clone()))
        } else {
            None
        }),
        
        repo_list(repos.clone()),
    ))
    .spacing(15.0)
    .padding(20.0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubRepo {
    id: u32,
    name: String,
    full_name: String,
    description: Option<String>,
    stars: u32,
    language: Option<String>,
    html_url: String,
}

async fn fetch_github_repos(
    client: HttpClient,
    username: String,
    loading: Binding<bool>,
    repos: Binding<Vec<GitHubRepo>>,
    error: Binding<Option<String>>,
) {
    loading.set(true);
    error.set(None);
    
    let endpoint = format!("/users/{}/repos?sort=stars&per_page=10", username);
    
    match client.get::<Vec<GitHubRepo>>(&endpoint).await {
        Ok(repo_list) => {
            repos.set(repo_list);
        }
        Err(e) => {
            error.set(Some(e.to_string()));
        }
    }
    
    loading.set(false);
}

fn repo_list(repos: Binding<Vec<GitHubRepo>>) -> impl View {
    scroll(
        vstack(
            repos.get().map(|repos| {
                repos.into_iter().map(|repo| {
                    repo_card(repo)
                })
            })
        )
        .spacing(10.0)
    )
    .max_height(500.0)
}

fn repo_card(repo: GitHubRepo) -> impl View {
    vstack((
        hstack((
            text!("{}", repo.name)
                .font_size(16.0)
                .font_weight(FontWeight::Medium),
                
            spacer(),
            
            hstack((
                text("⭐"),
                text!("{}", repo.stars),
            ))
            .spacing(2.0),
        )),
        
        s!(if let Some(desc) = &repo.description {
            Some(
                text!("{}", desc)
                    .font_size(14.0)
                    .color(Color::secondary())
                    .line_height(1.4)
            )
        } else {
            None
        }),
        
        hstack((
            s!(if let Some(lang) = &repo.language {
                Some(
                    text!("{}", lang)
                        .font_size(12.0)
                        .padding(EdgeInsets::symmetric(6.0, 3.0))
                        .background(Color::primary().opacity(0.1))
                        .corner_radius(4.0)
                )
            } else {
                None
            }),
            
            spacer(),
            
            link_button("View on GitHub", repo.html_url),
        )),
    ))
    .spacing(10.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(10.0)
}
```

## Async Data Loading

### Data Loading States

```rust,ignore
#[derive(Clone, Debug)]
enum LoadingState<T, E = String> {
    Idle,
    Loading,
    Success(T),
    Error(E),
}

impl<T, E> LoadingState<T, E> {
    fn is_loading(&self) -> bool {
        matches!(self, LoadingState::Loading)
    }
    
    fn is_success(&self) -> bool {
        matches!(self, LoadingState::Success(_))
    }
    
    fn is_error(&self) -> bool {
        matches!(self, LoadingState::Error(_))
    }
    
    fn data(&self) -> Option<&T> {
        match self {
            LoadingState::Success(data) => Some(data),
            _ => None,
        }
    }
}

fn async_loading_demo() -> impl View {
    let user_state = binding(LoadingState::<User>::Idle);
    let posts_state = binding(LoadingState::<Vec<Post>>::Idle);
    let selected_user_id = binding(1u32);
    
    vstack((
        text("Async Data Loading").font_size(20.0),
        
        // User selection
        hstack((
            text("User ID:"),
            number_field(s!(selected_user_id as f64))
                .min(1.0)
                .max(10.0)
                .on_change({
                    let selected_user_id = selected_user_id.clone();
                    move |value| selected_user_id.set(value as u32)
                }),
            button("Load User")
                .disabled(user_state.get().is_loading())
                .action({
                    let user_state = user_state.clone();
                    let selected_user_id = selected_user_id.clone();
                    move |_| {
                        load_user(selected_user_id.get(), user_state.clone());
                    }
                }),
        ))
        .spacing(10.0),
        
        // User display
        render_loading_state(user_state.clone(), render_user, render_user_error),
        
        // Posts section
        s!(if user_state.get().is_success() {
            Some(
                vstack((
                    button("Load User Posts")
                        .disabled(posts_state.get().is_loading())
                        .action({
                            let posts_state = posts_state.clone();
                            let selected_user_id = selected_user_id.clone();
                            move |_| {
                                load_user_posts(selected_user_id.get(), posts_state.clone());
                            }
                        }),
                        
                    render_loading_state(posts_state.clone(), render_posts, render_posts_error),
                ))
                .spacing(15.0)
            )
        } else {
            None
        }),
    ))
    .spacing(15.0)
    .padding(20.0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Post {
    id: u32,
    title: String,
    body: String,
    user_id: u32,
}

fn render_loading_state<T, F, G>(
    state: Binding<LoadingState<T>>,
    render_success: F,
    render_error: G,
) -> impl View
where
    T: Clone + 'static,
    F: Fn(T) -> Box<dyn View> + 'static,
    G: Fn(String) -> Box<dyn View> + 'static,
{
    s!(match state {
        LoadingState::Idle => None,
        LoadingState::Loading => Some(Box::new(loading_indicator()) as Box<dyn View>),
        LoadingState::Success(data) => Some(render_success(data)),
        LoadingState::Error(error) => Some(render_error(error)),
    })
}

async fn load_user(user_id: u32, state: Binding<LoadingState<User>>) {
    state.set(LoadingState::Loading);
    
    match fetch_user(user_id).await {
        Ok(user) => state.set(LoadingState::Success(user)),
        Err(e) => state.set(LoadingState::Error(e.to_string())),
    }
}

async fn load_user_posts(user_id: u32, state: Binding<LoadingState<Vec<Post>>>) {
    state.set(LoadingState::Loading);
    
    match fetch_user_posts(user_id).await {
        Ok(posts) => state.set(LoadingState::Success(posts)),
        Err(e) => state.set(LoadingState::Error(e.to_string())),
    }
}

async fn fetch_user(user_id: u32) -> Result<User, HttpError> {
    let client = HttpClient::new("https://jsonplaceholder.typicode.com".to_string());
    client.get(&format!("/users/{}", user_id)).await
}

async fn fetch_user_posts(user_id: u32) -> Result<Vec<Post>, HttpError> {
    let client = HttpClient::new("https://jsonplaceholder.typicode.com".to_string());
    client.get(&format!("/users/{}/posts", user_id)).await
}

fn render_user(user: User) -> Box<dyn View> {
    Box::new(
        vstack((
            text!("User: {}", user.name)
                .font_size(18.0)
                .font_weight(FontWeight::Medium),
            text!("Email: {}", user.email)
                .color(Color::secondary()),
        ))
        .spacing(5.0)
        .padding(15.0)
        .background(Color::light_gray())
        .corner_radius(8.0)
    )
}

fn render_user_error(error: String) -> Box<dyn View> {
    Box::new(
        text!("Failed to load user: {}", error)
            .color(Color::red())
            .padding(10.0)
            .background(Color::red().opacity(0.1))
            .corner_radius(8.0)
    )
}

fn render_posts(posts: Vec<Post>) -> Box<dyn View> {
    Box::new(
        scroll(
            vstack(
                posts.into_iter().map(|post| {
                    post_card(post)
                })
            )
            .spacing(10.0)
        )
        .max_height(300.0)
    )
}

fn render_posts_error(error: String) -> Box<dyn View> {
    Box::new(
        text!("Failed to load posts: {}", error)
            .color(Color::red())
            .padding(10.0)
            .background(Color::red().opacity(0.1))
            .corner_radius(8.0)
    )
}

fn post_card(post: Post) -> impl View {
    vstack((
        text!("{}", post.title)
            .font_weight(FontWeight::Medium)
            .line_height(1.3),
        text!("{}", post.body)
            .font_size(14.0)
            .color(Color::secondary())
            .line_height(1.4),
    ))
    .spacing(8.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(8.0)
}
```

## Real-time Communication

### WebSocket Integration

```rust,ignore
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Clone, Debug)]
enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

fn websocket_demo() -> impl View {
    let connection_state = binding(ConnectionState::Disconnected);
    let messages = binding(Vec::<ChatMessage>::new());
    let new_message = binding(String::new());
    let websocket_url = binding("wss://echo.websocket.org".to_string());
    
    vstack((
        text("WebSocket Demo").font_size(20.0),
        
        // Connection controls
        connection_controls(connection_state.clone(), websocket_url.clone()),
        
        // Message input
        s!(if let ConnectionState::Connected = connection_state {
            Some(message_input(new_message.clone(), messages.clone()))
        } else {
            None
        }),
        
        // Messages display
        messages_display(messages.clone()),
    ))
    .spacing(15.0)
    .padding(20.0)
}

#[derive(Clone, Debug)]
struct ChatMessage {
    id: u32,
    content: String,
    timestamp: String,
    is_sent: bool,
}

fn connection_controls(
    state: Binding<ConnectionState>,
    url: Binding<String>
) -> impl View {
    vstack((
        text_field(url.clone())
            .label("WebSocket URL")
            .disabled(s!(matches!(state, ConnectionState::Connected | ConnectionState::Connecting))),
            
        hstack((
            connection_status(state.clone()),
            
            spacer(),
            
            s!(match state {
                ConnectionState::Disconnected | ConnectionState::Error(_) => {
                    Some(
                        button("Connect")
                            .action({
                                let state = state.clone();
                                let url = url.clone();
                                move |_| {
                                    connect_websocket(url.get(), state.clone());
                                }
                            })
                    )
                }
                ConnectionState::Connected => {
                    Some(
                        button("Disconnect")
                            .action({
                                let state = state.clone();
                                move |_| {
                                    state.set(ConnectionState::Disconnected);
                                }
                            })
                    )
                }
                ConnectionState::Connecting => {
                    Some(
                        button("Connecting...")
                            .disabled(true)
                    )
                }
            }),
        )),
    ))
    .spacing(10.0)
    .padding(15.0)
    .background(Color::light_gray())
    .corner_radius(8.0)
}

fn connection_status(state: Binding<ConnectionState>) -> impl View {
    hstack((
        connection_indicator(state.clone()),
        text!(s!(match state {
            ConnectionState::Disconnected => "Disconnected",
            ConnectionState::Connecting => "Connecting...",
            ConnectionState::Connected => "Connected",
            ConnectionState::Error(_) => "Error",
        })),
    ))
    .spacing(8.0)
}

fn connection_indicator(state: Binding<ConnectionState>) -> impl View {
    circle()
        .width(12.0)
        .height(12.0)
        .color(s!(match state {
            ConnectionState::Disconnected => Color::gray(),
            ConnectionState::Connecting => Color::orange(),
            ConnectionState::Connected => Color::green(),
            ConnectionState::Error(_) => Color::red(),
        }))
}

async fn connect_websocket(url: String, state: Binding<ConnectionState>) {
    state.set(ConnectionState::Connecting);
    
    match connect_async(&url).await {
        Ok((ws_stream, _)) => {
            state.set(ConnectionState::Connected);
            
            let (mut write, mut read) = ws_stream.split();
            
            // Handle incoming messages
            task::spawn({
                let state = state.clone();
                async move {
                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                println!("Received: {}", text);
                            }
                            Ok(Message::Close(_)) => {
                                state.set(ConnectionState::Disconnected);
                                break;
                            }
                            Err(e) => {
                                state.set(ConnectionState::Error(e.to_string()));
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            });
        }
        Err(e) => {
            state.set(ConnectionState::Error(e.to_string()));
        }
    }
}

fn message_input(
    new_message: Binding<String>,
    messages: Binding<Vec<ChatMessage>>
) -> impl View {
    hstack((
        text_field(new_message.clone())
            .placeholder("Type a message...")
            .flex(1)
            .on_submit({
                let new_message = new_message.clone();
                let messages = messages.clone();
                move || {
                    send_message(new_message.clone(), messages.clone());
                }
            }),
            
        button("Send")
            .disabled(s!(new_message.is_empty()))
            .action({
                let new_message = new_message.clone();
                let messages = messages.clone();
                move |_| {
                    send_message(new_message.clone(), messages.clone());
                }
            }),
    ))
    .spacing(10.0)
}

fn send_message(
    new_message: Binding<String>,
    messages: Binding<Vec<ChatMessage>>
) {
    let message_content = new_message.get();
    if !message_content.trim().is_empty() {
        let message = ChatMessage {
            id: messages.get().len() as u32 + 1,
            content: message_content,
            timestamp: chrono::Utc::now().format("%H:%M:%S").to_string(),
            is_sent: true,
        };
        
        messages.update(|mut msgs| {
            msgs.push(message);
            msgs
        });
        
        new_message.set(String::new());
    }
}

fn messages_display(messages: Binding<Vec<ChatMessage>>) -> impl View {
    scroll(
        vstack(
            messages.get().map(|msgs| {
                msgs.into_iter().map(|msg| {
                    message_bubble(msg)
                })
            })
        )
        .spacing(8.0)
    )
    .max_height(300.0)
}

fn message_bubble(message: ChatMessage) -> impl View {
    hstack((
        s!(if message.is_sent { Some(spacer()) } else { None }),
        
        vstack((
            text!("{}", message.content)
                .padding(12.0)
                .background(s!(if message.is_sent { 
                    Color::primary() 
                } else { 
                    Color::light_gray() 
                }))
                .color(s!(if message.is_sent { 
                    Color::white() 
                } else { 
                    Color::text() 
                }))
                .corner_radius(12.0),
                
            text!("{}", message.timestamp)
                .font_size(11.0)
                .color(Color::secondary())
                .alignment(s!(if message.is_sent { 
                    TextAlignment::Trailing 
                } else { 
                    TextAlignment::Leading 
                })),
        ))
        .spacing(4.0)
        .max_width(250.0),
        
        s!(if !message.is_sent { Some(spacer()) } else { None }),
    ))
}
```

## Summary

WaterUI's networking capabilities provide:

- **HTTP Client**: Full-featured HTTP client with authentication and error handling
- **Async Data Loading**: Reactive loading states with proper error handling
- **WebSocket Support**: Real-time communication with connection management
- **Request/Response Patterns**: Type-safe serialization and deserialization
- **Error Handling**: Comprehensive error types and user-friendly error display
- **Performance**: Efficient async operations with reactive UI updates

Key best practices:
- Use loading states to provide user feedback
- Implement proper error handling and display
- Structure HTTP clients with reusable configurations
- Handle network failures gracefully
- Use reactive bindings for automatic UI updates
- Test network operations thoroughly

Next: [Storage](16-storage.md)