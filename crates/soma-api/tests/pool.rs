#[allow(
    dead_code,
    reason = "each integration test uses a subset of the shared fixtures"
)]
mod support;

use std::time::Duration;

use soma_api::{FacadePool, SandboxStorage};
use support::{FIXTURE_INSTANCE_ID, FakeFacade, Mode};

#[test]
fn a_pooled_lease_reaches_the_facade_for_both_volume_reads() {
    // Every request is served on a lease from the pool. A lease that answered these two from the
    // trait's defaults refused every start that named a volume and never detached one (SOMA#166).
    let volume = "5".repeat(32);
    let pool = FacadePool::open(1, || {
        Ok::<_, ()>(
            FakeFacade::new(Mode::Succeed)
                .with_volume_binding(&volume, "/srv/soma/volumes/05/volume.img")
                .with_owned_volume(&volume, "attached"),
        )
    })
    .expect("open the pool");
    let mut lease = pool
        .acquire_timeout(Duration::from_millis(10))
        .expect("one facade");

    let binding = lease
        .volume_binding(FIXTURE_INSTANCE_ID)
        .expect("the lease forwards the binding");
    assert_eq!(binding.volume, volume);
    let owned = lease
        .volume_owned(FIXTURE_INSTANCE_ID)
        .expect("the lease forwards the owned volume");
    assert_eq!(owned.map(|owned| owned.volume), Some(volume));
}
