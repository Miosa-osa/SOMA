//! The launch of a sandbox a create answered before it was ready.
//!
//! A create that asked for an early answer sends its `201` as soon as its launch has taken a
//! facade lease, and the launch carries on behind that answer. Nothing else about the sandbox
//! changes: every later call waits for the launch to finish, and reads the same failure a
//! synchronous create would have answered with. That is only possible if the launch outlives
//! the request that started it, so the create hands it to this handle and the sandbox's own
//! record in [`crate::runner::sandboxes`] holds the handle rather than the request.
//!
//! The handle is a `watch` channel: one launch, any number of callers waiting on it. A caller
//! that arrives after the launch ended reads the terminal state that was already published, so
//! it never sees a sandbox as ready that is not, and never waits for a state that has passed.

use soma::ExecutionReceipt;
use tokio::sync::watch;

use crate::runner::backend::CallTiming;

/// What one launch has reported so far.
#[derive(Clone, Debug)]
pub enum LaunchState {
    /// The launch task has not taken a facade lease yet, so nothing has been launched and no
    /// machine exists. This is the state a create's own answer is formed from.
    Pending,
    /// A facade lease is held and the engine is bringing the machine up.
    Launched,
    /// The machine is up, with the launch evidence a synchronous create reports.
    Ready {
        receipt: Box<ExecutionReceipt>,
        call: CallTiming,
    },
    /// The launch ended in failure; the code is the engine's own stable one, which the create's
    /// log line carries.
    Failed(&'static str),
    /// The launch could not take a facade lease at all, which is the create's `429`.
    Busy,
    /// The launch task ended without reporting anything terminal, which is a lost launch rather
    /// than a sandbox: nothing here can say whether a machine was left behind.
    Lost,
}

impl LaunchState {
    /// Whether the launch is still in flight.
    const fn in_flight(&self) -> bool {
        matches!(self, Self::Pending | Self::Launched)
    }
}

/// The in-flight launch of one sandbox, as seen by every call that has to wait for it.
#[derive(Clone, Debug)]
pub struct Starting {
    state: watch::Receiver<LaunchState>,
}

impl Starting {
    /// Opens one launch channel: the sender for the task that runs the launch, the handle for the
    /// create that answers before it ends and for every call that waits on it.
    #[must_use]
    pub fn channel() -> (watch::Sender<LaunchState>, Self) {
        let (sender, state) = watch::channel(LaunchState::Pending);
        (sender, Self { state })
    }

    /// The state the launch has reached, without waiting.
    #[must_use]
    pub fn state(&self) -> LaunchState {
        self.state.borrow().clone()
    }

    /// Whether the machine is up.
    #[must_use]
    pub fn ready(&self) -> bool {
        matches!(*self.state.borrow(), LaunchState::Ready { .. })
    }

    /// Waits until the launch has taken its facade lease, or has ended before it could.
    ///
    /// This is what a create waits for before it answers an early `201`: once the launch holds a
    /// lease it cannot be refused for want of one, so the create either answers `201` for a
    /// launch that is running or answers the same failure a synchronous create would have.
    /// A launch that finished this quickly reports `Ready` here, and the create answers the
    /// truth about a sandbox that is already up.
    pub async fn begun(&mut self) -> LaunchState {
        self.until(|state| !matches!(state, LaunchState::Pending))
            .await
    }

    /// Waits until nothing about the launch will change again.
    ///
    /// The wait is bounded by the launch itself, which is the same bound a client of a
    /// synchronous create has today: there is no second timeout here to disagree with it.
    pub async fn settled(&mut self) -> LaunchState {
        self.until(|state| !state.in_flight()).await
    }

    /// Waits for the first state `reached` accepts.
    async fn until(&mut self, mut reached: impl FnMut(&LaunchState) -> bool) -> LaunchState {
        loop {
            let current = self.state.borrow_and_update().clone();
            if reached(&current) {
                return current;
            }
            // A sender that went away without sending a terminal state is a launch that was
            // lost, which the caller has to answer for rather than wait on.
            if self.state.changed().await.is_err() {
                return LaunchState::Lost;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{LaunchState, Starting};

    #[tokio::test]
    async fn a_lost_launch_is_reported_rather_than_waited_on() {
        let (sender, mut starting) = Starting::channel();
        drop(sender);
        assert!(matches!(starting.settled().await, LaunchState::Lost));
    }

    #[tokio::test]
    async fn a_handle_taken_later_reads_the_state_that_was_already_published() {
        let (sender, create) = Starting::channel();
        sender
            .send(LaunchState::Failed("launch_failed"))
            .expect("sent");
        // The record hands every later call its own handle over the same channel.
        let mut late = create.clone();
        assert!(matches!(
            late.settled().await,
            LaunchState::Failed("launch_failed")
        ));
    }

    #[tokio::test]
    async fn begun_answers_when_the_lease_is_taken_and_settled_when_the_launch_ends() {
        let (sender, mut starting) = Starting::channel();
        let launch = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            sender.send(LaunchState::Launched).expect("sent");
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender.send(LaunchState::Busy).expect("sent");
        });
        let begun = starting.begun().await;
        assert!(matches!(begun, LaunchState::Launched), "{begun:?}");
        assert!(matches!(starting.settled().await, LaunchState::Busy));
        launch.await.expect("the fake launch ran");
    }

    #[tokio::test]
    async fn a_launch_that_ended_before_the_create_looked_reports_that_it_ended() {
        let (sender, mut create) = Starting::channel();
        sender.send(LaunchState::Busy).expect("sent");
        // `begun` must not report a launch as running when it is already over.
        assert!(matches!(create.begun().await, LaunchState::Busy));
        assert!(!create.state().in_flight());
    }
}
