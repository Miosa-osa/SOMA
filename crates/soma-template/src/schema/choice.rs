//! Closed choice sets of the document with their stable wire discriminants.

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum EgressIntent {
    Deny,
    Allowlist,
    Unrestricted,
}

impl EgressIntent {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Allowlist => "allowlist",
            Self::Unrestricted => "unrestricted",
        }
    }

    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Deny => 0,
            Self::Allowlist => 1,
            Self::Unrestricted => 2,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Deny),
            1 => Some(Self::Allowlist),
            2 => Some(Self::Unrestricted),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum IngressIntent {
    Deny,
    Unrestricted,
}

impl IngressIntent {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Unrestricted => "unrestricted",
        }
    }

    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Deny => 0,
            Self::Unrestricted => 1,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Deny),
            1 => Some(Self::Unrestricted),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum IdleAction {
    Destroy,
    Stop,
    Checkpoint,
}

impl IdleAction {
    pub const ALL: [Self; 3] = [Self::Destroy, Self::Stop, Self::Checkpoint];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Destroy => "destroy",
            Self::Stop => "stop",
            Self::Checkpoint => "checkpoint",
        }
    }

    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Destroy => 0,
            Self::Stop => 1,
            Self::Checkpoint => 2,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Destroy),
            1 => Some(Self::Stop),
            2 => Some(Self::Checkpoint),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum SecretDelivery {
    Environment,
    File,
    EgressProxy,
}

impl SecretDelivery {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::File => "file",
            Self::EgressProxy => "egress-proxy",
        }
    }

    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Environment => 0,
            Self::File => 1,
            Self::EgressProxy => 2,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Environment),
            1 => Some(Self::File),
            2 => Some(Self::EgressProxy),
            _ => None,
        }
    }
}
