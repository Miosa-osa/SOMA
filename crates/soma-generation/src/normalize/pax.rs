use std::io::Read;

use super::{NormalizeError, NormalizeErrorKind, NormalizePhase};

pub(super) struct EffectiveNames {
    pub(super) path: Vec<u8>,
    pub(super) link: Option<Vec<u8>>,
}

pub(super) fn effective_names<R: Read>(
    entry: &mut tar::Entry<'_, R>,
) -> Result<EffectiveNames, NormalizeError> {
    let mut pax_path = None;
    let mut pax_link = None;
    if let Some(extensions) = entry.pax_extensions().map_err(|_| invalid())? {
        for extension in extensions {
            let extension = extension.map_err(|_| invalid())?;
            let value = extension.value_bytes();
            if std::str::from_utf8(value).is_err() {
                return Err(invalid());
            }
            match extension.key_bytes() {
                b"path" if pax_path.is_none() => pax_path = Some(value.to_vec()),
                b"linkpath" if pax_link.is_none() => pax_link = Some(value.to_vec()),
                b"path" | b"linkpath" => return Err(invalid()),
                _ => return Err(unsupported()),
            }
        }
    }
    Ok(EffectiveNames {
        path: pax_path.unwrap_or_else(|| entry.path_bytes().into_owned()),
        link: pax_link.or_else(|| entry.link_name_bytes().map(std::borrow::Cow::into_owned)),
    })
}

const fn invalid() -> NormalizeError {
    NormalizeError::new(NormalizePhase::ApplyLayer, NormalizeErrorKind::InvalidInput)
}

const fn unsupported() -> NormalizeError {
    NormalizeError::new(NormalizePhase::ApplyLayer, NormalizeErrorKind::Unsupported)
}
