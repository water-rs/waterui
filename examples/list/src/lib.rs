//! List Example - Demonstrates WaterUI's List component
//!
//! This example showcases:
//! - Basic List usage with static items
//! - List::for_each for dynamic collections
//! - ListItem configuration

use waterui::app::App;
use waterui::component::list::{List, ListItem};
use waterui::prelude::*;
use waterui::{AnyView, Identifiable};

#[derive(Clone)]
struct Contact {
    id: u64,
    name: &'static str,
    role: &'static str,
}

impl Identifiable for Contact {
    type Id = u64;
    fn id(&self) -> Self::Id {
        self.id
    }
}

fn main() -> impl View {
    let contacts = vec![
        Contact {
            id: 1,
            name: "Alice Chen",
            role: "Software Engineer",
        },
        Contact {
            id: 2,
            name: "Bob Smith",
            role: "Product Manager",
        },
        Contact {
            id: 3,
            name: "Carol Williams",
            role: "Designer",
        },
        Contact {
            id: 4,
            name: "David Kim",
            role: "DevOps Engineer",
        },
        Contact {
            id: 5,
            name: "Eva Martinez",
            role: "Data Scientist",
        },
        Contact {
            id: 6,
            name: "Frank Johnson",
            role: "QA Lead",
        },
        Contact {
            id: 7,
            name: "Grace Lee",
            role: "Tech Lead",
        },
        Contact {
            id: 8,
            name: "Henry Brown",
            role: "Backend Developer",
        },
    ];

    List::for_each(contacts, |contact| ListItem {
        content: AnyView::new(
            vstack((
                text(contact.name).size(17.0).bold(),
                text(contact.role)
                    .size(14.0)
                    .foreground(Color::srgb(128, 128, 128)),
            ))
            .padding_with(EdgeInsets::symmetric(12.0, 16.0)),
        ),
        on_delete: None,
    })
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
