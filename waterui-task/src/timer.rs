use core::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll},
    time::Duration,
};

use alloc::sync::Arc;

use crate::exec_after;

pub struct Timer {
    duration: Option<Duration>,
    finished: Arc<AtomicBool>,
}

impl Timer {
    pub fn after(duration: Duration) -> Self {
        Self {
            duration: Some(duration),
            finished: Arc::default(),
        }
    }
}

impl Future for Timer {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.finished.load(Ordering::Acquire) {
            return Poll::Ready(());
        }

        if let Some(duration) = self.duration.take() {
            let waker = cx.waker().clone();
            let finished = self.finished.clone();
            exec_after(duration, move || {
                finished.store(true, Ordering::Release);
                waker.wake();
            });
        }

        Poll::Pending
    }
}

pub async fn sleep(secs: u64) {
    Timer::after(Duration::from_secs(secs)).await;
}
