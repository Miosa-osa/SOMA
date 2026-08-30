//! Snapshots this host must refuse, and the object scan that proves what the snapshot is not.
//!
//! A rejection has to happen before anything runs: a flipped byte in the state manifest, a
//! flipped byte in the memory object, and a foreign CPU template all fail while the machine is
//! still nothing but a decoded manifest.

use std::{fs, path::Path, time::Duration};

use soma_guest::{GuestLaunchMaterial, HostLaunchMaterial, LAUNCH_PAGE_SIZE, LaunchNetwork};
use soma_kvm::snapshot::compatibility::Incompatibility;
use soma_kvm::x86_64::{
    LAUNCH_PAGE_GPA, Milestone, RestoreRequest, SandboxDisks, SnapshotError, SnapshotPaths, restore,
};

use crate::{
    x86_64_sandbox_boot_host::require_kvm, x86_64_sandbox_boot_session as session,
    x86_64_snapshot_restore_fixture as fixture, x86_64_snapshot_restore_report as report,
};

/// The magic every launch page starts with; it must never appear in a published object.
const LAUNCH_PAGE_MAGIC: &[u8] = b"SOMA-LAUNCH-PAGE";
/// Byte offset of the CPU-template digest inside the fixed manifest header.
///
/// `SOMASNP\0` plus the schema version, architecture, page size, Generation identity, and the
/// machine and device contract digests: 8 + 2 + 2 + 4 + 32 + 32 + 32.
const CPU_TEMPLATE_OFFSET: usize = 112;
const EXIT_GRACE: Duration = Duration::from_secs(10);

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn a_tampered_object_is_rejected_before_any_vcpu_exists() {
    require_kvm();
    let fixture = fixture::shared();

    let state = tampered(&fixture, "tampered-state", |paths, source| {
        report::tamper(&source.state(), &paths.state(), 400);
        link(&source.memory(), &paths.memory());
        link(&source.overlay(), &paths.overlay());
    });
    let error = attempt(&fixture, &state, false).expect_err("a tampered manifest was accepted");
    eprintln!("[tamper] state.somasnap -> {error}");
    assert!(
        matches!(
            error,
            SnapshotError::Manifest(_) | SnapshotError::Section(_)
        ),
        "unexpected rejection: {error:?}"
    );

    let memory = tampered(&fixture, "tampered-memory", |paths, source| {
        report::tamper(&source.memory(), &paths.memory(), 4096 * 300);
        link(&source.state(), &paths.state());
        link(&source.overlay(), &paths.overlay());
    });
    let error =
        attempt(&fixture, &memory, true).expect_err("a tampered memory object was accepted");
    eprintln!("[tamper] memory.raw -> {error}");
    assert!(
        matches!(error, SnapshotError::Memory(_)),
        "unexpected rejection: {error:?}"
    );
    // Without the installation-time verification the same object maps and runs, which is
    // exactly why the digest check is the installation boundary rather than a request-time one.
    assert!(attempt(&fixture, &memory, false).is_ok());
}

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn a_foreign_cpu_template_rejects_the_snapshot() {
    require_kvm();
    let fixture = fixture::shared();
    let foreign = tampered(&fixture, "foreign-template", |paths, source| {
        report::tamper(&source.state(), &paths.state(), CPU_TEMPLATE_OFFSET);
        link(&source.memory(), &paths.memory());
        link(&source.overlay(), &paths.overlay());
    });
    let error = attempt(&fixture, &foreign, false).expect_err("a foreign template was accepted");
    eprintln!("[compatibility] cpu template -> {error}");
    assert!(
        matches!(
            error,
            SnapshotError::Incompatible(Incompatibility::CpuTemplate { .. })
        ),
        "unexpected rejection: {error:?}"
    );
}

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn the_published_objects_carry_no_launch_material() {
    require_kvm();
    let fixture = fixture::shared();
    let memory = report::read(&fixture.paths.memory());
    let overlay = report::read(&fixture.paths.overlay());
    let state = report::read(&fixture.paths.state());

    for (name, bytes) in [("overlay.raw", &overlay), ("state.somasnap", &state)] {
        assert_eq!(
            report::occurrences(bytes, LAUNCH_PAGE_MAGIC),
            0,
            "{name} carries launch-page material"
        );
    }
    // The magic does occur in guest RAM, because the pinned agent's own code contains the
    // constant it compares against. What must not exist is a page: every occurrence is fed to
    // the production decoder, and none of them is a launch page.
    let agent = report::read(&fixture.agent);
    let in_agent = report::occurrences(&agent, LAUNCH_PAGE_MAGIC);
    let offsets = report::offsets(&memory, LAUNCH_PAGE_MAGIC);
    assert!(
        in_agent > 0,
        "the pinned guest agent does not contain the launch-page domain, so the count in \
         guest RAM has no innocent explanation"
    );
    for offset in &offsets {
        let Some(window) = memory.get(*offset..offset + LAUNCH_PAGE_SIZE) else {
            continue;
        };
        let mut page = [0_u8; LAUNCH_PAGE_SIZE];
        page.copy_from_slice(window);
        assert!(
            GuestLaunchMaterial::take_from_page(&mut page).is_err(),
            "a valid launch page exists at offset {offset} of the captured memory"
        );
    }
    eprintln!(
        "[scan] the launch-page domain occurs {} times in memory.raw and {in_agent} times in \
         the pinned agent binary; none of them decodes as a launch page",
        offsets.len()
    );
    // Since ADR 0024 the responder secret is Instance authority delivered through the launch
    // page, never a Generation input, and capture happens before any launch material exists.
    // A freshly generated Instance's own secret halves must therefore appear nowhere in the
    // captured objects: the snapshot carries no authority to reuse.
    let fresh = HostLaunchMaterial::generate(
        fixture.generation_id,
        session::random16(),
        session::random16(),
        LaunchNetwork::new(
            9,
            9,
            [0x02, 0, 0, 0, 0, 1],
            [10, 0, 0, 2],
            24,
            [10, 0, 0, 1],
            [10, 0, 0, 1],
            session::now_unix_nanos(),
        )
        .expect("placeholder network"),
    )
    .expect("fresh Instance launch material");
    let public = fresh.responder_public_key().to_bytes();
    for (name, bytes) in [
        ("memory.raw", &memory),
        ("overlay.raw", &overlay),
        ("state.somasnap", &state),
    ] {
        assert_eq!(
            report::occurrences(bytes, &public),
            0,
            "{name} carries an Instance responder identity"
        );
    }
    eprintln!(
        "[scan] no Instance responder identity appears in memory.raw, overlay.raw, or \
         state.somasnap; capture precedes every launch page"
    );

    assert_eq!(u64::try_from(memory.len()).unwrap(), fixture.ram_bytes);
    assert_eq!(
        report::digest(&fixture.paths.memory()),
        fixture.capture.memory_digest.to_string(),
        "the published memory object is not the one the manifest names"
    );
    assert_eq!(fixture.capture.memory_bytes, fixture.ram_bytes);
    assert_eq!(
        u64::try_from(overlay.len()).unwrap(),
        fixture.capture.overlay_bytes
    );
    assert_eq!(
        u64::try_from(state.len()).unwrap(),
        fixture.capture.state_bytes
    );
    assert!(
        LAUNCH_PAGE_GPA >= fixture.ram_bytes,
        "the launch-page slot lies inside the captured memory image"
    );
    eprintln!(
        "[scan] memory.raw is {} bytes covering [0, {:#x}); the launch page slot is at {:#x}; \
         the source machine entered KVM_RUN {} ns after creation began",
        memory.len(),
        fixture.ram_bytes,
        LAUNCH_PAGE_GPA,
        fixture.source.at(Milestone::RunStart).unwrap_or(0),
    );
}

/// Builds a sibling snapshot directory with one object replaced.
fn tampered(
    fixture: &fixture::Shared,
    name: &str,
    build: impl FnOnce(&SnapshotPaths, &SnapshotPaths),
) -> SnapshotPaths {
    let directory = fixture.scratch.join(name);
    let _ignored = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("create the tampered directory");
    let paths = SnapshotPaths::new(directory);
    build(&paths, &fixture.paths);
    paths
}

fn link(source: &Path, target: &Path) {
    fs::hard_link(source, target).expect("link the untouched object");
}

/// Attempts one restore and releases whatever it produced.
fn attempt(
    fixture: &fixture::Shared,
    paths: &SnapshotPaths,
    verify_artifacts: bool,
) -> Result<(), SnapshotError> {
    let (head_path, head) = fixture.private_head("tamper-head");
    let outcome = restore(RestoreRequest {
        paths: paths.clone(),
        disks: SandboxDisks {
            root: fixture.root(),
            overlay: head,
        },
        guest_cid: 30,
        memory_bytes: fixture.ram_bytes,
        verify_artifacts,
    });
    let result = outcome.map(|restored| drop(restored.machine.finish(EXIT_GRACE)));
    let _ignored = fs::remove_file(&head_path);
    result
}
