use soma_guest::{
    Error, GuestLaunchMaterial, HostLaunchMaterial, LAUNCH_PAGE_SIZE, LaunchNetwork, SessionBinding,
};

#[derive(Debug, Eq, PartialEq)]
struct DeliveryRejected;

#[test]
fn fresh_launch_material_crosses_one_page_for_one_instance() {
    let generation = [1; 32];
    let instance = [2; 16];
    let operation = [3; 16];
    let host = HostLaunchMaterial::generate(generation, instance, operation, launch_network())
        .expect("operating-system launch material");
    assert_eq!(format!("{host:?}"), "HostLaunchMaterial([REDACTED])");
    let binding = *host.binding();
    let mut page = [0xA5; LAUNCH_PAGE_SIZE];
    let host = host
        .deliver_with(|encoded| {
            page.copy_from_slice(encoded);
            Ok::<(), ()>(())
        })
        .expect("launch-page delivery");
    assert_eq!(
        format!("{host:?}"),
        "DeliveredHostLaunchMaterial([REDACTED])"
    );
    assert_eq!(host.binding(), &binding);
    drop(host);
    let guest = GuestLaunchMaterial::take_from_page(&mut page).expect("guest launch material");
    assert_eq!(page, [0; LAUNCH_PAGE_SIZE]);
    assert_eq!(guest.network(), launch_network());
    assert_eq!(format!("{guest:?}"), "GuestLaunchMaterial([REDACTED])");
    assert_eq!(guest.binding(), &binding);
    assert_ne!(
        binding,
        SessionBinding::new(generation, instance, operation, [4; 32]).expect("other binding")
    );

    let _guest = guest
        .reseed_with(|seed| {
            assert_ne!(seed, &[0; 64]);
            Ok::<(), ()>(())
        })
        .expect("entropy repair");
}

#[test]
fn every_malformed_launch_page_is_rejected_and_fully_wiped() {
    let host = HostLaunchMaterial::generate([1; 32], [2; 16], [3; 16], launch_network())
        .expect("operating-system launch material");
    let mut canonical = [0; LAUNCH_PAGE_SIZE];
    let _delivered = host
        .deliver_with(|encoded| {
            canonical.copy_from_slice(encoded);
            Ok::<(), ()>(())
        })
        .expect("launch-page delivery");

    let mut malformed = Vec::new();
    for range in [
        0..16,
        20..52,
        52..68,
        68..84,
        84..116,
        116..148,
        148..212,
        212..247,
        247..279,
    ] {
        let mut page = canonical;
        page[range].fill(0);
        malformed.push(page);
    }
    for offset in [16, 18, 212, 247, 278, 279, 310, 311, 4095] {
        let mut page = canonical;
        page[offset] ^= 1;
        malformed.push(page);
    }

    for mut page in malformed {
        assert_eq!(
            GuestLaunchMaterial::take_from_page(&mut page).expect_err("malformed page"),
            Error::LaunchPageRejected
        );
        assert!(page.iter().all(|byte| *byte == 0));
    }
}

#[test]
fn failed_delivery_never_yields_host_handshake_material() {
    let host = HostLaunchMaterial::generate([1; 32], [2; 16], [3; 16], launch_network())
        .expect("operating-system launch material");
    let mut calls = 0;

    let result = host.deliver_with(|page| {
        calls += 1;
        assert_ne!(page, &[0; LAUNCH_PAGE_SIZE]);
        Err(DeliveryRejected)
    });

    assert_eq!(result.expect_err("delivery must fail"), DeliveryRejected);
    assert_eq!(calls, 1);
}

fn launch_network() -> LaunchNetwork {
    LaunchNetwork::new(
        3,
        1,
        [0x02, 0, 0, 0, 0, 1],
        [10, 0, 0, 2],
        24,
        [10, 0, 0, 1],
        [10, 0, 0, 1],
        1,
    )
    .expect("fixed test network")
}
