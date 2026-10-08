//! The exec shell choice: a plain shell by default, the login shell behind one flag.

use super::{Plan, plan};

#[test]
fn an_exec_runs_under_a_plain_shell_by_default() {
    for command in ["node -v", "echo $HOME", "/bin/echo hi", "cd /tmp"] {
        assert_eq!(
            plan(command, true),
            Plan::PlainShell(command),
            "{command} must not be a login shell"
        );
    }
}

#[test]
fn the_rollback_flag_restores_the_login_shell() {
    for command in ["node -v", "/bin/echo hi", "echo $HOME", "cd /tmp"] {
        assert_eq!(
            plan(command, false),
            Plan::LoginShell(command),
            "{command} must be a login shell when the flag is off"
        );
    }
}
