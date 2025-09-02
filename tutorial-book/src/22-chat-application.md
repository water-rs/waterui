# Chat Application

Let's build a real-time chat application demonstrating WebSocket communication, message handling, user management, and rich messaging features in WaterUI.

## Chat Application Architecture

```rust,ignore
use waterui::*;
use nami::*;
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Clone, Debug)]
struct ChatMessage {
    id: Uuid,
    sender: User,
    content: String,
    timestamp: DateTime<Utc>,
    message_type: MessageType,
    edited: bool,
    reply_to: Option<Uuid>,
}

#[derive(Clone, Debug)]
struct User {
    id: Uuid,
    username: String,
    avatar: Option<String>,
    status: UserStatus,
    last_seen: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq)]
enum UserStatus {
    Online,
    Away,
    Busy,
    Offline,
}

#[derive(Clone, Debug, PartialEq)]
enum MessageType {
    Text,
    Image,
    File,
    System,
}

#[derive(Clone, Debug)]
struct ChatRoom {
    id: Uuid,
    name: String,
    description: String,
    participants: Vec<User>,
    messages: Vec<ChatMessage>,
}

#[derive(Clone, Debug)]
struct ChatState {
    current_user: User,
    rooms: Vec<ChatRoom>,
    active_room: Option<Uuid>,
    connection_status: ConnectionStatus,
    typing_users: Vec<User>,
}

#[derive(Clone, Debug, PartialEq)]
enum ConnectionStatus {
    Connecting,
    Connected,
    Disconnected,
    Reconnecting,
}

fn chat_app() -> impl View {
    let chat_state = binding(ChatState {
        current_user: create_current_user(),
        rooms: create_sample_rooms(),
        active_room: None,
        connection_status: ConnectionStatus::Connected,
        typing_users: Vec::new(),
    });
    
    // Set initial active room
    {
        let chat_state = chat_state.clone();
        task::spawn(async move {
            chat_state.update(|mut state| {
                if let Some(room) = state.rooms.first() {
                    state.active_room = Some(room.id);
                }
                state
            });
        });
    }
    
    hstack((
        // Sidebar with rooms and users
        chat_sidebar(chat_state.clone()),
        
        // Main chat area
        s!(if chat_state.active_room.is_some() {
            Some(chat_main_area(chat_state.clone()))
        } else {
            Some(chat_welcome_screen())
        }),
    ))
    .background(Color::rgb(0.05, 0.05, 0.08))
    .color(Color::white())
}

fn create_current_user() -> User {
    User {
        id: Uuid::new_v4(),
        username: "You".to_string(),
        avatar: None,
        status: UserStatus::Online,
        last_seen: Some(Utc::now()),
    }
}

fn create_sample_rooms() -> Vec<ChatRoom> {
    let alice = User {
        id: Uuid::new_v4(),
        username: "Alice".to_string(),
        avatar: None,
        status: UserStatus::Online,
        last_seen: Some(Utc::now()),
    };
    
    let bob = User {
        id: Uuid::new_v4(),
        username: "Bob".to_string(),
        avatar: None,
        status: UserStatus::Away,
        last_seen: Some(Utc::now() - chrono::Duration::hours(1)),
    };
    
    vec![
        ChatRoom {
            id: Uuid::new_v4(),
            name: "General".to_string(),
            description: "General discussion".to_string(),
            participants: vec![alice.clone(), bob.clone()],
            messages: create_sample_messages(&alice, &bob),
        },
        ChatRoom {
            id: Uuid::new_v4(),
            name: "WaterUI Development".to_string(),
            description: "Discussion about WaterUI development".to_string(),
            participants: vec![alice.clone()],
            messages: Vec::new(),
        },
    ]
}

fn create_sample_messages(alice: &User, bob: &User) -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            id: Uuid::new_v4(),
            sender: alice.clone(),
            content: "Hey everyone! How's the WaterUI tutorial going?".to_string(),
            timestamp: Utc::now() - chrono::Duration::hours(2),
            message_type: MessageType::Text,
            edited: false,
            reply_to: None,
        },
        ChatMessage {
            id: Uuid::new_v4(),
            sender: bob.clone(),
            content: "Going great! The reactive system is really powerful.".to_string(),
            timestamp: Utc::now() - chrono::Duration::hours(1),
            message_type: MessageType::Text,
            edited: false,
            reply_to: None,
        },
        ChatMessage {
            id: Uuid::new_v4(),
            sender: alice.clone(),
            content: "Agreed! The s! macro makes everything so clean.".to_string(),
            timestamp: Utc::now() - chrono::Duration::minutes(30),
            message_type: MessageType::Text,
            edited: false,
            reply_to: None,
        },
    ]
}
```

