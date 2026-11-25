//! The owner records, on disk and in memory alike.

use super::*;

const ID: &str = "0123456789abcdef0123456789abcdef";

/// A directory of its own for one test, removed when the test ends.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("soma-owners-{}", unique()));
        fs::create_dir_all(&path).expect("scratch directory");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.0);
    }
}

fn both() -> (Scratch, [Owners; 2]) {
    let directory = Scratch::new();
    let stores = [
        Owners::in_directory(directory.path().join("owners")),
        Owners::in_memory(),
    ];
    (directory, stores)
}

#[test]
fn the_first_tenant_owns_an_identity_and_another_reads_it_as_taken() {
    let (_directory, stores) = both();
    for owners in &stores {
        assert_eq!(
            owners.access(Resource::Sandbox, ID, "a").unwrap(),
            Access::Unclaimed
        );
        assert_eq!(
            owners.claim(Resource::Sandbox, ID, "a").unwrap(),
            Claim::New
        );
        assert_eq!(
            owners.claim(Resource::Sandbox, ID, "a").unwrap(),
            Claim::Kept
        );
        assert_eq!(
            owners.claim(Resource::Sandbox, ID, "b").unwrap(),
            Claim::Refused
        );
        assert_eq!(
            owners.access(Resource::Sandbox, ID, "a").unwrap(),
            Access::Owner
        );
        assert_eq!(
            owners.access(Resource::Sandbox, ID, "b").unwrap(),
            Access::Other
        );
        assert_eq!(
            owners.access(Resource::Volume, ID, "b").unwrap(),
            Access::Unclaimed,
            "a sandbox and a volume with the same id are separate records"
        );
        owners.forget(Resource::Sandbox, ID).unwrap();
        assert_eq!(
            owners.claim(Resource::Sandbox, ID, "b").unwrap(),
            Claim::New
        );
    }
}

#[test]
fn a_record_outlives_the_store_that_wrote_it() {
    let directory = Scratch::new();
    let upper = ID.to_ascii_uppercase();
    Owners::in_directory(directory.path().to_path_buf())
        .claim(Resource::Volume, &upper, "a")
        .unwrap();
    let reopened = Owners::in_directory(directory.path().to_path_buf());
    assert_eq!(
        reopened.access(Resource::Volume, ID, "b").unwrap(),
        Access::Other
    );
    assert_eq!(
        reopened.access(Resource::Volume, ID, "a").unwrap(),
        Access::Owner
    );
}

#[test]
fn an_identity_that_is_not_hexadecimal_never_becomes_a_path() {
    let directory = Scratch::new();
    let owners = Owners::in_directory(directory.path().to_path_buf());
    for identity in ["", "../escape", "a/b", &"a".repeat(65)] {
        assert_eq!(
            owners.claim(Resource::Sandbox, identity, "a").unwrap(),
            Claim::Kept
        );
        assert_eq!(owners.owner(Resource::Sandbox, identity).unwrap(), None);
    }
    assert!(
        fs::read_dir(directory.path()).unwrap().next().is_none(),
        "nothing was written"
    );
}
