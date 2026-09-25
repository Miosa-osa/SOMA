//! Layout v4: the capture warm plan a Generation declares, carried in its initramfs.

use soma_generation::{
    CompileErrorKind, Sha256Digest,
    initramfs::{
        CAPTURE_WARM_PATH, INITRAMFS_LAYOUT_VERSION, INITRAMFS_WARM_LAYOUT_VERSION,
        build_initramfs, verify_initramfs,
    },
};
use soma_guest::CaptureWarmPlan;

const INIT: &[u8] = b"#!/bin/sh\nexec /bin/soma-guest-agent\n";
const AGENT: &[u8] = b"synthetic-guest-agent-bytes";
const PLAN: &[u8] = b"/usr/local/bin/node -v\n";

fn plan() -> CaptureWarmPlan {
    CaptureWarmPlan::decode(PLAN).unwrap()
}

fn warm_archive() -> Vec<u8> {
    build_initramfs(INIT, AGENT, Some(&plan()), 1 << 20).unwrap()
}

#[test]
fn no_plan_keeps_layout_v3_and_declares_nothing() {
    let archive = build_initramfs(INIT, AGENT, None, 1 << 20).unwrap();
    let contents = verify_initramfs(&archive).unwrap();
    assert_eq!(contents.layout_version, INITRAMFS_LAYOUT_VERSION);
    assert_eq!(contents.capture_warm, None);
    let name = format!("{CAPTURE_WARM_PATH}\0");
    assert!(
        !archive
            .windows(name.len())
            .any(|window| window == name.as_bytes())
    );
}

#[test]
fn a_plan_is_carried_as_layout_v4_and_verified_back_exactly() {
    let archive = warm_archive();
    assert_eq!(archive, warm_archive(), "the build is deterministic");
    let contents = verify_initramfs(&archive).unwrap();
    assert_eq!(contents.layout_version, INITRAMFS_WARM_LAYOUT_VERSION);
    assert_eq!(contents.capture_warm, Some(plan()));
    assert_eq!(contents.early_init_digest, Sha256Digest::of(INIT));
    assert_eq!(contents.guest_agent_digest, Sha256Digest::of(AGENT));
    let v3 = build_initramfs(INIT, AGENT, None, 1 << 20).unwrap();
    assert_ne!(
        Sha256Digest::of(&archive),
        Sha256Digest::of(&v3),
        "the plan is part of the initramfs digest the manifest binds"
    );
}

#[test]
fn a_plan_that_is_not_canonical_is_refused_even_at_the_same_length() {
    let mut archive = warm_archive();
    let start = archive
        .windows(PLAN.len())
        .position(|window| window == PLAN)
        .unwrap();
    // Same length, so every header field still matches: only the plan codec can refuse it.
    let relative = b"usr/local/bin/node  -v\n";
    assert_eq!(relative.len(), PLAN.len());
    archive[start..start + PLAN.len()].copy_from_slice(relative);
    assert_eq!(
        verify_initramfs(&archive).unwrap_err().kind(),
        CompileErrorKind::InvalidInput
    );
}

#[test]
fn an_empty_warm_entry_is_refused() {
    // A v4 archive whose plan body was emptied: an absent plan is spelled by having no entry.
    let archive = warm_archive();
    let start = archive
        .windows(PLAN.len())
        .position(|window| window == PLAN)
        .unwrap();
    let mut emptied = archive.clone();
    for byte in &mut emptied[start..start + PLAN.len()] {
        *byte = b'\n';
    }
    assert!(verify_initramfs(&emptied).is_err());
}

#[test]
fn the_plan_counts_against_the_byte_bound() {
    let warm = warm_archive();
    let exact = u64::try_from(warm.len()).unwrap();
    assert!(build_initramfs(INIT, AGENT, Some(&plan()), exact).is_ok());
    assert_eq!(
        build_initramfs(INIT, AGENT, Some(&plan()), exact - 1)
            .unwrap_err()
            .kind(),
        CompileErrorKind::LimitExceeded
    );
}