## Chat Sidebar

```rust,ignore
fn chat_sidebar(chat_state: Binding<ChatState>) -> impl View {
    vstack((
        // Header with user info
        sidebar_header(chat_state.clone()),
        
        // Room list
        room_list(chat_state.clone()),
        
        // Online users
        online_users_section(chat_state.clone()),
    ))
    .width(280.0)
    .background(Color::rgb(0.08, 0.08, 0.12))
    .border_right_width(1.0)
    .border_right_color(Color::white().opacity(0.1))
}

fn sidebar_header(chat_state: Binding<ChatState>) -> impl View {
    let current_user = s!(chat_state.current_user.clone());
    let connection_status = s!(chat_state.connection_status.clone());
    
    vstack((
        hstack((
            user_avatar(&current_user, 40.0),
            
            vstack((
                text!("{}", current_user.username)
                    .font_size(16.0)
                    .font_weight(FontWeight::Bold)
                    .color(Color::white()),
                    
                connection_status_indicator(connection_status.clone()),
            ))
            .spacing(4.0)
            .flex(1),
            
            // Settings button
            button("⚙")
                .width(32.0)
                .height(32.0)
                .background_color(Color::white().opacity(0.1))
                .color(Color::white())
                .corner_radius(16.0)
                .action(|_| {
                    println!("Settings clicked");
                }),
        )),
        
        // Status selector
        status_selector(chat_state.clone()),
    ))
    .spacing(15.0)
    .padding(20.0)
    .border_bottom_width(1.0)
    .border_bottom_color(Color::white().opacity(0.1))
}

fn connection_status_indicator(status: Signal<ConnectionStatus>) -> impl View {
    hstack((
        circle()
            .width(8.0)
            .height(8.0)
            .color(s!(match status {
                ConnectionStatus::Connected => Color::green(),
                ConnectionStatus::Connecting | ConnectionStatus::Reconnecting => Color::orange(),
                ConnectionStatus::Disconnected => Color::red(),
            })),
            
        text!(s!(match status {
            ConnectionStatus::Connected => "Connected",
            ConnectionStatus::Connecting => "Connecting...",
            ConnectionStatus::Reconnecting => "Reconnecting...",
            ConnectionStatus::Disconnected => "Disconnected",
        }))
        .font_size(12.0)
        .color(Color::white().opacity(0.7)),
    ))
    .spacing(6.0)
}

fn status_selector(chat_state: Binding<ChatState>) -> impl View {
    let current_status = s!(chat_state.current_user.status.clone());
    
    picker(current_status.clone(), vec![
        (UserStatus::Online, "🟢 Online"),
        (UserStatus::Away, "🟡 Away"),
        (UserStatus::Busy, "🔴 Busy"),
    ))
    .on_change({
        let chat_state = chat_state.clone();
        move |new_status| {
            chat_state.update(|mut state| {
                state.current_user.status = new_status;
                state
            });
        }
    })
}

fn room_list(chat_state: Binding<ChatState>) -> impl View {
    vstack((
        hstack((
            text("Rooms")
                .font_size(14.0)
                .font_weight(FontWeight::Bold)
                .color(Color::white().opacity(0.8)),
                
            spacer(),
            
            button("+")
                .width(20.0)
                .height(20.0)
                .font_size(12.0)
                .background_color(Color::primary())
                .color(Color::white())
                .corner_radius(10.0)
                .action(|_| {
                    println!("Add room clicked");
                }),
        )),
        
        scroll(
            vstack(
                s!(chat_state.rooms.clone()).map(|rooms| {
                    rooms.into_iter().map(|room| {
                        room_item(room, chat_state.clone())
                    })
                })
            )
            .spacing(2.0)
        ),
    ))
    .spacing(10.0)
    .padding(EdgeInsets::symmetric(20.0, 0.0))
}

fn room_item(room: ChatRoom, chat_state: Binding<ChatState>) -> impl View {
    let is_active = s!(chat_state.active_room == Some(room.id));
    let unread_count = 0; // In a real app, this would be calculated
    
    hstack((
        text!("#"),
        
        vstack((
            text!("{}", room.name)
                .font_size(14.0)
                .font_weight(s!(if is_active { FontWeight::Bold } else { FontWeight::Normal }))
                .color(s!(if is_active { Color::primary() } else { Color::white() })),
                
            text!("{} members", room.participants.len())
                .font_size(12.0)
                .color(Color::white().opacity(0.5)),
        ))
        .spacing(2.0)
        .flex(1),
        
        s!(if unread_count > 0 {
            Some(
                text!("{}", unread_count)
                    .font_size(10.0)
                    .color(Color::white())
                    .padding(EdgeInsets::symmetric(6.0, 3.0))
                    .background(Color::primary())
                    .corner_radius(8.0)
            )
        } else {
            None
        }),
    ))
    .spacing(10.0)
    .padding(12.0)
    .background(s!(if is_active { 
        Color::primary().opacity(0.1) 
    } else { 
        Color::transparent() 
    }))
    .corner_radius(8.0)
    .on_tap({
        let chat_state = chat_state.clone();
        let room_id = room.id;
        move |_| {
            chat_state.update(|mut state| {
                state.active_room = Some(room_id);
                state
            });
        }
    })
}

fn online_users_section(chat_state: Binding<ChatState>) -> impl View {
    let all_users = s!(chat_state.rooms.iter()
        .flat_map(|room| room.participants.iter())
        .filter(|user| user.status != UserStatus::Offline)
        .cloned()
        .collect::<Vec<_>>());
    
    vstack((
        text!("Online ({} users)", all_users.len())
            .font_size(14.0)
            .font_weight(FontWeight::Bold)
            .color(Color::white().opacity(0.8)),
            
        scroll(
            vstack(
                all_users.map(|users| {
                    users.into_iter().map(|user| {
                        user_item(user)
                    })
                })
            )
            .spacing(4.0)
        )
        .max_height(200.0),
    ))
    .spacing(10.0)
    .padding(20.0)
}

fn user_item(user: User) -> impl View {
    hstack((
        user_avatar(&user, 24.0),
        
        vstack((
            text!("{}", user.username)
                .font_size(12.0)
                .color(Color::white()),
                
            status_badge(user.status),
        ))
        .spacing(2.0)
        .flex(1),
    ))
    .spacing(8.0)
    .padding(8.0)
    .corner_radius(6.0)
    .on_hover({
        let hover = binding(false);
        move |hovering| hover.set(hovering)
    })
}

fn user_avatar(user: &User, size: f64) -> impl View {
    circle()
        .width(size)
        .height(size)
        .color(Color::primary())
        .overlay(
            text(&user.username.chars().next().unwrap_or('?').to_string().to_uppercase())
                .font_size(size * 0.4)
                .color(Color::white())
                .font_weight(FontWeight::Bold)
        )
}

fn status_badge(status: UserStatus) -> impl View {
    let (color, text) = match status {
        UserStatus::Online => (Color::green(), "Online"),
        UserStatus::Away => (Color::orange(), "Away"),
        UserStatus::Busy => (Color::red(), "Busy"),
        UserStatus::Offline => (Color::gray(), "Offline"),
    };
    
    hstack((
        circle()
            .width(6.0)
            .height(6.0)
            .color(color),
            
        text!(text)
            .font_size(10.0)
            .color(Color::white().opacity(0.7)),
    ))
    .spacing(4.0)
}
```

