//! `spawn_local` work must be able to make progress in an offscreen test.

use std::cell::Cell;
use std::rc::Rc;

#[test]
fn parked_local_work_runs_when_drained() {
    waterui_testing::install_test_executor();

    let ran = Rc::new(Cell::new(false));
    let ran_in_task = Rc::clone(&ran);
    executor_core::spawn_local(async move {
        ran_in_task.set(true);
    })
    .detach();

    assert!(
        !ran.get(),
        "work must stay parked until a safe drain point, so it never re-enters the spawning call"
    );

    let drained = waterui_testing::drain_parked_local_work();

    assert_eq!(drained, 1);
    assert!(ran.get(), "draining must run the parked work");
}

#[test]
fn draining_runs_only_what_was_already_parked() {
    waterui_testing::install_test_executor();

    let rounds = Rc::new(Cell::new(0_usize));
    let rounds_in_task = Rc::clone(&rounds);
    executor_core::spawn_local(async move {
        rounds_in_task.set(rounds_in_task.get() + 1);
        // Spawning from inside a drained task must not extend this drain.
        let inner = Rc::clone(&rounds_in_task);
        executor_core::spawn_local(async move {
            inner.set(inner.get() + 1);
        })
        .detach();
    })
    .detach();

    let _ = waterui_testing::drain_parked_local_work();
    assert_eq!(rounds.get(), 1, "a self-spawning task must not spin the drain");

    let _ = waterui_testing::drain_parked_local_work();
    assert_eq!(rounds.get(), 2, "the next drain picks up the newly parked work");
}
