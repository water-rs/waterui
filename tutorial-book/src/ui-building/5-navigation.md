# Navigation Components

Navigation is crucial for multi-screen applications. WaterUI provides powerful navigation components for building intuitive user interfaces with routing, tabs, and hierarchical navigation.

## Basic Navigation

### Navigation Stack

```rust,ignore
fn navigation_demo() -> impl View {
    let nav_controller = NavigationController::new();
    
    navigation_stack(nav_controller.clone(), home_screen())
}

fn home_screen() -> impl View {
    let nav = use_navigation();
    
    vstack((
        text("Home Screen").font_size(24.0),
        
        button("Go to Profile")
            .action({
                let nav = nav.clone();
                move |_| {
                    nav.push(profile_screen());
                }
            }),
            
        button("Go to Settings")
            .action({
                let nav = nav.clone();
                move |_| {
                    nav.push(settings_screen());
                }
            }),
    ))
    .spacing(15.0)
    .padding(20.0)
}

fn profile_screen() -> impl View {
    let nav = use_navigation();
    
    vstack((
        text("Profile Screen").font_size(24.0),
        
        "User: John Doe"
            .font_size(16.0)
            .color(Color::secondary()),
            
        button("Edit Profile")
            .action({
                let nav = nav.clone();
                move |_| {
                    nav.push(edit_profile_screen());
                }
            }),
            
        button("Back")
            .action({
                let nav = nav.clone();
                move |_| {
                    nav.pop();
                }
            }),
    ))
    .spacing(15.0)
    .padding(20.0)
}

fn edit_profile_screen() -> impl View {
    let nav = use_navigation();
    let name = binding("John Doe".to_string());
    
    vstack((
        text("Edit Profile").font_size(24.0),
        
        text_field(name.clone())
            .label("Name")
            .placeholder("Enter your name"),
            
        hstack((
            button("Cancel")
                .style(ButtonStyle::Secondary)
                .action({
                    let nav = nav.clone();
                    move |_| {
                        nav.pop();
                    }
                }),
                
            button("Save")
                .action({
                    let nav = nav.clone();
                    move |_| {
                        // Save logic here
                        nav.pop();
                    }
                }),
        ))
        .spacing(10.0),
    ))
    .spacing(15.0)
    .padding(20.0)
}
```

### Tab Navigation

