use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendClass {
    DevelopmentOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationKind {
    VirtualMachinePerOciContainer,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ComponentVersion {
    name: String,
    version: String,
    build_type: String,
    commit: String,
}

impl ComponentVersion {
    pub(crate) fn new(name: String, version: String, build_type: String, commit: String) -> Self {
        Self {
            name,
            version,
            build_type,
            commit,
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub fn build_type(&self) -> &str {
        &self.build_type
    }

    #[must_use]
    pub fn commit(&self) -> &str {
        &self.commit
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CapabilityReport {
    backend_class: BackendClass,
    isolation: IsolationKind,
    runtime_ready: bool,
    cli: ComponentVersion,
    api_server: Option<ComponentVersion>,
}

impl CapabilityReport {
    pub(crate) const fn new(cli: ComponentVersion, api_server: Option<ComponentVersion>) -> Self {
        Self {
            backend_class: BackendClass::DevelopmentOnly,
            isolation: IsolationKind::VirtualMachinePerOciContainer,
            runtime_ready: true,
            cli,
            api_server,
        }
    }

    #[must_use]
    pub const fn backend_class(&self) -> BackendClass {
        self.backend_class
    }

    #[must_use]
    pub const fn isolation(&self) -> IsolationKind {
        self.isolation
    }

    #[must_use]
    pub const fn runtime_ready(&self) -> bool {
        self.runtime_ready
    }

    #[must_use]
    pub const fn cli(&self) -> &ComponentVersion {
        &self.cli
    }

    #[must_use]
    pub const fn api_server(&self) -> Option<&ComponentVersion> {
        self.api_server.as_ref()
    }
}
