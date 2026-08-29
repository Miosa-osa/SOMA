use std::time::{Duration, Instant};

use crate::GuestCommand;

const HANDSHAKE_BUDGET: Duration = Duration::from_secs(10);
const REPAIR_BUDGET: Duration = Duration::from_secs(5);
const PROBE_DELIVERY_GRACE: Duration = Duration::from_secs(1);
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(5);
const EXECUTE_DELIVERY_GRACE: Duration = Duration::from_secs(1);

pub(super) fn handshake() -> Instant {
    after(HANDSHAKE_BUDGET)
}

pub(super) fn repair() -> Instant {
    after(REPAIR_BUDGET)
}

pub(super) fn probe() -> Instant {
    let command = GuestCommand::readiness_probe();
    after(command_budget_with_grace(&command, PROBE_DELIVERY_GRACE))
}

pub(super) fn execute(command: &GuestCommand) -> Instant {
    after(command_budget(command))
}

pub(super) fn shutdown() -> Instant {
    after(SHUTDOWN_BUDGET)
}

fn command_budget(command: &GuestCommand) -> Duration {
    command_budget_with_grace(command, EXECUTE_DELIVERY_GRACE)
}

fn command_budget_with_grace(command: &GuestCommand, grace: Duration) -> Duration {
    Duration::from_millis(u64::from(command.timeout_millis()))
        .checked_add(grace)
        .unwrap_or(Duration::MAX)
}

fn after(budget: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(budget).unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_budgets_are_explicit_liveness_ceilings() {
        assert_eq!(HANDSHAKE_BUDGET, Duration::from_secs(10));
        assert_eq!(REPAIR_BUDGET, Duration::from_secs(5));
        assert_eq!(SHUTDOWN_BUDGET, Duration::from_secs(5));
        assert_eq!(PROBE_DELIVERY_GRACE, Duration::from_secs(1));
        assert_eq!(EXECUTE_DELIVERY_GRACE, Duration::from_secs(1));
        assert_eq!(
            command_budget(&GuestCommand::readiness_probe()),
            Duration::from_secs(2)
        );
    }

    #[test]
    fn execute_budget_is_command_timeout_plus_delivery_grace() {
        let command =
            GuestCommand::new(b"/bin/true".to_vec(), vec![], 275, 1).expect("bounded command");

        assert_eq!(command_budget(&command), Duration::from_millis(1_275));
    }
}
