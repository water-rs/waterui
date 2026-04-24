//! GTK4 Menu component implementation.

use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{MenuButton, Popover, Widget};
use nami::Signal;
use waterui_controls::menu::{ResolvedMenu, ResolvedMenuItem};
use waterui_core::{Environment, Native};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::{store_watcher_guard, store_watcher_guards};

impl GtkComponent for Native<ResolvedMenu> {
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

pub(crate) fn rebuild_menu_popover(
    popover: &Popover,
    items: Vec<ResolvedMenuItem>,
    env: &Environment,
) {
    let list = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let on_activate = Rc::new({
        let popover = popover.clone();
        move || popover.popdown()
    });
    append_menu_items(&list, items, env, &on_activate);
    popover.set_child(Some(&list));
}

pub(crate) fn append_menu_items(
    list: &gtk4::Box,
    items: Vec<ResolvedMenuItem>,
    env: &Environment,
    on_activate: &Rc<dyn Fn()>,
) {
    for item in items {
        match item {
            ResolvedMenuItem::Command(command) => {
                let button = gtk4::Button::with_label(&command.label.content.get().to_plain());
                button.add_css_class("flat");
                button.set_sensitive(!command.disabled.get());

                let disabled_guard = command.disabled.watch({
                    let button = button.clone();
                    move |ctx: nami::watcher::Context<bool>| {
                        let disabled = ctx.into_value();
                        let button = button.clone();
                        glib::idle_add_local_once(move || {
                            button.set_sensitive(!disabled);
                        });
                    }
                });

                let action = command.action.clone();
                let env = env.clone();
                let on_activate = on_activate.clone();
                button.connect_clicked(move |_| {
                    let _ = action.call(&env);
                    on_activate();
                });

                store_watcher_guards(&button, vec![disabled_guard]);
                list.append(&button);
            }
            ResolvedMenuItem::Divider => {
                list.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
            }
            ResolvedMenuItem::Menu(menu) => {
                let button = MenuButton::new();
                let title = gtk4::Label::new(Some(&menu.label.content.get().to_plain()));
                title.set_xalign(0.0);
                button.set_child(Some(&title));
                button.add_css_class("flat");

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
                list.append(&button);
            }
        }
    }
}
