use super::command::{CONTROL_TIMEOUT, command};

pub(super) fn container_name(instance: &str) -> String {
    format!("soma-{instance}")
}

pub(super) fn remove(name: &str) -> bool {
    command(&["rm", "--force", name], CONTROL_TIMEOUT)
        .status
        .is_some_and(|status| status.success())
}