```rust,ignore
fn tab_navigation_demo() -> impl View {
    let selected_tab = binding(TabItem::Home);
    
    tab_view(selected_tab.clone(), vec![
        tab_item(TabItem::Home, "Home", "🏠", home_tab_content()),
        tab_item(TabItem::Search, "Search", "🔍", search_tab_content()),
        tab_item(TabItem::Favorites, "Favorites", "❤️", favorites_tab_content()),
        tab_item(TabItem::Profile, "Profile", "👤", profile_tab_content()),
    ))
}

#[derive(Clone, PartialEq)]
enum TabItem {
    Home,
    Search,
    Favorites,
    Profile,
}

fn home_tab_content() -> impl View {
    scroll(
        vstack((
            text("Welcome Home!").font_size(24.0),
            
            card([
                text("Recent Activity").font_weight(FontWeight::Bold),
                "You have 3 new messages",
                "2 items added to favorites",
            ))
            .spacing(10.0)
            .padding(15.0),
            
            card([
                text("Quick Actions").font_weight(FontWeight::Bold),
                hstack((
                    quick_action_button("📝", "Notes"),
                    quick_action_button("📷", "Camera"), 
                    quick_action_button("📍", "Maps"),
                    quick_action_button("🎵", "Music"),
                ))
                .spacing(15.0),
            ))
            .spacing(10.0)
            .padding(15.0),
        ))
        .spacing(20.0)
        .padding(20.0)
    )
}

fn search_tab_content() -> impl View {
    let search_query = binding(String::new());
    let search_results = binding(Vec::<SearchResult>::new());
    
    vstack((
        search_bar(search_query.clone(), {
            let search_results = search_results.clone();
            move |query| {
                // Simulate search
                let results = if query.is_empty() {
                    vec![]
                } else {
                    vec![
                        SearchResult { title: format!("Result for '{}'", query), description: "Description 1".to_string() },
                        SearchResult { title: format!("Another result for '{}'", query), description: "Description 2".to_string() },
                    ]
                };
                search_results.set(results);
            }
        }),
        
        scroll(
            vstack(
                search_results.get().map(|results| {
                    results.into_iter().map(|result| {
                        search_result_item(result)
                    })
                })
            )
            .spacing(10.0)
        ),
    ))
    .spacing(15.0)
    .padding(20.0)
}

fn favorites_tab_content() -> impl View {
    let favorites = binding(vec![
        "Favorite Item 1".to_string(),
        "Favorite Item 2".to_string(),
        "Favorite Item 3".to_string(),
    ));
    
    vstack((
        text("Your Favorites").font_size(24.0),
        
        scroll(
            vstack(
                favorites.get().map(|items| {
                    items.into_iter().map(|item| {
                        favorite_item(item)
                    })
                })
            )
            .spacing(10.0)
        ),
    ))
    .spacing(20.0)
    .padding(20.0)
}

fn profile_tab_content() -> impl View {
    vstack((
        // Profile header
        vstack((
            avatar("👤")
                .width(80.0)
                .height(80.0),
            text("John Doe").font_size(20.0),
            "john.doe@example.com"
                .color(Color::secondary()),
        ))
        .spacing(10.0)
        .alignment(.center),
        
        // Profile options
        vstack((
            profile_option("👤", "Edit Profile"),
            profile_option("🔔", "Notifications"),
            profile_option("🔒", "Privacy"),
            profile_option("❓", "Help"),
            profile_option("⚙️", "Settings"),
        ))
        .spacing(5.0),
    ))
    .spacing(30.0)
    .padding(20.0)
}

#[derive(Clone)]
struct SearchResult {
    title: String,
    description: String,
}

fn search_result_item(result: SearchResult) -> impl View {
    vstack((
        text(&result.title).font_weight(FontWeight::Medium),
        text(&result.description)
            .font_size(14.0)
            .color(Color::secondary()),
    ))
    .alignment(.leading)
    .padding(15.0)
    .background(Color::item_background())
    .corner_radius(8.0)
    .spacing(5.0)
}

fn quick_action_button(icon: &str, label: &str) -> impl View {
    vstack((
        text(icon).font_size(24.0),
        text(label).font_size(12.0),
    ))
    .alignment(.center)
    .padding(15.0)
    .background(Color::light_gray())
    .corner_radius(10.0)
    .spacing(5.0)
    .on_tap(move |_| {
        println!("Quick action: {}", label);
    })
}

fn favorite_item(item: String) -> impl View {
    hstack((
        "❤️",
        text(&item).flex(1),
        button("Remove")
            .style(ButtonStyle::Destructive)
            .action(move |_| {
                println!("Removing favorite: {}", item);
            }),
    ))
    .padding(15.0)
    .background(Color::item_background())
    .corner_radius(8.0)
    .spacing(10.0)
}

fn profile_option(icon: &str, label: &str) -> impl View {
    hstack((
        text(icon),
        text(label).flex(1),
        text("›").color(Color::secondary()),
    ))
    .padding(15.0)
    .background(Color::item_background())
    .corner_radius(8.0)
    .spacing(15.0)
    .on_tap(move |_| {
        println!("Profile option: {}", label);
    })
}
```

## Advanced Navigation Patterns

### Side Navigation (Drawer)

