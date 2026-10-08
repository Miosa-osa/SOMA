//! The exec classifier: which commands keep a shell and which run as their own argv.

use super::super::command::exec_command;
use super::{ENV, Plan, SHELL, plan};

/// A command that runs without a shell, with the program the guest execs first.
fn direct(command: &str) -> (&str, Vec<&str>) {
    match plan(command, true) {
        Plan::Direct { program, arguments } => (program, arguments),
        other => panic!("{command} must run without a shell, got {other:?}"),
    }
}

/// A command a shell must interpret, when the shell-free path is on.
fn shelled(command: &str) -> &str {
    match plan(command, true) {
        Plan::PlainShell(text) => text,
        other => panic!("{command} must keep the shell, got {other:?}"),
    }
}

#[test]
fn an_exec_no_longer_runs_under_a_login_shell() {
    // A word list runs as its own argv, with the lookup program resolving a bare name.
    let direct = exec_command("node -v", true, 30_000).expect("command");
    assert_eq!(direct.executable(), ENV);
    assert_eq!(
        direct.arguments(),
        &["node".to_owned(), "-v".to_owned()][..]
    );
    assert!(!direct.arguments().contains(&"-l".to_owned()));

    // A command a shell must interpret keeps one, but not the login shell.
    let shelled = exec_command("echo $HOME", true, 30_000).expect("command");
    assert_eq!(shelled.executable(), SHELL);
    assert_eq!(
        shelled.arguments(),
        &["-c".to_owned(), "echo $HOME".to_owned()][..]
    );
    assert!(!shelled.arguments().contains(&"-l".to_owned()));

    // The rollback flag restores the fast lane's argv exactly.
    let rolled_back = exec_command("node -v", false, 30_000).expect("command");
    assert_eq!(rolled_back.executable(), SHELL);
    assert_eq!(
        rolled_back.arguments(),
        &["-lc".to_owned(), "node -v".to_owned()][..]
    );
}

#[test]
fn a_word_list_runs_as_its_own_argv() {
    // The headline case: a bare name needs the PATH lookup a shell would do, and the lookup
    // program performs exactly that without adding a process.
    assert_eq!(direct("node -v"), (ENV, vec!["node", "-v"]));
    assert_eq!(direct("npm install"), (ENV, vec!["npm", "install"]));
    assert_eq!(direct("git status"), (ENV, vec!["git", "status"]));
    assert_eq!(
        direct("ls -1A /workspace"),
        (ENV, vec!["ls", "-1A", "/workspace"])
    );
    // An absolute program needs no lookup at all.
    assert_eq!(
        direct("/usr/local/bin/node --version"),
        ("/usr/local/bin/node", vec!["--version"])
    );
    assert_eq!(direct("/bin/echo hi"), ("/bin/echo", vec!["hi"]));
    // A relative path with a slash is exec'd as it is; a shell would do the same.
    assert_eq!(direct("./run.sh now"), (ENV, vec!["./run.sh", "now"]));
}

#[test]
fn quotes_globs_and_expansions_keep_the_shell() {
    // The single quotes are the shell's, and dropping them would change the argument.
    assert_eq!(shelled("ls -1A '/workspace'"), "ls -1A '/workspace'");
    assert_eq!(shelled(r#"echo "hello world""#), r#"echo "hello world""#);
    assert_eq!(shelled("echo $HOME"), "echo $HOME");
    assert_eq!(shelled("echo ${HOME}"), "echo ${HOME}");
    assert_eq!(shelled("ls ~"), "ls ~");
    assert_eq!(shelled("rm -rf /tmp/*"), "rm -rf /tmp/*");
    assert_eq!(shelled("ls ?.txt"), "ls ?.txt");
    assert_eq!(shelled("ls file[12]"), "ls file[12]");
    assert_eq!(shelled("ls | wc -l"), "ls | wc -l");
    assert_eq!(shelled("make && make test"), "make && make test");
    assert_eq!(shelled("cat a > b"), "cat a > b");
    assert_eq!(shelled("node -v # version"), "node -v # version");
    assert_eq!(shelled("printf '%s\\n' hi"), "printf '%s\\n' hi");
    assert_eq!(
        shelled("node --experimental-x=1 -e 1"),
        "node --experimental-x=1 -e 1"
    );
}

#[test]
fn an_assignment_prefix_keeps_the_shell() {
    // `FOO=bar cmd` is a shell assignment, not an argv; `=` is what excludes it.
    assert_eq!(shelled("FOO=bar node -v"), "FOO=bar node -v");
    assert_eq!(shelled("PATH=/usr/bin ls"), "PATH=/usr/bin ls");
}

#[test]
fn a_shell_builtin_keeps_the_shell() {
    // `cd` has no external program, so running it directly would be a different act.
    assert_eq!(shelled("cd /workspace"), "cd /workspace");
    assert_eq!(shelled("true"), "true");
    assert_eq!(shelled("export PATH"), "export PATH");
    assert_eq!(shelled("echo hello"), "echo hello");
    assert_eq!(shelled("test -f /etc/hosts"), "test -f /etc/hosts");
    assert_eq!(shelled(": nothing"), ": nothing");
    assert_eq!(shelled("wait"), "wait");
    // A leading dash is an option to whatever runs the command, never a program name.
    assert_eq!(shelled("-x"), "-x");
}

#[test]
fn an_empty_command_keeps_the_shell() {
    assert_eq!(shelled(""), "");
    assert_eq!(shelled("   "), "   ");
}

#[test]
fn the_rollback_flag_restores_the_login_shell() {
    for command in ["node -v", "/bin/echo hi", "echo $HOME", "cd /tmp"] {
        assert_eq!(
            plan(command, false),
            Plan::LoginShell(command),
            "{command} must be a login shell when the flag is off"
        );
        assert!(
            !matches!(plan(command, false), Plan::Direct { .. }),
            "{command} must never run directly when the flag is off"
        );
    }
    // With the flag on, a shell command is the plain shell and never the login shell.
    assert!(
        !matches!(plan("echo $HOME", true), Plan::LoginShell(_)),
        "the login shell only comes back with the flag off"
    );
}
