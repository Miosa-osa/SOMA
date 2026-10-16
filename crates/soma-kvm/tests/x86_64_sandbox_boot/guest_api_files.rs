//! The level 2 file operations: ranged transfer, rename, chmod, stat, and a paged listing.
//!
//! They live apart from the sequence so each file stays inside the source budget.

use super::*;

/// A 20 MiB ranged write and read, compared by digest on both sides.
pub(crate) fn ranged_transfers<'a>(repaired: Repaired<'a>) -> Result<Repaired<'a>, String> {
    // Level 2 files: a 20 MiB ranged write and read, compared by digest on both sides.
    let data = pattern(TOTAL);
    let expected = hex(&Sha256::digest(&data));
    let mut repaired = repaired;
    for (index, chunk) in data.chunks(PART).enumerate() {
        let (next, outcome) = repaired
            .write_range(
                RangeWrite {
                    path: b"/tmp/big.bin",
                    offset: (index * PART) as u64,
                    create: index == 0,
                    shorten: (index + 1) * PART >= TOTAL,
                    bytes: chunk,
                },
                PART,
            )
            .map_err(|error| format!("write_range {index}: {error}"))?;
        repaired = next;
        check!(
            outcome == WholeFileWrite::Written,
            "write_range {index}: {outcome:?}"
        );
    }
    let (next, stat) = file(
        repaired,
        FileRequest::Stat {
            path: path("/tmp/big.bin"),
        },
    )?;
    repaired = next;
    eprintln!("[guest-api] stat big.bin -> {stat:?}");
    check!(
        matches!(stat, FileOutcome::Stat { kind: EntryKind::File, size, .. } if size == TOTAL as u64),
        "stat: {stat:?}"
    );
    let mut read_back = Vec::with_capacity(TOTAL);
    let mut offset = 0_u64;
    loop {
        let (next, read) = repaired
            .read_range(b"/tmp/big.bin", offset, PART, PART)
            .map_err(|error| format!("read_range at {offset}: {error}"))?;
        repaired = next;
        let RangeRead::Bytes { bytes, end } = read else {
            return Err(format!("read_range at {offset}: {read:?}"));
        };
        offset += bytes.len() as u64;
        read_back.extend_from_slice(&bytes);
        if end || bytes.is_empty() {
            break;
        }
    }
    let observed = hex(&Sha256::digest(&read_back));
    eprintln!(
        "[guest-api] sha256 host-sent={expected} host-read-back={observed} bytes={}",
        read_back.len()
    );
    check!(
        read_back.len() == TOTAL && observed == expected,
        "the 20 MiB round trip differs"
    );
    let (next, guest_sum, _) = run(
        repaired,
        &["sha256sum", "/tmp/big.bin"],
        CommandContext::default(),
    )?;
    repaired = next;
    eprintln!("[guest-api] sha256 guest-side={}", guest_sum.trim());
    check!(
        guest_sum.starts_with(&expected),
        "the guest computed a different digest: {guest_sum}"
    );

    Ok(repaired)
}

/// Rename replaces atomically; chmod and stat report what the guest made.
pub(crate) fn rename_chmod_and_stat<'a>(
    mut repaired: Repaired<'a>,
) -> Result<Repaired<'a>, String> {
    let (next, outcome) = file(
        repaired,
        FileRequest::Rename {
            path: path("/tmp/big.bin"),
            to: path("/tmp/renamed.bin"),
        },
    )?;
    repaired = next;
    check!(outcome == FileOutcome::Done, "rename: {outcome:?}");
    let (next, outcome) = file(
        repaired,
        FileRequest::SetMode {
            path: path("/tmp/renamed.bin"),
            mode: 0o640,
        },
    )?;
    repaired = next;
    check!(outcome == FileOutcome::Done, "chmod: {outcome:?}");
    let (next, stat) = file(
        repaired,
        FileRequest::Stat {
            path: path("/tmp/renamed.bin"),
        },
    )?;
    repaired = next;
    check!(
        matches!(stat, FileOutcome::Stat { size, mode, .. } if size == TOTAL as u64 && mode == 0o640),
        "stat after rename and chmod: {stat:?}"
    );
    let (next, gone) = file(
        repaired,
        FileRequest::Stat {
            path: path("/tmp/big.bin"),
        },
    )?;
    repaired = next;
    check!(
        gone == FileOutcome::Failed(FileFailure::NotFound),
        "the source remained: {gone:?}"
    );
    Ok(repaired)
}

/// A detailed listing reports sizes and a long directory pages by offset.
pub(crate) fn detailed_listing<'a>(mut repaired: Repaired<'a>) -> Result<Repaired<'a>, String> {
    // A detailed listing reports sizes and a long directory pages by offset.
    let (next, listing) = file(
        repaired,
        FileRequest::ReadDirectoryDetailed {
            path: path("/tmp"),
            offset: 0,
        },
    )?;
    repaired = next;
    eprintln!("[guest-api] detailed listing -> {listing:?}");
    check!(
        matches!(&listing, FileOutcome::ListedDetailed { entries, .. }
            if entries.iter().any(|entry| &*entry.name == b"renamed.bin" && entry.size == TOTAL as u64)),
        "detailed listing lacks renamed.bin with its size"
    );

    Ok(repaired)
}
