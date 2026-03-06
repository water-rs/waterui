//! GTK4 Menu component implementation.

use gtk4::prelude::*;
use gtk4::{MenuButton, Popover, Widget};
use nami::Signal;
use waterui_controls::menu::{Menu, MenuItem};
use waterui_core::{Environment, Native};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guard;

impl GtkComponent for Native<Menu> {
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let menu = self.into_inner();

        let button = MenuButton::new();
        let label = renderer.render_any(menu.label, env);
        button.set_child(Some(&label));

        let popover = Popover::new();
        button.set_popover(Some(&popover));

        rebuild_menu_popover(&popover, menu.items.get(), env);

        let guard = menu.items.watch({
            let popover = popover.clone();
            let env = env.clone();
            move |ctx| {
                let items = ctx.into_value();
                let popover = popover.clone();
                let env = env.clone();
                glib::idle_add_local_once(move || {
                    rebuild_menu_popover(&popover, items, &env);
                });
            }
        });

        store_watcher_guard(&button, Box::new(guard));
        button.upcast()
    }
}

fn rebuild_menu_popover(popover: &Popover, items: Vec<MenuItem>, env: &Environment) {
    let list = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    for item in items {
        let button = gtk4::Button::with_label(&menu_item_label(&item));
        button.add_css_class("flat");
        let action = item.action.clone();
        let env = env.clone();
        let popover_for_click = popover.clone();
        button.connect_clicked(move |_| {
            action.call(&env);
            popover_for_click.popdown();
        });
        list.append(&button);
    }
    popover.set_child(Some(&list));
}

fn menu_item_label(item: &MenuItem) -> String {
    item.label.content().get().to_plain().as_str().to_owned()
}
