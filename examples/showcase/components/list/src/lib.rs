//! List Example - Demonstrates WaterUI's List component
//!
//! This example showcases:
//! - Basic List usage with static items
//! - List::for_each for dynamic collections
//! - ListItem configuration

use waterui::Identifiable;
use waterui::app::App;
use waterui::component::list::{List, ListItem};
use waterui::prelude::theme_color::{Foreground, MutedForeground};
use waterui::prelude::*;
use waterui::preview;

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

#[preview]
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

    List::for_each(contacts, |contact| {
        ListItem::new(
            vstack((
                text(contact.name).sub_headline().foreground(Foreground),
                text(contact.role).caption().foreground(MutedForeground),
            ))
            .alignment(HorizontalAlignment::Leading)
            .padding_with(EdgeInsets::symmetric(12.0, 16.0)),
        )
    })
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
