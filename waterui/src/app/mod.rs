mod bridge;
pub use bridge::Bridge;
use waterui_core::Environment;

pub struct App {
    pub _env: Environment,
    pub _bridge: Bridge,
}

impl App {
    pub fn new(env: Environment) -> Self {
        let bridge = Bridge::new(&env);
        Self {
            _env: env,
            _bridge: bridge,
        }
    }

    pub fn env(&self) -> Environment {
        self._env.clone()
    }

    pub fn task(&self, f: impl FnOnce() + Send + Sync + 'static) {
        self._bridge.send_blocking(f).unwrap();
    }

    pub fn lanuch(&self) {
        self._env.executor().run();
    }
}
