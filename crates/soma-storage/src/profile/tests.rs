use super::*;

fn recipe(name: &str, version: u32, mib: u64) -> OverlayRecipe {
    OverlayRecipe {
        name: ClassName::new(name).expect("class name"),
        version,
        logical_bytes: LogicalBytes::new(mib * 1024 * 1024, BlockSize::B4096).expect("size"),
        block_size: BlockSize::B4096,
        uuid_policy: UuidPolicy::Derived,
        features: Ext4FeatureSet::V1,
        inode_policy: InodePolicy::bytes_per_inode(16384).expect("ratio"),
        mount_options: MountOptions::new(&[
            MountOption::NoAtime,
            MountOption::NoDev,
            MountOption::NoSuid,
            MountOption::NoAtime,
        ]),
    }
}

fn class(name: &str, version: u32, mib: u64, free_mib: u64) -> OverlayClass {
    OverlayClass::publish(
        recipe(name, version, mib),
        TemplateDigest::from_bytes([u8::try_from(version).expect("small version"); 32]),
        FreeSpaceEvidence {
            minimum_free_bytes: free_mib * 1024 * 1024,
        },
    )
}

#[test]
fn logical_bytes_enforce_range_and_alignment() {
    assert_eq!(
        LogicalBytes::new(MIN_LOGICAL_BYTES - 1, BlockSize::B4096),
        Err(DimensionError::LogicalBytesOutOfRange {
            bytes: MIN_LOGICAL_BYTES - 1
        })
    );
    assert_eq!(
        LogicalBytes::new(MAX_LOGICAL_BYTES + 4096, BlockSize::B4096),
        Err(DimensionError::LogicalBytesOutOfRange {
            bytes: MAX_LOGICAL_BYTES + 4096
        })
    );
    assert_eq!(
        LogicalBytes::new(MIN_LOGICAL_BYTES + 1024, BlockSize::B4096),
        Err(DimensionError::LogicalBytesUnaligned {
            bytes: MIN_LOGICAL_BYTES + 1024,
            block: 4096
        })
    );
    assert_eq!(
        LogicalBytes::new(MIN_LOGICAL_BYTES + 1024, BlockSize::B1024).map(LogicalBytes::get),
        Ok(MIN_LOGICAL_BYTES + 1024)
    );
}

#[test]
fn inode_policy_accepts_only_power_of_two_ratios_in_range() {
    assert!(InodePolicy::bytes_per_inode(16384).is_ok());
    assert_eq!(
        InodePolicy::bytes_per_inode(512),
        Err(DimensionError::InodeRatioInvalid {
            bytes_per_inode: 512
        })
    );
    assert_eq!(
        InodePolicy::bytes_per_inode(3000),
        Err(DimensionError::InodeRatioInvalid {
            bytes_per_inode: 3000
        })
    );
    assert_eq!(
        InodePolicy::bytes_per_inode(128 * 1024 * 1024),
        Err(DimensionError::InodeRatioInvalid {
            bytes_per_inode: 128 * 1024 * 1024
        })
    );
}

#[test]
fn class_names_and_mount_options_are_validated_and_rendered() {
    assert!(ClassName::new("ovl-10g").is_ok());
    assert_eq!(ClassName::new(""), Err(DimensionError::ClassNameInvalid));
    assert_eq!(ClassName::new("Ovl"), Err(DimensionError::ClassNameInvalid));
    assert_eq!(
        ClassName::new("-ovl"),
        Err(DimensionError::ClassNameInvalid)
    );
    assert_eq!(
        ClassName::new("a".repeat(MAX_CLASS_NAME_BYTES + 1)),
        Err(DimensionError::ClassNameInvalid)
    );
    let options = recipe("ovl", 1, 64).mount_options;
    assert_eq!(options.as_slice().len(), 3);
    assert_eq!(options.render(), "noatime,nodev,nosuid");
    assert_eq!(
        Ext4FeatureSet::V1.mke2fs_argument().split(',').next(),
        Some("none")
    );
}

#[test]
fn template_digest_round_trips_through_hex() {
    let digest = TemplateDigest::from_bytes([0xab; 32]);
    let text = digest.to_string();
    assert_eq!(text.len(), 64);
    assert_eq!(TemplateDigest::from_hex(&text), Ok(digest));
    assert_eq!(
        TemplateDigest::from_hex("zz"),
        Err(DimensionError::DigestTextInvalid)
    );
    assert_eq!(
        TemplateDigest::from_hex(&"A".repeat(64)),
        Err(DimensionError::DigestTextInvalid)
    );
    assert_eq!(format!("{digest:?}"), format!("TemplateDigest({text})"));
}

#[test]
fn catalog_resolves_exact_sizes_only() {
    let catalog = ClassCatalog::new(vec![
        class("ovl-1g", 1, 1024, 64),
        class("ovl-4g", 1, 4096, 256),
    ])
    .expect("catalog");
    assert_eq!(catalog.classes().len(), 2);
    let resolved = catalog.resolve(1024 * 1024 * 1024).expect("exact");
    assert_eq!(resolved.recipe().name.as_str(), "ovl-1g");
    assert_eq!(
        resolved.template_digest(),
        TemplateDigest::from_bytes([1; 32])
    );
    assert_eq!(resolved.free_space().minimum_free_bytes, 64 * 1024 * 1024);
    assert_eq!(
        catalog.resolve(1024 * 1024 * 1024 + 4096),
        Err(ClassRejection::NoExactClass {
            requested_bytes: 1024 * 1024 * 1024 + 4096
        })
    );
    assert_eq!(
        catalog.resolve(0),
        Err(ClassRejection::NoExactClass { requested_bytes: 0 })
    );
}

#[test]
fn catalog_rejects_duplicate_identity_and_duplicate_size() {
    let duplicate_identity =
        ClassCatalog::new(vec![class("ovl", 1, 1024, 1), class("ovl", 1, 2048, 1)]);
    assert_eq!(
        duplicate_identity,
        Err(CatalogError::DuplicateIdentity(
            ClassName::new("ovl").expect("name"),
            1
        ))
    );
    let duplicate_size = ClassCatalog::new(vec![class("a", 1, 1024, 1), class("b", 1, 1024, 1)]);
    assert_eq!(
        duplicate_size,
        Err(CatalogError::DuplicateLogicalBytes(1024 * 1024 * 1024))
    );
    assert!(ClassCatalog::new(vec![class("ovl", 1, 1024, 1), class("ovl", 2, 2048, 1)]).is_ok());
}

#[test]
fn classes_serialize_with_validated_names() {
    let class = class("ovl-1g", 3, 1024, 64);
    let json = serde_json::to_string(&class).expect("json");
    let parsed: OverlayClass = serde_json::from_str(&json).expect("parse");
    assert_eq!(parsed, class);
    let tampered = json.replace("\"ovl-1g\"", "\"../ovl\"");
    assert!(serde_json::from_str::<OverlayClass>(&tampered).is_err());
}
