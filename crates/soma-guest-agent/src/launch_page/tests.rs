use soma_guest::{HostLaunchMaterial, LAUNCH_PAGE_GUEST_ADDRESS, LAUNCH_PAGE_SIZE, LaunchNetwork};

use super::page::{PAGE_DOMAIN, Page, WORD_BYTES, Word};
use super::{LaunchPageError, consume};

fn delivered_page() -> Page {
    let network = LaunchNetwork::new(
        3,
        1,
        [0x02, 0, 0, 0, 0, 1],
        [10, 0, 0, 2],
        24,
        [10, 0, 0, 1],
        [10, 0, 0, 1],
        1,
    )
    .expect("network");
    let host = HostLaunchMaterial::generate([1; 32], [2; 16], [3; 16], network).expect("host");
    let mut page = Page::filled(0xA5);
    host.deliver_with(|bytes| {
        page.bytes_mut().copy_from_slice(bytes);
        Ok::<(), ()>(())
    })
    .expect("delivery");
    page
}

#[test]
fn a_valid_page_is_consumed_and_the_view_is_zero_afterwards() {
    let mut view = delivered_page();
    assert!(view.has_domain());

    let material = consume(&mut view).expect("material");

    assert_eq!(*view.bytes_mut(), [0; LAUNCH_PAGE_SIZE]);
    assert!(!view.has_domain());
    assert_eq!(material.binding().instance(), &[2; 16]);
    assert_eq!(material.network().vsock_cid(), 3);
}

#[test]
fn a_malformed_page_is_rejected_but_still_erased() {
    let mut view = delivered_page();
    view.bytes_mut()[100] ^= 1;

    assert_eq!(
        consume(&mut view).expect_err("malformed page"),
        LaunchPageError::Rejected
    );
    assert_eq!(*view.bytes_mut(), [0; LAUNCH_PAGE_SIZE]);
}

#[test]
fn an_absent_page_is_not_present() {
    assert!(!Page::zeroed().has_domain());
    assert!(!Page::filled(0xFF).has_domain());
    assert_eq!(LAUNCH_PAGE_GUEST_ADDRESS, 0xd010_0000);
    assert_eq!(LAUNCH_PAGE_GUEST_ADDRESS % 4096, 0);
}

#[test]
fn a_page_is_whole_words_of_bytes() {
    // The word walk is only sound because the page divides exactly, and the domain probe only
    // reads whole words because the domain does too.
    assert_eq!(LAUNCH_PAGE_SIZE % WORD_BYTES, 0);
    assert_eq!(PAGE_DOMAIN.len() % WORD_BYTES, 0);
    assert!(PAGE_DOMAIN.len() < LAUNCH_PAGE_SIZE);
    assert_eq!(
        std::mem::align_of::<Page>() % std::mem::align_of::<Word>(),
        0
    );
}
