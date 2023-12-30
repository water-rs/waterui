use crate::{component::AnyView, env::EnvironmentBuilder, Environment, View, ViewExt};

pub struct AppBuilder {
    view: AnyView,
    environment: EnvironmentBuilder,
}

impl AppBuilder {
    pub fn new(view: impl View + 'static) -> Self {
        Self {
            view: view.anyview(),
            environment: EnvironmentBuilder::new(),
        }
    }

    pub fn env<T: Send + Sync + 'static>(mut self, value: T) -> Self {
        self.environment.insert(value);
        self
    }
}

pub struct App {
    pub(crate) view: AnyView,
    pub(crate) environment: Environment,
}

impl App {
    pub fn builder(view: impl View + 'static) -> AppBuilder {
        AppBuilder::new(view)
    }
}

impl AppBuilder {
    pub fn build(self) -> App {
        App {
            view: self.view,
            environment: self.environment.build(),
        }
    }
}