```rust,ignore
fn side_navigation_demo() -> impl View {
    let drawer_open = binding(false);
    let current_page = binding(DrawerPage::Dashboard);
    
    drawer_layout(
        drawer_open.clone(),
        side_drawer(current_page.clone(), drawer_open.clone()),
        main_content(current_page.clone(), drawer_open.clone())
    )
}

#[derive(Clone, PartialEq)]
enum DrawerPage {
    Dashboard,
    Analytics,
    Users,
    Settings,
}

fn side_drawer(
    current_page: Binding<DrawerPage>,
    drawer_open: Binding<bool>
) -> impl View {
    vstack((
        // Drawer header
        hstack((
            "🏢",
            text("My App").font_weight(FontWeight::Bold),
            spacer(),
            button("×")
                .action({
                    let drawer_open = drawer_open.clone();
                    move |_| drawer_open.set(false)
                }),
        ))
        .padding(20.0)
        .background(Color::primary()),
        
        // Navigation items
        vstack((
            drawer_item(DrawerPage::Dashboard, "📊", "Dashboard", current_page.clone(), drawer_open.clone()),
            drawer_item(DrawerPage::Analytics, "📈", "Analytics", current_page.clone(), drawer_open.clone()),
            drawer_item(DrawerPage::Users, "👥", "Users", current_page.clone(), drawer_open.clone()),
            drawer_item(DrawerPage::Settings, "⚙️", "Settings", current_page.clone(), drawer_open.clone()),
        ))
        .spacing(5.0)
        .padding(10.0),
        
        spacer(),
        
        // Footer
        "v1.0.0"
            .font_size(12.0)
            .color(Color::secondary())
            .padding(20.0),
    ))
    .width(280.0)
    .background(Color::surface())
}

fn drawer_item(
    page: DrawerPage,
    icon: &str,
    label: &str,
    current_page: Binding<DrawerPage>,
    drawer_open: Binding<bool>
) -> impl View {
    let is_selected = s!(current_page == page);
    let page_clone = page.clone();
    
    hstack((
        text(icon),
        text(label).flex(1),
    ))
    .padding(15.0)
    .background(s!(if is_selected { Color::primary().opacity(0.1) } else { Color::transparent() }))
    .corner_radius(8.0)
    .spacing(15.0)
    .on_tap({
        let current_page = current_page.clone();
        let drawer_open = drawer_open.clone();
        move |_| {
            current_page.set(page_clone.clone());
            drawer_open.set(false);
        }
    })
}

fn main_content(
    current_page: Binding<DrawerPage>,
    drawer_open: Binding<bool>
) -> impl View {
    vstack((
        // App bar
        hstack((
            button("☰")
                .action({
                    let drawer_open = drawer_open.clone();
                    move |_| drawer_open.set(true)
                }),
            text!("{:?}", current_page).flex(1),
            button("⚙️"),
        ))
        .padding(15.0)
        .background(Color::primary()),
        
        // Page content
        s!(match current_page {
            DrawerPage::Dashboard => Some(dashboard_content()),
            DrawerPage::Analytics => Some(analytics_content()),
            DrawerPage::Users => Some(users_content()),
            DrawerPage::Settings => Some(settings_content()),
        }),
    ))
}
```

### Breadcrumb Navigation

```rust,ignore
fn breadcrumb_demo() -> impl View {
    let breadcrumbs = binding(vec![
        BreadcrumbItem { label: "Home".to_string(), route: "/".to_string() },
        BreadcrumbItem { label: "Products".to_string(), route: "/products".to_string() },
        BreadcrumbItem { label: "Electronics".to_string(), route: "/products/electronics".to_string() },
        BreadcrumbItem { label: "Smartphones".to_string(), route: "/products/electronics/smartphones".to_string() },
    ));
    
    vstack((
        breadcrumb_navigation(breadcrumbs.clone()),
        
        // Page content
        vstack((
            text("Smartphones").font_size(24.0),
            "Browse our selection of smartphones",
            
            button("Go to Tablets")
                .action({
                    let breadcrumbs = breadcrumbs.clone();
                    move |_| {
                        breadcrumbs.set(vec![
                            BreadcrumbItem { label: "Home".to_string(), route: "/".to_string() },
                            BreadcrumbItem { label: "Products".to_string(), route: "/products".to_string() },
                            BreadcrumbItem { label: "Electronics".to_string(), route: "/products/electronics".to_string() },
                            BreadcrumbItem { label: "Tablets".to_string(), route: "/products/electronics/tablets".to_string() },
                        ));
                    }
                }),
        ))
        .spacing(15.0)
        .padding(20.0),
    ))
    .spacing(10.0)
}

#[derive(Clone)]
struct BreadcrumbItem {
    label: String,
    route: String,
}

fn breadcrumb_navigation(breadcrumbs: Binding<Vec<BreadcrumbItem>>) -> impl View {
    scroll_horizontal(
        hstack(
            breadcrumbs.get().map(|items| {
                items.into_iter().enumerate().map(|(index, item)| {
                    let is_last = index == items.len() - 1;
                    
                    hstack((
                        if is_last {
                            text(&item.label)
                                .color(Color::primary())
                                .font_weight(FontWeight::Medium)
                        } else {
                            text(&item.label)
                                .color(Color::link())
                                .underline(true)
                                .on_tap(move |_| {
                                    println!("Navigate to: {}", item.route);
                                })
                        },
                        
                        if !is_last {
                            Some(
                                text("›")
                                    .color(Color::secondary())
                                    .margin_horizontal(8.0)
                            )
                        } else {
                            None
                        },
                    ))
                })
            })
        )
    )
    .padding(15.0)
    .background(Color::light_gray())
}
```

