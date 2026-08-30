//! The socket directory must be proved before any privileged change reaches it.

use super::*;

#[test]
fn a_planted_symlink_is_refused_before_any_privileged_change_reaches_it() {
    let root = tempfile::tempdir().expect("state dir");
    let victim = root.path().join("victim");
    std::fs::create_dir(&victim).expect("victim directory");
    let target = c_path(victim.as_os_str()).expect("victim path");
    // SAFETY: the path is a valid NUL-terminated string owned by this process.
    assert_eq!(unsafe { libc::chmod(target.as_ptr(), 0o777) }, 0);
    let planted = root.path().join("run");
    std::os::unix::fs::symlink(&victim, &planted).expect("plant the symlink");

    assert_eq!(
        ControlListener::bind(&planted.join("broker.sock"), authority())
            .expect_err("a symlinked socket directory"),
        Error::Unauthorized("socket directory type")
    );

    assert_eq!(
        facts(&target).expect("stat").expect("present").0 & 0o7777,
        0o777,
        "the broker mutated a directory it had not proved"
    );
}

#[test]
fn a_directory_that_already_exists_is_accepted_as_it_is_or_refused() {
    let root = tempfile::tempdir().expect("state dir");
    let shared = root.path().join("run");
    std::fs::create_dir(&shared).expect("shared directory");
    let target = c_path(shared.as_os_str()).expect("shared path");
    // SAFETY: the path is a valid NUL-terminated string owned by this process.
    assert_eq!(unsafe { libc::chmod(target.as_ptr(), 0o777) }, 0);

    assert_eq!(
        ControlListener::bind(&shared.join("broker.sock"), authority())
            .expect_err("a directory the broker does not already own"),
        Error::Unauthorized("socket directory mode")
    );

    assert_eq!(
        facts(&target).expect("stat").expect("present").0 & 0o7777,
        0o777,
        "the broker took over a pre-existing directory instead of refusing it"
    );
}
