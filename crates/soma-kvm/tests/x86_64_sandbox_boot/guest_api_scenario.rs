//! The guest API proof, in the order a host would use the API.
//!
//! This file is the sequence; the detail of each group of steps lives in a child module.

use super::*;

/// Everything the proof asserts, in the order a host would use the API.
pub(crate) fn scenario<'a>(repaired: Repaired<'a>) -> Result<Repaired<'a>, String> {
    let want = expected_level();
    let nonce = nonce();
    let repaired = probe_and_plants(repaired, want, &nonce)?;
    if want < 2 {
        return the_old_guest_path(repaired);
    }
    let repaired = super::files::ranged_transfers(repaired)?;
    let repaired = super::files::rename_chmod_and_stat(repaired)?;
    let repaired = super::files::detailed_listing(repaired)?;
    let repaired = super::context::persisted_environment(repaired)?;
    let repaired = super::context::command_context(repaired)?;
    super::context::streaming_output(repaired)
}

/// The level probe, and the files a workload plants under the probe path.
pub(crate) fn probe_and_plants<'a>(
    repaired: Repaired<'a>,
    want: u8,
    nonce: &[u8],
) -> Result<Repaired<'a>, String> {
    // The level probe, then a file planted at the probe path by a workload.
    let (repaired, api) = repaired
        .probe_guest_api(nonce)
        .map_err(|error| format!("probe: {error}"))?;
    eprintln!("[guest-api] probe -> {api:?}");
    check!(
        api == if want >= 2 {
            GuestApi::Level(2)
        } else {
            GuestApi::Legacy
        },
        "probe answered {api:?}, expected level {want}"
    );
    // A workload plants files at the probe path, one with the shape of a genuine answer but a
    // nonce it could not have known. None may change what the host believes.
    let (repaired, made) = file(
        repaired,
        FileRequest::MakeDirectory {
            path: path("/.soma/guest-api"),
            parents: true,
        },
    )?;
    eprintln!("[guest-api] mkdir /.soma/guest-api -> {made:?}");
    check!(
        made == FileOutcome::Done,
        "could not create the probe directory: {made:?}"
    );
    let planted = [crate_probe_prefix(), nonce.to_vec()].concat();
    let mut repaired = repaired;
    for forged in [&b"2\n"[..], b"2 0123456789abcdef\n", b"2"] {
        let (next, written) = repaired
            .write_whole_file(&planted, forged, 64)
            .map_err(|error| format!("plant: {error}"))?;
        repaired = next;
        eprintln!("[guest-api] plant {forged:?} at the probe path -> {written:?}");
        check!(
            written == WholeFileWrite::Written,
            "the plant was not written: {written:?}"
        );
        let (next, seen) = repaired
            .probe_guest_api(nonce)
            .map_err(|error| format!("probe after plant: {error}"))?;
        repaired = next;
        eprintln!("[guest-api] probe after plant -> {seen:?}");
        check!(
            seen == api,
            "a planted file changed the answer: {api:?} -> {seen:?}"
        );
    }
    Ok(repaired)
}

/// The old guest path: everything a level 1 agent did before the API existed.
pub(crate) fn the_old_guest_path<'a>(repaired: Repaired<'a>) -> Result<Repaired<'a>, String> {
    eprintln!(
        "[guest-api] level 1: the host must not send a level 2 request; nothing more is asked"
    );
    // The old guest still does everything it did before: a whole write, a read, a listing.
    let (repaired, written) = repaired
        .write_whole_file(b"/tmp/legacy.txt", b"hello", 64)
        .map_err(|error| format!("legacy write: {error}"))?;
    check!(
        written == WholeFileWrite::Written,
        "legacy write: {written:?}"
    );
    let (repaired, read) = repaired
        .read_whole_file(b"/tmp/legacy.txt", 64)
        .map_err(|error| format!("legacy read: {error}"))?;
    check!(
        read == WholeFileRead::Bytes(b"hello".to_vec()),
        "legacy read: {read:?}"
    );
    let (repaired, listed) = file(
        repaired,
        FileRequest::ReadDirectory {
            path: path("/tmp"),
            offset: 0,
        },
    )?;
    check!(
        matches!(&listed, FileOutcome::Listed { entries, .. } if entries.iter().any(|entry| &*entry.name == b"legacy.txt")),
        "legacy listing: {listed:?}"
    );
    Ok(repaired)
}