## Route Management

### Router Implementation

```rust,ignore
fn router_demo() -> impl View {
    let router = Router::new();
    
    router.route("/", home_route())
        .route("/profile/:id", profile_route())
        .route("/settings", settings_route())
        .route("/about", about_route())
        .not_found(not_found_route())
}

fn home_route() -> impl View {
    let router = use_router();
    
    vstack((
        text("Home Page").font_size(24.0),
        
        button("Go to Profile")
            .action({
                let router = router.clone();
                move |_| {
                    router.navigate("/profile/123");
                }
            }),
            
        button("Go to Settings")
            .action({
                let router = router.clone();
                move |_| {
                    router.navigate("/settings");
                }
            }),
    ))
    .spacing(15.0)
    .padding(20.0)
}

fn profile_route() -> impl View {
    let router = use_router();
    let user_id = router.param("id").unwrap_or("unknown");
    
    vstack((
        text!("Profile Page for User {}", user_id).font_size(24.0),
        
        button("Back to Home")
            .action({
                let router = router.clone();
                move |_| {
                    router.navigate("/");
                }
            }),
            
        button("Edit Profile")
            .action({
                let router = router.clone();
                let user_id = user_id.clone();
                move |_| {
                    router.navigate(&format!("/profile/{}/edit", user_id));
                }
            }),
    ))
    .spacing(15.0)
    .padding(20.0)
}

fn settings_route() -> impl View {
    let router = use_router();
    
    vstack((
        text("Settings Page").font_size(24.0),
        
        settings_list(),
        
        button("Back to Home")
            .action({
                let router = router.clone();
                move |_| {
                    router.navigate("/");
                }
            }),
    ))
    .spacing(15.0)
    .padding(20.0)
}
```

### Deep Linking

```rust,ignore
fn deep_link_demo() -> impl View {
    let deep_link_handler = DeepLinkHandler::new();
    
    deep_link_handler
        .handle("myapp://profile/:id", handle_profile_deep_link)
        .handle("myapp://settings/:section", handle_settings_deep_link)
        .handle("myapp://share", handle_share_deep_link)
}

fn handle_profile_deep_link(params: HashMap<String, String>) -> impl View {
    let user_id = params.get("id").unwrap_or(&"unknown".to_string()).clone();
    
    navigation_stack(
        NavigationController::new(),
        profile_screen_with_id(user_id)
    )
}

fn handle_settings_deep_link(params: HashMap<String, String>) -> impl View {
    let section = params.get("section").unwrap_or(&"general".to_string()).clone();
    
    navigation_stack(
        NavigationController::new(),
        settings_screen_with_section(section)
    )
}
```

## Navigation State Management

### Navigation History

