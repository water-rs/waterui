use crate::{component::AnyView, env::EnvironmentBuilder, Environment, View, ViewExt};

pub struct AppBuilder {
    content: AnyView,
    env: EnvironmentBuilder,
}

impl AppBuilder {
    pub fn new(content: impl View + 'static) -> Self {
        Self {
            content: content.anyview(),
            env: EnvironmentBuilder::new(),
        }
    }

    pub fn env<T: 'static>(mut self, value: T) -> Self {
        self.env.insert(value);
        self
    }
}

pub struct App {
    pub _content: AnyView,
    pub _env: Environment,
}

impl App {
    pub fn builder(view: impl View + 'static) -> AppBuilder {
        AppBuilder::new(view)
    }
}

impl AppBuilder {
    pub fn build(self) -> App {
        App {
            _content: self.content,
            _env: self.env.build(),
        }
    }
}
