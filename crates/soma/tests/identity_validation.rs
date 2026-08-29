use soma::{
    ExecutionLimits, GenerationId, InstanceId, MachineShape, OciDigest, OciPlatform, OperationId,
    RequestFingerprint, WorkloadIdentity,
};

#[test]
fn public_identities_accept_only_their_canonical_forms() {
    assert!(OperationId::new("1".repeat(32)).is_ok());
    assert!(InstanceId::new("2".repeat(32)).is_ok());
    assert!(GenerationId::new(format!("sha256:{}", "3".repeat(64))).is_ok());

    for invalid in ["0".repeat(32), "A".repeat(32), "1".repeat(31)] {
        assert!(OperationId::new(invalid.clone()).is_err());
        assert!(InstanceId::new(invalid).is_err());
    }
    assert!(GenerationId::new("3".repeat(64)).is_err());
    assert!(GenerationId::new(format!("sha256:{}", "A".repeat(64))).is_err());
}

#[test]
fn serde_deserialization_reuses_all_public_validators() {
    assert!(serde_json::from_str::<OperationId>(r#""ABC""#).is_err());
    assert!(serde_json::from_str::<InstanceId>(&format!(r#""{}""#, "0".repeat(32))).is_err());
    assert!(serde_json::from_str::<GenerationId>(r#""generation-1""#).is_err());
    assert!(serde_json::from_str::<OciDigest>(r#""sha256:ABC""#).is_err());
    assert!(
        serde_json::from_str::<OciPlatform>(
            r#"{"operating_system":"LINUX","architecture":"amd64","variant":null}"#,
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<MachineShape>(
            r#"{"vcpu_count":0,"memory_mib":1024,"storage_mib":8192,"capabilities":{"network_policy":"unspecified"}}"#,
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<ExecutionLimits>(r#"{"timeout_ms":0,"max_output_bytes":1024}"#,)
            .is_err()
    );
    assert!(serde_json::from_str::<RequestFingerprint>(r#""sha256:abc""#).is_err());
}

#[test]
fn machine_shape_has_one_shared_portable_boundary() {
    assert!(
        MachineShape::new(
            MachineShape::MIN_VCPU_COUNT,
            MachineShape::MIN_MEMORY_MIB,
            MachineShape::MIN_STORAGE_MIB,
        )
        .is_ok()
    );
    assert!(
        MachineShape::new(
            MachineShape::MAX_VCPU_COUNT,
            MachineShape::MAX_MEMORY_MIB,
            MachineShape::MAX_STORAGE_MIB,
        )
        .is_ok()
    );
    assert!(
        MachineShape::new(
            MachineShape::MIN_VCPU_COUNT,
            MachineShape::MIN_MEMORY_MIB - 1,
            MachineShape::MIN_STORAGE_MIB,
        )
        .is_err()
    );
    assert!(
        MachineShape::new(
            MachineShape::MIN_VCPU_COUNT,
            MachineShape::MIN_MEMORY_MIB,
            0,
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<MachineShape>(
            r#"{"vcpu_count":65536,"memory_mib":1,"storage_mib":1,"capabilities":{"network_policy":"unspecified"}}"#,
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<MachineShape>(
            r#"{"vcpu_count":1,"memory_mib":18446744073709551616,"storage_mib":1,"capabilities":{"network_policy":"unspecified"}}"#,
        )
        .is_err()
    );
    assert_eq!(MachineShape::DEFAULT_VCPU_COUNT, 1);
    assert_eq!(MachineShape::DEFAULT_MEMORY_MIB, 1_024);
    assert_eq!(MachineShape::DEFAULT_STORAGE_MIB, 10_240);
}

#[test]
fn workload_identity_preserves_optional_index_and_platform_manifest_digests() {
    let index = OciDigest::parse(format!("sha256:{}", "a".repeat(64))).expect("index");
    let manifest = OciDigest::parse(format!("sha256:{}", "b".repeat(64))).expect("manifest");
    let workload = WorkloadIdentity::new(manifest.clone(), OciPlatform::linux_arm64(), None)
        .with_index_digest(index.clone());
    let decoded: WorkloadIdentity =
        serde_json::from_slice(&serde_json::to_vec(&workload).expect("encode")).expect("decode");

    assert_eq!(decoded.index_digest(), Some(&index));
    assert_eq!(decoded.manifest_digest(), &manifest);
}
