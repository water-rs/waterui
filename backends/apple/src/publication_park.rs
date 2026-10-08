//! The parked wait on a GPU context publication, shared by the
//! `MetalPresenter` leaves (`gpu_surface`, `filtered`).
//!
//! A leaf whose frame finds its context generation lost parks: the
//! display link pauses once on the transition into the wait — deliveries
//! still arriving while parked drop unpresented — and exactly one
//! `context_after` task stays outstanding, a re-park replacing
//! (cancelling) the previous one. The publication's wake clears the hold
//! and runs the leaf's own replay, which re-arms the link. A terminal
//! frame failure takes the same wait through [`PublicationPark::fail`],
//! which records the typed carrier the leaf's capture contract answers
//! until the rebind.

use alloc::rc::Rc;
use alloc::sync::Arc;
use core::cell::RefCell;

use waterui_graphics::gpu::GpuRuntime;

/// Which wait holds the leaf — `Parked` is the transient device-loss
/// wait; `Failed` is the terminal-until-rebind typed failure.
#[derive(Clone)]
enum HoldKind {
    /// The frame's context generation reported device loss.
    Parked,
    /// A typed frame failure settled — the failed generation is the key
    /// that makes a second settle for the same failure a no-op, and the
    /// carrier is the terminal outcome a capture's completion forwards.
    Failed {
        /// The generation the failure settled under.
        generation: u64,
        /// The typed failure the capture's `Failed` outcome carries.
        error: Arc<dyn std::error::Error + Send + Sync>,
    },
}

/// A parked wait on a context publication: `watch` is the parked task —
/// replacing or dropping it cancels the wait — and its wake clears the
/// hold, then replays the owed work through the leaf's resume.
struct Hold {
    /// Which wait this is.
    kind: HoldKind,
    /// The parked `context_after` subscription — dropping the task
    /// cancels the wait; nothing else needs the handle.
    #[expect(
        dead_code,
        reason = "the handle is kept for its Drop, which cancels the parked task"
    )]
    watch: executor_core::AnyLocalExecutorTask<()>,
}

/// A leaf's parked wait on the next GPU context publication — the
/// device-loss park and the settled failure share one hold: while it
/// stands, the leaf's link stays paused and its delivered frames drop,
/// and the publication's wake clears it exactly once.
pub struct PublicationPark {
    /// The outstanding wait — `Some` while the leaf is parked or failed.
    /// Replacing the task cancels the previous wait.
    hold: RefCell<Option<Hold>>,
}

impl PublicationPark {
    /// An empty park — no wait outstanding.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            hold: RefCell::new(None),
        }
    }

    /// Whether a wait — parked or failed — is outstanding. The leaf's
    /// demand gate holds the link paused on it and its frame path drops
    /// every delivery.
    pub fn is_held(&self) -> bool {
        self.hold.borrow().is_some()
    }

    /// Whether the outstanding wait is the terminal-failure kind —
    /// readiness participation and the capture contract consult it.
    pub fn is_failed(&self) -> bool {
        matches!(
            &*self.hold.borrow(),
            Some(Hold {
                kind: HoldKind::Failed { .. },
                ..
            })
        )
    }

    /// The generation the outstanding `Failed` hold settled under —
    /// [`fail`](Self::fail)'s once-per-generation dedup key.
    fn failed_generation(&self) -> Option<u64> {
        match &*self.hold.borrow() {
            Some(Hold {
                kind: HoldKind::Failed { generation, .. },
                ..
            }) => Some(*generation),
            _ => None,
        }
    }

    /// The capture outcome a held leaf answers: the settled failure's
    /// typed `Failed` carrier — terminal on this context — or `Deferred`
    /// for a transient park, whose replay the publication wake drives.
    pub fn capture_error(&self) -> cocoa_ui::capture::CaptureError {
        match &*self.hold.borrow() {
            Some(Hold {
                kind: HoldKind::Failed { error, .. },
                ..
            }) => cocoa_ui::capture::CaptureError::Failed(error.clone()),
            _ => cocoa_ui::capture::CaptureError::Deferred,
        }
    }

    /// Parks the leaf until the runtime publishes a live context newer
    /// than `generation`: `pause` runs once on the transition into the
    /// wait — a hold already standing is only re-armed, and a `Failed`
    /// kind survives the re-park — and exactly one `context_after` task
    /// stays outstanding, its wake clearing the hold and running
    /// `resume`.
    pub fn park(
        self: &Rc<Self>,
        runtime: &GpuRuntime,
        generation: u64,
        pause: impl FnOnce(),
        resume: impl FnOnce() + 'static,
    ) {
        if self.hold.borrow().is_none() {
            // Entering the wait pauses the link — a stale delivery
            // still arriving drops on `is_held`.
            pause();
        }
        let kind = match &*self.hold.borrow() {
            Some(Hold {
                kind: failed @ HoldKind::Failed { .. },
                ..
            }) => failed.clone(),
            _ => HoldKind::Parked,
        };
        self.arm(runtime, generation, kind, resume);
    }

    /// Settles `error` as the leaf's terminal failure on `generation`:
    /// the `Failed` hold records the typed carrier, one settle per
    /// generation (`true`; a same-generation repeat is a no-op answering
    /// `false`). `pause` runs unless a failure already held the link,
    /// and the publication wait re-arms on `generation`.
    pub fn fail(
        self: &Rc<Self>,
        runtime: &GpuRuntime,
        generation: u64,
        error: Arc<dyn std::error::Error + Send + Sync>,
        pause: impl FnOnce(),
        resume: impl FnOnce() + 'static,
    ) -> bool {
        if self.failed_generation() == Some(generation) {
            // One failure record per generation — a second settle for
            // the same failure lands nothing further.
            return false;
        }
        if !self.is_failed() {
            // A failure supersedes a park — and a live link is paused
            // either way.
            pause();
        }
        self.arm(
            runtime,
            generation,
            HoldKind::Failed { generation, error },
            resume,
        );
        true
    }

    /// Arms the `context_after` wait on `generation` under `kind` —
    /// replacing an outstanding wait drops (cancels) its task. The wake
    /// takes the hold: a wake that finds none landed stale — superseded
    /// or cleared — and replays nothing; otherwise `resume` replays the
    /// owed work.
    fn arm(
        self: &Rc<Self>,
        runtime: &GpuRuntime,
        generation: u64,
        kind: HoldKind,
        resume: impl FnOnce() + 'static,
    ) {
        let weak = Rc::downgrade(self);
        let runtime = runtime.clone();
        *self.hold.borrow_mut() = Some(Hold {
            kind,
            watch: executor_core::spawn_local(async move {
                let _published = runtime.context_after(generation).await;
                let Some(park) = weak.upgrade() else {
                    return;
                };
                if park.hold.borrow_mut().take().is_none() {
                    // The wait was replaced or the hold already cleared —
                    // this wake landed late and replays nothing.
                    return;
                }
                resume();
            }),
        });
    }

    /// Drops the outstanding wait without running its resume —
    /// teardown: a leaf that is gone never replays.
    pub fn clear(&self) {
        self.hold.borrow_mut().take();
    }
}
