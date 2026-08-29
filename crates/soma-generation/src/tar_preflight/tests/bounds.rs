use super::*;

#[test]
fn oversized_local_pax_and_gnu_extensions_fail_before_their_body_is_read() {
    for (kind, size) in [
        (EntryType::XHeader, IMPORT_POLICY.pax_record_ceiling + 1),
        (
            EntryType::GNULongName,
            IMPORT_POLICY.long_record_ceiling + 1,
        ),
        (
            EntryType::GNULongLink,
            IMPORT_POLICY.long_record_ceiling + 1,
        ),
    ] {
        let reader = HeaderOnly::new(&header(kind, size));
        assert_eq!(
            run_preflight(reader, 1 << 31, IMPORT_POLICY),
            Err(PreflightError::LimitExceeded)
        );
    }
}

#[test]
fn global_pax_is_rejected_from_its_header_even_when_local_pax_is_allowed() {
    let reader = HeaderOnly::new(&header(EntryType::XGlobalHeader, 1 << 30));
    assert_eq!(
        run_preflight(reader, 1 << 31, IMPORT_POLICY),
        Err(PreflightError::Unsupported)
    );
}

#[test]
fn gnu_sparse_is_rejected_from_its_header_before_its_body_is_read() {
    let reader = HeaderOnly::new(&header(EntryType::GNUSparse, 1 << 30));
    assert_eq!(
        run_preflight(reader, 1 << 31, IMPORT_POLICY),
        Err(PreflightError::Unsupported)
    );
}

#[test]
fn local_pax_records_share_one_aggregate_budget() {
    let bytes = two_local_pax_records();
    let sizes = raw_extension_sizes(&bytes, EntryType::XHeader);
    assert_eq!(sizes.len(), 2);
    let mut budget = PreflightBudget::new(RAW_HEADER_CEILING, sizes[0]);

    assert_eq!(
        preflight(
            bytes.as_slice(),
            bytes.len() as u64,
            IMPORT_POLICY,
            &mut budget,
        ),
        Err(PreflightError::LimitExceeded)
    );
}

#[test]
fn repeated_zero_length_extensions_fail_at_the_header_work_ceiling() {
    assert_header_ceiling(EntryType::XHeader);
}

#[test]
fn repeated_zero_length_ordinary_entries_fail_at_the_header_work_ceiling() {
    assert_header_ceiling(EntryType::Regular);
}

#[test]
fn raw_header_budget_is_shared_across_layer_preflights() {
    let first = ordinary_layer("first");
    let second = ordinary_layer("second");
    let mut budget = PreflightBudget::new(1, EXTENSION_BYTE_CEILING);

    run_layer(&first, &mut budget).unwrap();
    assert_eq!(
        run_layer(&second, &mut budget),
        Err(PreflightError::LimitExceeded)
    );
}

#[test]
fn pax_byte_budget_is_shared_across_layer_preflights() {
    let first = one_local_pax_record("first");
    let second = one_local_pax_record("second");
    assert_shared_extension_budget(&first, &second, EntryType::XHeader);
}

#[test]
fn gnu_extension_byte_budget_is_shared_across_layer_preflights() {
    let first = one_gnu_naming_record();
    let second = one_gnu_naming_record();
    assert_shared_extension_budget(&first, &second, EntryType::GNULongName);
}

fn assert_header_ceiling(kind: EntryType) {
    const CEILING: u32 = 2;
    let mut headers = Vec::new();
    for _ in 0..=CEILING {
        headers.extend_from_slice(&header(kind, 0));
    }
    let mut reader = PrefixOnly::new(&headers);
    let mut budget = PreflightBudget::new(CEILING, EXTENSION_BYTE_CEILING);

    assert_eq!(
        preflight(&mut reader, u64::MAX, IMPORT_POLICY, &mut budget),
        Err(PreflightError::LimitExceeded)
    );
    assert_eq!(reader.position, headers.len());
}

fn assert_shared_extension_budget(first: &[u8], second: &[u8], kind: EntryType) {
    let extension_size = raw_extension_sizes(first, kind)[0];
    let mut budget = PreflightBudget::new(4, extension_size);

    run_layer(first, &mut budget).unwrap();
    assert_eq!(
        run_layer(second, &mut budget),
        Err(PreflightError::LimitExceeded)
    );
}

fn run_layer(bytes: &[u8], budget: &mut PreflightBudget) -> Result<(), PreflightError> {
    preflight(bytes, bytes.len() as u64, IMPORT_POLICY, budget)
}

fn ordinary_layer(path: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut bytes);
        append_empty_file(&mut builder, path);
        builder.finish().unwrap();
    }
    bytes
}

fn one_local_pax_record(path: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut bytes);
        builder
            .append_pax_extensions([("path", path.as_bytes())])
            .unwrap();
        append_empty_file(&mut builder, "placeholder");
        builder.finish().unwrap();
    }
    bytes
}

fn two_local_pax_records() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut bytes);
        for path in ["one", "two"] {
            builder
                .append_pax_extensions([("path", path.as_bytes())])
                .unwrap();
            append_empty_file(&mut builder, "placeholder");
        }
        builder.finish().unwrap();
    }
    bytes
}

fn one_gnu_naming_record() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut bytes);
        append_gnu(&mut builder, EntryType::GNULongName);
        append_empty_file(&mut builder, "placeholder");
        builder.finish().unwrap();
    }
    bytes
}

fn append_empty_file(builder: &mut tar::Builder<&mut Vec<u8>>, path: &str) {
    let mut header = tar::Header::new_ustar();
    header.set_path(path).unwrap();
    header.set_size(0);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append(&header, io::empty()).unwrap();
}

fn raw_extension_sizes(bytes: &[u8], kind: EntryType) -> Vec<u64> {
    let mut archive = tar::Archive::new(bytes);
    archive
        .entries()
        .unwrap()
        .raw(true)
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.header().entry_type() == kind)
        .map(|entry| entry.size())
        .collect()
}
