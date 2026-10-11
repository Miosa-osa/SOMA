//! The runtime backend a command line asks for.

use clap::ValueEnum;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum BackendSelection {
    #[default]
    Auto,
    Macos,
    Docker,
    Kvm,
}

impl From<BackendSelection> for soma_local::BackendSelection {
    fn from(value: BackendSelection) -> Self {
        match value {
            BackendSelection::Auto => Self::Auto,
            BackendSelection::Macos => Self::Macos,
            BackendSelection::Docker => Self::Docker,
            BackendSelection::Kvm => Self::Kvm,
        }
    }
}
