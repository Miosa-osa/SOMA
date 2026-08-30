//! Shared live-test scaffolding: the guest stand-in and the world namespace.

pub mod burst;
pub mod checks;
pub mod codec;
pub mod control;
pub mod delivery;
pub mod frames;
pub mod session;
pub mod world;

use std::{net::Ipv4Addr, path::Path};

use soma_netd::{
    Broker, BundleId, CleanupGeneration, InstanceId, InterfaceName, NetNamespace, NetworkProfile,
    OperationId, SubnetPlan,
};

/// Fails the test with an explicit prerequisite message when the process cannot create
/// network namespaces, so an unprivileged run never reads as a pass.
pub fn require_privilege() {
    if let Err(error) = NetNamespace::probe_privilege() {
        panic!(
            "prerequisite failed: {error}; run inside the pinned privileged container via scripts/netd-live-tests.sh"
        );
    }
    assert!(
        Path::new("/dev/net/tun").exists(),
        "prerequisite failed: /dev/net/tun is absent"
    );
    assert!(
        Path::new("/usr/sbin/nft").exists(),
        "prerequisite failed: /usr/sbin/nft is absent"
    );
}

pub fn profile() -> NetworkProfile {
    NetworkProfile::new(
        InterfaceName::new(world::UPLINK).expect("uplink"),
        SubnetPlan::new(Ipv4Addr::new(10, 200, 0, 0), 16).expect("leases"),
        SubnetPlan::new(Ipv4Addr::new(10, 201, 0, 0), 16).expect("transit"),
        vec![world::DECLARED_RESOLVER],
        &[world::HOST_ADDRESS.into()],
        &[],
    )
    .expect("profile")
}

pub fn broker(state: &Path, limit: u32) -> Broker {
    Broker::open(
        profile(),
        state,
        CleanupGeneration::new(1).expect("generation"),
        limit,
    )
    .expect("broker")
}

pub fn ids(seed: u8) -> (BundleId, InstanceId, OperationId) {
    let mut bytes = [seed; 16];
    bytes[15] = 1;
    (
        BundleId::new(bytes).expect("bundle"),
        InstanceId::new(bytes).expect("instance"),
        OperationId::new(bytes).expect("operation"),
    )
}

/// Detaches one namespace pin and leaves an ordinary file in its place, so a later teardown
/// cannot enter it and must report an incomplete release.
#[allow(unsafe_code)]
pub fn break_namespace_pin(pin: &Path) {
    let target = std::ffi::CString::new(pin.as_os_str().as_encoded_bytes()).expect("pin path");
    // SAFETY: the path is a valid NUL-terminated string and `MNT_DETACH` has no memory
    // preconditions.
    let detached = unsafe { libc::umount2(target.as_ptr(), libc::MNT_DETACH) };
    assert_eq!(detached, 0, "the namespace pin could not be detached");
    std::fs::write(pin, b"not a namespace").expect("plant a plain file in the pin's place");
}