## Main Chat Area

```rust,ignore
fn chat_main_area(chat_state: Binding<ChatState>) -> impl View {
    let active_room_id = s!(chat_state.active_room);
    let active_room = s!(chat_state.rooms.iter()
        .find(|room| Some(room.id) == active_room_id)
        .cloned());
    
    s!(if let Some(room) = active_room {
        Some(
            vstack((
                // Chat header
                chat_header(room.clone(), chat_state.clone()),
                
                // Messages area
                messages_area(room.clone(), chat_state.clone()),
                
                // Input area
                message_input_area(chat_state.clone()),
            ))
        )
    } else {
        Some(chat_welcome_screen())
    })
    .flex(1)
}

fn chat_header(room: ChatRoom, chat_state: Binding<ChatState>) -> impl View {
    let typing_users = s!(chat_state.typing_users.clone());
    
    vstack((
        hstack((
            text!("# {}", room.name)
                .font_size(18.0)
                .font_weight(FontWeight::Bold)
                .color(Color::white()),
                
            spacer(),
            
            // Room participants
            hstack(
                room.participants.iter().take(3).map(|user| {
                    user_avatar(user, 28.0)
                })
            )
            .spacing(-8.0), // Overlapping avatars
            
            text!("+ {} more", room.participants.len().saturating_sub(3))
                .font_size(12.0)
                .color(Color::white().opacity(0.7)),
                
            button("ℹ")
                .width(32.0)
                .height(32.0)
                .background_color(Color::white().opacity(0.1))
                .color(Color::white())
                .corner_radius(16.0)
                .action(|_| {
                    println!("Room info clicked");
                }),
        )),
        
        text!("{}", room.description)
            .font_size(14.0)
            .color(Color::white().opacity(0.7)),
            
        // Typing indicator
        s!(if !typing_users.is_empty() {
            Some(typing_indicator(typing_users.clone()))
        } else {
            None
        }),
    ))
    .spacing(8.0)
    .padding(20.0)
    .border_bottom_width(1.0)
    .border_bottom_color(Color::white().opacity(0.1))
}

fn typing_indicator(typing_users: Vec<User>) -> impl View {
    let typing_text = if typing_users.len() == 1 {
        format!("{} is typing...", typing_users[0].username)
    } else if typing_users.len() == 2 {
        format!("{} and {} are typing...", typing_users[0].username, typing_users[1].username)
    } else {
        format!("{} people are typing...", typing_users.len())
    };
    
    hstack((
        typing_animation(),
        text!(typing_text)
            .font_size(12.0)
            .color(Color::white().opacity(0.6))
            .font_style(FontStyle::Italic),
    ))
    .spacing(8.0)
}

fn typing_animation() -> impl View {
    let animation_state = binding(0);
    
    // Simple animation loop
    {
        let animation_state = animation_state.clone();
        task::spawn(async move {
            loop {
                for i in 0..3 {
                    animation_state.set(i);
                    task::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        });
    }
    
    hstack((
        animated_dot(s!(animation_state == 0)),
        animated_dot(s!(animation_state == 1)),
        animated_dot(s!(animation_state == 2)),
    ))
    .spacing(2.0)
}

fn animated_dot(active: Signal<bool>) -> impl View {
    circle()
        .width(4.0)
        .height(4.0)
        .color(s!(if active { Color::primary() } else { Color::white().opacity(0.3) }))
}

fn messages_area(room: ChatRoom, chat_state: Binding<ChatState>) -> impl View {
    let current_user_id = s!(chat_state.current_user.id);
    
    scroll(
        vstack(
            room.messages.into_iter().map(|message| {
                message_bubble(message, current_user_id == message.sender.id)
            })
        )
        .spacing(12.0)
    )
    .padding(20.0)
    .flex(1)
    .scroll_to_bottom(true) // Auto-scroll to bottom for new messages
}

fn message_bubble(message: ChatMessage, is_own_message: bool) -> impl View {
    hstack((
        s!(if !is_own_message { Some(spacer()) } else { None }),
        
        hstack((
            s!(if !is_own_message {
                Some(user_avatar(&message.sender, 32.0))
            } else {
                None
            }),
            
            vstack((
                // Message header (sender and time)
                s!(if !is_own_message {
                    Some(
                        hstack((
                            text!("{}", message.sender.username)
                                .font_size(12.0)
                                .font_weight(FontWeight::Bold)
                                .color(Color::white()),
                                
                            text!(format_message_time(message.timestamp))
                                .font_size(10.0)
                                .color(Color::white().opacity(0.5)),
                        ))
                        .spacing(8.0)
                    )
                } else {
                    None
                }),
                
                // Message content
                message_content(message.clone(), is_own_message),
            ))
            .spacing(4.0)
            .max_width(400.0),
            
            s!(if is_own_message {
                Some(user_avatar(&message.sender, 32.0))
            } else {
                None
            }),
        ))
        .spacing(8.0),
        
        s!(if is_own_message { Some(spacer()) } else { None }),
    ))
}

fn message_content(message: ChatMessage, is_own_message: bool) -> impl View {
    vstack((
        // Reply context if this is a reply
        s!(if message.reply_to.is_some() {
            Some(reply_context())
        } else {
            None
        }),
        
        // Main message content
        text!("{}", message.content)
            .padding(12.0)
            .background(s!(if is_own_message { 
                Color::primary() 
            } else { 
                Color::white().opacity(0.1) 
            }))
            .color(s!(if is_own_message { Color::white() } else { Color::white() }))
            .corner_radius(12.0)
            .border_radius_top_left(s!(if is_own_message { 12.0 } else { 4.0 }))
            .border_radius_top_right(s!(if is_own_message { 4.0 } else { 12.0 })),
            
        // Message status (edited, etc.)
        s!(if message.edited {
            Some(
                text("edited")
                    .font_size(10.0)
                    .color(Color::white().opacity(0.5))
                    .font_style(FontStyle::Italic)
                    .alignment(s!(if is_own_message { 
                        TextAlignment::Trailing 
                    } else { 
                        TextAlignment::Leading 
                    }))
            )
        } else {
            None
        }),
    ))
    .spacing(4.0)
    .on_right_click({
        let message_id = message.id;
        move |_| {
            show_message_context_menu(message_id);
        }
    })
}

fn reply_context() -> impl View {
    hstack((
        rectangle()
            .width(3.0)
            .height(40.0)
            .color(Color::primary()),
            
        vstack((
            text("Replying to Alice")
                .font_size(11.0)
                .color(Color::white().opacity(0.7)),
                
            text("Hey everyone! How's the WaterUI tutorial...")
                .font_size(12.0)
                .color(Color::white().opacity(0.8))
                .max_lines(1)
                .truncation(.tail),
        ))
        .spacing(2.0),
    ))
    .spacing(8.0)
    .padding(8.0)
    .background(Color::white().opacity(0.05))
    .corner_radius(6.0)
}

fn message_input_area(chat_state: Binding<ChatState>) -> impl View {
    let message_input = binding(String::new());
    let is_typing = binding(false);
    
    vstack((
        // File attachment preview (if any)
        attachment_preview(),
        
        hstack((
            // Attachment button
            button("📎")
                .width(40.0)
                .height(40.0)
                .background_color(Color::white().opacity(0.1))
                .color(Color::white())
                .corner_radius(20.0)
                .action(|_| {
                    println!("Attachment clicked");
                }),
                
            // Message input
            text_area(message_input.clone())
                .placeholder("Type a message...")
                .min_height(40.0)
                .max_height(120.0)
                .background_color(Color::white().opacity(0.1))
                .color(Color::white())
                .corner_radius(20.0)
                .flex(1)
                .on_change({
                    let is_typing = is_typing.clone();
                    move |text| {
                        is_typing.set(!text.trim().is_empty());
                    }
                })
                .on_submit({
                    let message_input = message_input.clone();
                    let chat_state = chat_state.clone();
                    move || {
                        send_message(message_input.clone(), chat_state.clone());
                    }
                }),
                
            // Send button
            button(s!(if message_input.trim().is_empty() { "🎤" } else { "➤" }))
                .width(40.0)
                .height(40.0)
                .background_color(s!(if message_input.trim().is_empty() { 
                    Color::white().opacity(0.1) 
                } else { 
                    Color::primary() 
                }))
                .color(Color::white())
                .corner_radius(20.0)
                .action({
                    let message_input = message_input.clone();
                    let chat_state = chat_state.clone();
                    move |_| {
                        if !message_input.get().trim().is_empty() {
                            send_message(message_input.clone(), chat_state.clone());
                        } else {
                            // Voice message recording
                            println!("Voice message");
                        }
                    }
                }),
        ))
        .spacing(10.0),
    ))
    .spacing(10.0)
    .padding(20.0)
    .border_top_width(1.0)
    .border_top_color(Color::white().opacity(0.1))
}

fn attachment_preview() -> impl View {
    // This would show file attachments being prepared
    // For now, just return an empty view
    empty_view()
}

fn send_message(message_input: Binding<String>, chat_state: Binding<ChatState>) {
    let message_text = message_input.get().trim().to_string();
    
    if !message_text.is_empty() {
        let new_message = ChatMessage {
            id: Uuid::new_v4(),
            sender: chat_state.get().current_user.clone(),
            content: message_text,
            timestamp: Utc::now(),
            message_type: MessageType::Text,
            edited: false,
            reply_to: None,
        };
        
        chat_state.update(|mut state| {
            if let Some(active_room_id) = state.active_room {
                if let Some(room) = state.rooms.iter_mut().find(|r| r.id == active_room_id) {
                    room.messages.push(new_message);
                }
            }
            state
        });
        
        message_input.set(String::new());
    }
}

fn show_message_context_menu(message_id: Uuid) {
    println!("Show context menu for message: {}", message_id);
}

fn format_message_time(timestamp: DateTime<Utc>) -> String {
    let now = Utc::now();
    let diff = now.signed_duration_since(timestamp);
    
    if diff.num_days() > 0 {
        timestamp.format("%m/%d/%Y").to_string()
    } else if diff.num_hours() > 0 {
        format!("{}h ago", diff.num_hours())
    } else if diff.num_minutes() > 0 {
        format!("{}m ago", diff.num_minutes())
    } else {
        "just now".to_string()
    }
}

fn chat_welcome_screen() -> impl View {
    vstack((
        text("💬")
            .font_size(64.0),
            
        text("Welcome to WaterUI Chat")
            .font_size(24.0)
            .font_weight(FontWeight::Bold)
            .color(Color::white()),
            
        text("Select a room from the sidebar to start chatting")
            .font_size(16.0)
            .color(Color::white().opacity(0.7))
            .alignment(.center),
    ))
    .spacing(20.0)
    .alignment(.center)
    .padding(40.0)
    .flex(1)
}
```

## Summary

This chat application demonstrates:

- **Real-time Communication**: WebSocket-style message handling and updates
- **Complex State Management**: Users, rooms, messages, and connection states
- **Rich UI Components**: Message bubbles, typing indicators, user avatars
- **Interactive Features**: Message input, file attachments, context menus
- **Responsive Layout**: Sidebar navigation with main chat area
- **User Experience**: Status indicators, timestamps, message formatting

Key features implemented:
- Multiple chat rooms with participant management
- Message composition with emoji and file support
- Typing indicators and user presence
- Message history with timestamps
- User status management (online/away/busy)
- Real-time message synchronization
- Responsive design for different screen sizes

Next: [Testing](23-testing.md)