//! The ownership seam, proved against a real Host daemon on a real socket.
//!
//! The daemon here is served by the in-process development launcher, so these tests prove
//! exactly what this change claims and nothing more: the Instance identity is owned by the
//! Host and is therefore addressable and terminable from a client that did not create it. The
//! guest session is not part of that claim, because it is still resident in the process that
//! launched the machine.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use soma::{InstanceId, OperationId};
use soma_hostd::{
    Admission, CleanRepair, CpuClass, CpuInventory, DirectoryVolumes, ExhaustedBehavior,
    GenerationId, HostProfile, Limits, MachineShape, MeasuredOverhead, MemoryClass,
    MemoryInventory, MemoryShape, NetworkInventory, OperatorLimits, OvercommitPolicy,
    OverlayIdentity, Pool, PoolAdmission, PoolKey, ProcessInventory, Runtime, SingleNode,
    StorageInventory, ValidShape, WorkloadClass,
    client::ClientError,
    daemon,
    testing::{InProcessBroker, InProcessLauncher, ProcessTable},
};
use soma_netd::ProfileDigest;
use soma_storage::{ClassName, TemplateDigest};

use super::Ownership;

#[path = "runtime/test_host.rs"]
mod host;
#[path = "runtime/refusal_tests.rs"]
mod refusal;
#[path = "runtime/restart_tests.rs"]
mod restart;

use host::Host;

/// How long a test waits for the daemon to bind its socket.
const PATIENCE: Duration = Duration::from_secs(5);

fn instance(tag: char) -> InstanceId {
    InstanceId::new(std::iter::repeat_n(tag, 32).collect::<String>()).expect("instance")
}

fn operation(tag: char) -> OperationId {
    OperationId::new(std::iter::repeat_n(tag, 32).collect::<String>()).expect("operation")
}

#[test]
fn no_configured_locator_keeps_the_in_process_lifecycle() {
    let ownership = Ownership::resolve(None).expect("an unconfigured Host resolves");
    assert!(
        matches!(ownership, Ownership::InProcess),
        "a Host with no Runtime owns its Instances in this process, as it always has"
    );
    let instance = instance('1');
    ownership
        .register(&instance, &operation('2'), 3)
        .expect("registration is nothing to do without a Host Runtime");
    assert!(
        !ownership.is_live(&instance),
        "nothing outside this process can be asked about the Instance"
    );
    assert!(
        ownership.release(&instance).expect("release"),
        "ownership no Host holds is ended by definition"
    );
}

#[test]
fn a_configured_host_runtime_that_is_not_serving_is_refused_rather_than_ignored() {
    // Inside a directory of this case's own, so the name is absent by construction rather than
    // by the hope that nothing else ever created it.
    let directory = tempfile::tempdir().expect("a temporary directory");
    let absent = directory.path().join("no-such-hostd.sock");
    let error = Ownership::resolve(Some(absent.as_os_str()))
        .err()
        .expect("a configured Runtime that nothing serves is a failure");
    assert!(
        matches!(error, ClientError::Connect(_)),
        "the refusal names the connection, so an operator is not told the request was rejected"
    );
}

#[test]
fn a_same_instance_restore_can_allocate_again_after_its_first_attempt_terminated() {
    use super::super::identity::LaunchIdentity;
    use soma::BackendFailureKind;

    let host = Host::serve();
    let instance = instance('6');
    let operation = operation('7');
    let first = LaunchIdentity::derive(&instance).expect("first attempt");
    let second = LaunchIdentity::derive(&instance).expect("second attempt");
    let owner = host.connect();

    owner
        .register_jailed(&instance, &operation, first, true)
        .expect("first restore attempt registered");
    assert_eq!(
        owner.register_jailed(&instance, &operation, second, true),
        Err(BackendFailureKind::ResourceConflict),
        "a live registration cannot be replaced"
    );
    assert!(owner.release(&instance).expect("release the first attempt"));
    assert!(!owner.is_live(&instance));

    owner
        .register_jailed(&instance, &operation, second, true)
        .expect("retry can allocate after confirmed release");
    assert!(owner.is_live(&instance));
    assert!(
        owner
            .release(&instance)
            .expect("release the second attempt")
    );
}

#[test]
fn a_registered_instance_is_addressable_from_a_second_client() {
    let host = Host::serve();
    let instance = instance('3');

    let launcher = host.connect();
    launcher
        .register(&instance, &operation('4'), 7)
        .expect("the Host accepts the registration");
    // Dropping the client that registered the Instance closes its connection and nothing else,
    // which is the whole difference from the in-process handle this replaces.
    drop(launcher);

    let observer = host.connect();
    assert!(
        observer.is_live(&instance),
        "a client that never launched the Instance still addresses it by identity"
    );
    assert!(
        !observer.is_live(&self::instance('5')),
        "an identity the Host never launched is not reported live"
    );
}

#[test]
fn a_release_from_a_second_client_is_terminal() {
    let host = Host::serve();
    let instance = instance('6');

    let launcher = host.connect();
    launcher
        .register(&instance, &operation('7'), 8)
        .expect("the Host accepts the registration");
    drop(launcher);

    let destroyer = host.connect();
    assert!(
        destroyer.release(&instance).expect("release"),
        "a client that never launched the Instance may still end it"
    );
    assert!(
        destroyer.release(&instance).expect("repeat"),
        "the repeat is answered from the durable record rather than refused"
    );
    assert!(
        !destroyer.is_live(&instance),
        "the released Instance is no longer live"
    );
}
