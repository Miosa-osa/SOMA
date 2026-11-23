use super::*;

#[test]
fn resolver_and_hosts_files_are_rendered_from_launch_material() {
    assert_eq!(
        resolver_file([10, 0, 0, 1]),
        "nameserver 10.0.0.1\noptions timeout:2 attempts:2\n"
    );
    assert_eq!(
        hosts_file("soma-abc", [10, 0, 0, 2]),
        "127.0.0.1 localhost\n10.0.0.2 soma-abc\n"
    );
}

#[test]
fn repair_refuses_an_unverified_target_before_it_opens_a_control_socket() {
    assert_eq!(target::require(target::COMPILED), Ok(()));
    assert_eq!(
        target::require(target::TargetAbi {
            architecture: "aarch64",
            ..target::VERIFIED
        }),
        Err(NetworkError {
            step: NetworkStep::UnsupportedTarget,
            errno: libc::ENOTSUP,
        })
    );
}

#[test]
fn reading_flags_of_a_missing_interface_fails_with_a_kernel_errno() {
    let socket = control_socket().expect("datagram socket");
    let error = get_flags(&socket, "soma-none0", NetworkStep::LinkDown).expect_err("missing");
    assert_eq!(error.step, NetworkStep::LinkDown);
    assert_eq!(error.errno, libc::ENODEV);
}
