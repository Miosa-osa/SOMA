use super::*;

#[test]
fn local_pax_cannot_mix_with_gnu_naming_extensions_in_either_order() {
    for (kind, key) in [
        (EntryType::GNULongName, "path"),
        (EntryType::GNULongLink, "linkpath"),
    ] {
        for pax_first in [true, false] {
            let bytes = mixed_naming_extensions(kind, key, pax_first);
            assert_eq!(
                run_preflight(bytes.as_slice(), bytes.len() as u64, IMPORT_POLICY),
                Err(PreflightError::Unsupported)
            );
        }
    }
}

fn mixed_naming_extensions(kind: EntryType, key: &str, pax_first: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut bytes);
        if pax_first {
            append_pax(&mut builder, key);
            append_gnu(&mut builder, kind);
        } else {
            append_gnu(&mut builder, kind);
            append_pax(&mut builder, key);
        }
        builder.finish().unwrap();
    }
    bytes
}
