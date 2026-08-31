//! Live proof that a directory larger than one listing is paged, and paged exactly once each.
//!
//! One listing carries at most [`MAX_ENTRIES`] names, so any real working directory needs more
//! than one request. The property that matters is not that the pages arrive but that they
//! partition the directory: every name is seen, and no name is seen twice. A component test can
//! only assert that against a directory it invented, so this one asks a real guest to make more
//! entries than one page holds and then walks the listing the caller would walk.

use std::collections::BTreeSet;

use soma_guest::{EntryKind, FileOutcome, FileRequest, MAX_ENTRIES};
use soma_kvm::x86_64::SandboxMachine;

use crate::{
    x86_64_sandbox_boot_host::require_kvm,
    x86_64_snapshot_restore_capability::{assert_no_leak, shell, succeeded},
    x86_64_snapshot_restore_fixture as fixture, x86_64_snapshot_restore_instance as instance,
    x86_64_snapshot_restore_workload::{self as workload, Session, Workload},
};

/// The directory the guest fills and the host pages through.
const PAGED: &[u8] = b"/tmp/soma-paged";
/// How many entries the guest makes: more than one page, and not a whole multiple of one.
const ENTRIES: usize = 1500;
/// The script that makes them, plus one subdirectory so a page carries more than one kind.
const FILL: &[u8] = b"set -e; rm -rf /tmp/soma-paged; mkdir -p /tmp/soma-paged; \
     cd /tmp/soma-paged; i=1; while [ $i -le 1499 ]; do : > \"entry-$i\"; i=$((i+1)); done; \
     mkdir entry-dir; ls -1 | wc -l";
/// Most pages the walk will take before it declares the guest is not making progress.
const PAGE_CEILING: usize = 8;

/// What one paged walk retains.
pub struct Paged {
    pub guest_count: String,
    pub pages: Vec<usize>,
    pub names: Vec<Vec<u8>>,
    pub directories: usize,
}

struct PagedWorkload;

impl Workload for PagedWorkload {
    type Output = Paged;

    fn run<'a>(
        &mut self,
        _machine: &'a SandboxMachine,
        session: Session<'a>,
    ) -> Result<(Session<'a>, Paged), String> {
        let (mut session, executed) = workload::execute(session, &shell(&[b"-c", FILL]))?;
        let guest_count = succeeded("fill", &executed);
        let mut pages = Vec::new();
        let mut names = Vec::new();
        let mut directories = 0_usize;
        let mut offset: u32 = 0;
        loop {
            let (next, outcome) = session
                .file(FileRequest::ReadDirectory {
                    path: PAGED.into(),
                    offset,
                })
                .map_err(|error| format!("list the directory: {error}"))?;
            session = next;
            let FileOutcome::Listed { entries, more } = outcome else {
                return Err(format!("a listing answered with {outcome:?}"));
            };
            pages.push(entries.len());
            for entry in &entries {
                if entry.kind == EntryKind::Directory {
                    directories += 1;
                }
                names.push(entry.name.to_vec());
            }
            // The caller advances by what it was given, which is the only accounting the
            // protocol offers; a page that carries nothing and still claims more would leave
            // this walk asking the same question for ever, so it stops instead.
            if entries.is_empty() || !more {
                return Ok((
                    session,
                    Paged {
                        guest_count,
                        pages,
                        names,
                        directories,
                    },
                ));
            }
            offset = offset.saturating_add(u32::try_from(entries.len()).unwrap_or(u32::MAX));
            if pages.len() > PAGE_CEILING {
                return Err(format!(
                    "the listing did not end within {PAGE_CEILING} pages"
                ));
            }
        }
    }
}

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn a_directory_larger_than_one_listing_pages_and_every_entry_is_seen_once() {
    require_kvm();
    let fixture = fixture::shared();
    let restored = instance::run_workload(&fixture, "files-paging", 42, PagedWorkload);
    assert_no_leak(&restored);

    let paged = &restored.output;
    eprintln!(
        "[paging] the guest reported {} entries; the host took pages {:?} for {} names",
        paged.guest_count.trim(),
        paged.pages,
        paged.names.len()
    );
    assert_eq!(paged.guest_count.trim(), ENTRIES.to_string());
    assert!(
        paged.pages.len() > 1,
        "one page held the whole directory, so nothing about paging was proved"
    );
    assert_eq!(
        paged.pages[0], MAX_ENTRIES,
        "the first page was not the full listing bound"
    );
    assert_eq!(
        paged.names.len(),
        ENTRIES,
        "the walk did not see every entry"
    );

    let unique: BTreeSet<&Vec<u8>> = paged.names.iter().collect();
    assert_eq!(
        unique.len(),
        ENTRIES,
        "the walk saw {} distinct names out of {} entries, so a page repeated one",
        unique.len(),
        paged.names.len()
    );
    for index in 1..ENTRIES {
        let expected = format!("entry-{index}").into_bytes();
        assert!(
            unique.contains(&expected),
            "the walk never saw entry-{index}"
        );
    }
    assert!(
        unique.contains(&b"entry-dir".to_vec()),
        "the walk never saw the one subdirectory"
    );
    assert_eq!(
        paged.directories, 1,
        "the listing reported {} directories where the guest made one",
        paged.directories
    );
}
