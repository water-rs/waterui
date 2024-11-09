use core::time::Duration;
use dispatch::{Queue, QueuePriority};

use crate::Priority;

impl From<Priority> for QueuePriority {
    fn from(val: Priority) -> Self {
        match val {
            Priority::Default => QueuePriority::Default,
            Priority::Background => QueuePriority::Background,
        }
    }
}

pub fn exec_main(f: impl FnOnce() + Send + 'static) {
    let main = Queue::main();
    main.exec_async(f);
}

pub fn exec(f: impl FnOnce() + Send + 'static, priority: Priority) {
    let queue = Queue::global(priority.into());
    queue.exec_async(f);
}

pub fn exec_after(delay: Duration, f: impl FnOnce() + Send + 'static) {
    let queue = Queue::global(dispatch::QueuePriority::Default);
    queue.exec_after(delay, f);
}
