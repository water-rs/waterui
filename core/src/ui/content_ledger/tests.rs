use alloc::rc::Rc;
use alloc::string::ToString;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::future::Future;
use core::pin::{Pin, pin};
use core::sync::atomic::{AtomicUsize, Ordering};
use core::task::{Context, Poll, Waker};
use core::time::Duration;
use std::task::Wake;

use super::{ContentError, ContentLedger, OwedContent, PendingContent};
use crate::{Environment, Signal};

/// A stand-in for a component that owes content after mount.
struct FakeLoader;

/// A generic owner, to check that the recorded name drops type arguments.
struct Wrapper<T>(core::marker::PhantomData<T>);

/// Counts the wakes it receives, so a test sees exactly when a future asked
/// to be polled again.
#[derive(Default)]
struct CountingWaker(AtomicUsize);

impl Wake for CountingWaker {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn waker() -> (Arc<CountingWaker>, Waker) {
    let counter = Arc::new(CountingWaker::default());
    (Arc::clone(&counter), Waker::from(counter))
}

fn ledger_env() -> (ContentLedger, Environment) {
    let ledger = ContentLedger::new();
    let mut env = Environment::new();
    env.insert(ledger.clone());
    (ledger, env)
}

fn owe(env: &Environment, detail: &'static str) -> OwedContent {
    ContentLedger::owe::<FakeLoader>(env, detail).expect("the environment carries a ledger")
}

#[test]
fn owe_without_a_ledger_records_nothing() {
    assert!(ContentLedger::owe::<FakeLoader>(&Environment::new(), "file:///a.png").is_none());
}

#[test]
fn fulfil_resolves_the_entry() {
    let (ledger, env) = ledger_env();
    let token = owe(&env, "file:///a.png");
    assert_eq!(ledger.pending().snapshot(), 1);
    token.fulfil();
    assert_eq!(ledger.pending().snapshot(), 0);
    assert_eq!(ledger.outstanding(), Vec::new());
    let (_, waker) = waker();
    let poll = pin!(ledger.settled()).poll(&mut Context::from_waker(&waker));
    assert!(matches!(poll, Poll::Ready(Ok(()))));
}

#[test]
fn fail_reports_the_view_and_its_cause() {
    let (ledger, env) = ledger_env();
    owe(&env, "file:///missing.png").fail(std::io::Error::other("no such file"));
    assert_eq!(
        ledger.pending().snapshot(),
        0,
        "a failed entry is no longer owed"
    );
    let (_, waker) = waker();
    let Poll::Ready(Err(failure)) = pin!(ledger.settled()).poll(&mut Context::from_waker(&waker))
    else {
        panic!("a failed load settles the ledger with the failure");
    };
    assert_eq!(failure.view(), "FakeLoader");
    assert_eq!(failure.detail(), "file:///missing.png");
    assert_eq!(
        failure.to_string(),
        "FakeLoader (file:///missing.png) failed to load its content"
    );
    assert_eq!(
        core::error::Error::source(&failure).map(ToString::to_string),
        Some("no such file".into())
    );
    let poll = pin!(ledger.settled()).poll(&mut Context::from_waker(&waker));
    assert!(
        matches!(poll, Poll::Ready(Ok(()))),
        "a failure is reported once"
    );
}

#[test]
fn dropping_an_unresolved_entry_releases_it() {
    let (ledger, env) = ledger_env();
    let token = owe(&env, "file:///a.png");
    drop(token);
    assert_eq!(ledger.pending().snapshot(), 0);
    let (_, waker) = waker();
    let poll = pin!(ledger.settled()).poll(&mut Context::from_waker(&waker));
    assert!(
        matches!(poll, Poll::Ready(Ok(()))),
        "an unmounted view's content is released, not failed"
    );
}

#[test]
fn pending_is_a_signal_over_the_owed_count() {
    let (ledger, env) = ledger_env();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let _guard = ledger.pending().watch({
        let seen = Rc::clone(&seen);
        move |context| seen.borrow_mut().push(context.into_value())
    });
    let first = owe(&env, "a");
    let second = owe(&env, "b");
    first.fulfil();
    drop(second);
    assert_eq!(*seen.borrow(), vec![1, 2, 1, 0]);
}

#[test]
fn settled_wakes_once_the_last_entry_resolves() {
    let (ledger, env) = ledger_env();
    let first = owe(&env, "a");
    let second = owe(&env, "b");
    let (counter, waker) = waker();
    let mut cx = Context::from_waker(&waker);
    let mut settled = ledger.settled();
    assert!(Pin::new(&mut settled).poll(&mut cx).is_pending());
    first.fulfil();
    assert_eq!(
        counter.0.load(Ordering::SeqCst),
        0,
        "content is still owed, so the waiter is not woken"
    );
    second.fulfil();
    assert_eq!(counter.0.load(Ordering::SeqCst), 1);
    assert!(matches!(
        Pin::new(&mut settled).poll(&mut cx),
        Poll::Ready(Ok(()))
    ));
}

#[test]
fn outstanding_names_each_view_in_registration_order() {
    let (ledger, env) = ledger_env();
    let _photo = owe(&env, "file:///a.png");
    let _wrapped = ContentLedger::owe::<Wrapper<u8>>(&env, "resource").expect("ledger installed");
    assert_eq!(
        ledger.outstanding(),
        vec![
            PendingContent::new("FakeLoader", "file:///a.png".into()),
            PendingContent::new("Wrapper", "resource".into()),
        ]
    );
}

/// A host timer the test fires by hand.
struct Expiry(Rc<Cell<bool>>);

impl Future for Expiry {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if self.0.get() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

#[test]
fn wait_awaits_content_a_drain_mounts_late() {
    let (ledger, env) = ledger_env();
    let first = owe(&env, "a");
    let late: Rc<RefCell<Option<OwedContent>>> = Rc::default();
    let drains = Rc::new(Cell::new(0_usize));
    let expired = Rc::new(Cell::new(false));
    let mut wait = pin!(
        ledger.wait(Duration::from_secs(10), Expiry(Rc::clone(&expired)), || {
            drains.set(drains.get() + 1);
            // The first drain mounts a view that owes content of its own.
            if drains.get() == 1 {
                *late.borrow_mut() = Some(owe(&env, "b"));
            }
            core::future::ready(())
        },)
    );
    let (_, waker) = waker();
    let mut cx = Context::from_waker(&waker);
    assert!(wait.as_mut().poll(&mut cx).is_pending());
    first.fulfil();
    assert!(
        wait.as_mut().poll(&mut cx).is_pending(),
        "content mounted by the drain holds the capture"
    );
    assert_eq!(drains.get(), 1);
    late.borrow_mut()
        .take()
        .expect("the drain mounted late content")
        .fulfil();
    assert!(matches!(wait.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
    assert_eq!(
        drains.get(),
        2,
        "a full pass that changes nothing ends the wait"
    );
}

#[test]
fn wait_reports_the_views_still_owing_at_the_deadline() {
    let (ledger, env) = ledger_env();
    let _token = owe(&env, "file:///a.png");
    let expired = Rc::new(Cell::new(false));
    let mut wait = pin!(
        ledger.wait(Duration::from_secs(10), Expiry(Rc::clone(&expired)), || {
            core::future::ready(())
        },)
    );
    let (_, waker) = waker();
    let mut cx = Context::from_waker(&waker);
    assert!(wait.as_mut().poll(&mut cx).is_pending());
    expired.set(true);
    let Poll::Ready(Err(error)) = wait.as_mut().poll(&mut cx) else {
        panic!("the deadline ends the wait with an error");
    };
    assert!(matches!(&error, ContentError::Pending { outstanding, .. } if outstanding.len() == 1));
    assert_eq!(
        error.to_string(),
        "FakeLoader (file:///a.png) still owes content after 10s"
    );
}

#[test]
fn wait_fails_with_a_failed_load() {
    let (ledger, env) = ledger_env();
    let token = owe(&env, "file:///missing.png");
    let mut wait = pin!(ledger.wait(
        Duration::from_secs(10),
        Expiry(Rc::new(Cell::new(false))),
        || core::future::ready(()),
    ));
    let (_, waker) = waker();
    let mut cx = Context::from_waker(&waker);
    assert!(wait.as_mut().poll(&mut cx).is_pending());
    token.fail(std::io::Error::other("no such file"));
    assert!(matches!(
        wait.as_mut().poll(&mut cx),
        Poll::Ready(Err(ContentError::Failed(failure))) if failure.view() == "FakeLoader"
    ));
}
