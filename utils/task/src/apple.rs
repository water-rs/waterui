use core::time::Duration;
use dispatch::{Queue, QueuePriority};

use crate::{PlatformExecutor, Priority};

impl From<Priority> for QueuePriority {
    fn from(val: Priority) -> Self {
        match val {
            Priority::Default => Self::Default,
            Priority::Background => Self::Background,
        }
    }
}
pub struct ApplePlatformExecutor;
impl PlatformExecutor for ApplePlatformExecutor {
    fn exec_main(f: impl FnOnce() + Send + 'static) {
        let main = Queue::main();
        main.exec_async(f);
    }

    fn exec(f: impl FnOnce() + Send + 'static, priority: Priority) {
        let queue = Queue::global(priority.into());
        queue.exec_async(f);
    }

    fn exec_after(delay: Duration, f: impl FnOnce() + Send + 'static) {
        let queue = Queue::global(dispatch::QueuePriority::Default);
        queue.exec_after(delay, f);
    }
}
