use serde::{Deserialize, Serialize};

use super::ProxyProfileSelector;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProxyPolicy {
    Disabled,
    Required { profile: ProxyProfileSelector },
}

impl ProxyPolicy {
    #[must_use]
    pub const fn disabled() -> Self {
        Self::Disabled
    }

    #[must_use]
    pub const fn required(profile: ProxyProfileSelector) -> Self {
        Self::Required { profile }
    }

    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        matches!(self, Self::Disabled)
    }
}