```rust,ignore
fn navigation_history_demo() -> impl View {
    let nav_history = NavigationHistory::new();
    let current_route = binding("/".to_string());
    
    vstack((
        // History display
        text("Navigation History").font_size(18.0),
        
        scroll(
            vstack(
                nav_history.history().get().map(|history| {
                    history.into_iter().enumerate().map(|(index, route)| {
                        history_item(route, index == history.len() - 1)
                    })
                })
            )
            .spacing(5.0)
        )
        .max_height(200.0),
        
        // Navigation controls
        hstack((
            button("Back")
                .disabled(s!(!nav_history.can_go_back()))
                .action({
                    let nav_history = nav_history.clone();
                    let current_route = current_route.clone();
                    move |_| {
                        if let Some(route) = nav_history.go_back() {
                            current_route.set(route);
                        }
                    }
                }),
                
            button("Forward")
                .disabled(s!(!nav_history.can_go_forward()))
                .action({
                    let nav_history = nav_history.clone();
                    let current_route = current_route.clone();
                    move |_| {
                        if let Some(route) = nav_history.go_forward() {
                            current_route.set(route);
                        }
                    }
                }),
        ))
        .spacing(10.0),
        
        // Quick navigation
        hstack((
            button("Home")
                .action({
                    let nav_history = nav_history.clone();
                    let current_route = current_route.clone();
                    move |_| {
                        nav_history.push("/");
                        current_route.set("/".to_string());
                    }
                }),
                
            button("Profile")
                .action({
                    let nav_history = nav_history.clone();
                    let current_route = current_route.clone();
                    move |_| {
                        nav_history.push("/profile");
                        current_route.set("/profile".to_string());
                    }
                }),
                
            button("Settings")
                .action({
                    let nav_history = nav_history.clone();
                    let current_route = current_route.clone();
                    move |_| {
                        nav_history.push("/settings");
                        current_route.set("/settings".to_string());
                    }
                }),
        ))
        .spacing(10.0),
        
        text!("Current route: {}", current_route),
    ))
    .spacing(20.0)
    .padding(20.0)
}

fn history_item(route: String, is_current: bool) -> impl View {
    hstack((
        "•",
        text(&route),
        if is_current {
            Some(text("← current").color(Color::primary()))
        } else {
            None
        },
    ))
    .spacing(5.0)
    .color(if is_current { Color::primary() } else { Color::text() })
}
```

## Testing Navigation

```rust,ignore
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_navigation_history() {
        let mut history = NavigationHistory::new();
        
        // Initial state
        assert!(!history.can_go_back());
        assert!(!history.can_go_forward());
        
        // Push routes
        history.push("/home");
        history.push("/profile");
        history.push("/settings");
        
        assert!(history.can_go_back());
        assert!(!history.can_go_forward());
        
        // Go back
        let route = history.go_back();
        assert_eq!(route, Some("/profile".to_string()));
        assert!(history.can_go_back());
        assert!(history.can_go_forward());
        
        // Go forward
        let route = history.go_forward();
        assert_eq!(route, Some("/settings".to_string()));
        assert!(!history.can_go_forward());
    }
    
    #[test]
    fn test_breadcrumb_generation() {
        let route = "/products/electronics/smartphones";
        let breadcrumbs = generate_breadcrumbs(route);
        
        assert_eq!(breadcrumbs.len(), 4);
        assert_eq!(breadcrumbs[0].label, "Home");
        assert_eq!(breadcrumbs[1].label, "Products");
        assert_eq!(breadcrumbs[2].label, "Electronics");
        assert_eq!(breadcrumbs[3].label, "Smartphones");
    }
}

fn generate_breadcrumbs(route: &str) -> Vec<BreadcrumbItem> {
    let parts: Vec<&str> = route.split('/').filter(|s| !s.is_empty()).collect();
    let mut breadcrumbs = vec![BreadcrumbItem {
        label: "Home".to_string(),
        route: "/".to_string(),
    }];
    
    let mut current_path = String::new();
    for part in parts {
        current_path.push('/');
        current_path.push_str(part);
        
        breadcrumbs.push(BreadcrumbItem {
            label: part.to_title_case(),
            route: current_path.clone(),
        });
    }
    
    breadcrumbs
}

trait ToTitleCase {
    fn to_title_case(&self) -> String;
}

impl ToTitleCase for &str {
    fn to_title_case(&self) -> String {
        self.chars()
            .enumerate()
            .map(|(i, c)| if i == 0 { c.to_uppercase().collect::<String>() } else { c.to_string() })
            .collect()
    }
}
```

## Summary

WaterUI's navigation system provides:

- **Navigation Stack**: Push/pop navigation with automatic back button handling
- **Tab Navigation**: Bottom tab bars with badge support
- **Side Navigation**: Drawer patterns for desktop and tablet layouts
- **Breadcrumbs**: Hierarchical navigation indicators
- **Routing**: URL-based routing with parameters and deep linking
- **History Management**: Back/forward navigation with state preservation

Key best practices:
- Use appropriate navigation patterns for your platform
- Provide clear navigation hierarchies
- Handle deep links and state restoration
- Test navigation flows thoroughly
- Consider accessibility for navigation elements
- Implement proper loading states during navigation

Next: [Styling and Theming](13-styling.md)
