use soma::InstanceId;

/// A public sandbox id: a lowercase hyphenated UUID whose first hex digit names its host.
///
/// The tag sits at index 0, inside the random `time_low` field of a version-4 UUID, so a tagged
/// id is still a valid UUID to every client that already parses one. This is the same splice the
/// host-mode fast lane performs (`Engine.Sandbox.SomaFastLane.HostMode.tag_sandbox_id/1`), so an
/// id minted by either path routes the same way.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SandboxId(String);

impl SandboxId {
    /// Mints a fresh id carrying `tag`.
    #[must_use]
    pub fn mint(tag: char) -> Self {
        let uuid = uuid::Uuid::new_v4().hyphenated().to_string();
        let mut tagged = String::with_capacity(uuid.len());
        tagged.push(tag);
        tagged.push_str(&uuid[1..]);
        Self(tagged)
    }

    /// Reads an id from a request path.
    ///
    /// Uppercase hex is accepted and folded to lowercase, as `Ecto.UUID.cast/1` does, so the same
    /// sandbox has one spelling everywhere it is stored or compared.
    #[must_use]
    pub fn parse(segment: &str) -> Option<Self> {
        let bytes = segment.as_bytes();
        if bytes.len() != 36 {
            return None;
        }
        let well_formed = bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        });
        well_formed.then(|| Self(segment.to_ascii_lowercase()))
    }

    /// The host tag this id carries.
    #[must_use]
    pub fn tag(&self) -> char {
        self.0.chars().next().unwrap_or('0')
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The facade's instance id for this sandbox: the same 32 hex digits without hyphens, which
    /// is how the fast lane has always named a public sandbox to SOMA.
    #[must_use]
    pub fn instance_id(&self) -> Option<InstanceId> {
        InstanceId::new(self.0.replace('-', "")).ok()
    }

    /// Recovers the public id from a facade instance id, the reverse of [`Self::instance_id`].
    #[must_use]
    pub fn from_instance_id(instance_id: &InstanceId) -> Option<Self> {
        let hex = instance_id.as_str();
        if hex.len() != 32 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        Self::parse(&format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        ))
    }
}

impl std::fmt::Display for SandboxId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Decodes the lowercase hex SHA-256 the control plane publishes as `key_hash`.
#[must_use]
pub fn decode_sha256_hex(hex: &str) -> Option<[u8; 32]> {
    let bytes = hex.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut hash = [0_u8; 32];
    let (pairs, _) = bytes.as_chunks::<2>();
    for (slot, pair) in hash.iter_mut().zip(pairs) {
        *slot = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(hash)
}

const fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::SandboxId;

    #[test]
    fn a_minted_id_carries_the_tag_and_is_still_a_version_four_uuid() {
        for tag in ['3', 'a', '1'] {
            let id = SandboxId::mint(tag);

            assert_eq!(id.tag(), tag);
            let parsed = uuid::Uuid::parse_str(id.as_str()).expect("still a UUID");
            assert_eq!(parsed.get_version_num(), 4);
            assert_eq!(SandboxId::parse(id.as_str()), Some(id));
        }
    }

    #[test]
    fn parsing_folds_case_and_refuses_malformed_ids() {
        assert_eq!(
            SandboxId::parse("3F2504E0-4F89-41D3-9A0C-0305E82C3301")
                .expect("uppercase parses")
                .as_str(),
            "3f2504e0-4f89-41d3-9a0c-0305e82c3301"
        );
        for bad in [
            "",
            "3f2504e04f8941d39a0c0305e82c3301",
            "3f2504e0-4f89-41d3-9a0c-0305e82c330",
            "3f2504e0-4f89-41d3-9a0c-0305e82c330g",
            "3f2504e0+4f89-41d3-9a0c-0305e82c3301",
        ] {
            assert!(SandboxId::parse(bad).is_none(), "{bad} must be refused");
        }
    }

    #[test]
    fn the_instance_id_round_trips() {
        let id = SandboxId::parse("a1b2c3d4-0000-4000-8000-000000000001").expect("valid");
        let instance = id.instance_id().expect("hex instance id");

        assert_eq!(instance.as_str(), "a1b2c3d4000040008000000000000001");
        assert_eq!(SandboxId::from_instance_id(&instance), Some(id));
    }
}
