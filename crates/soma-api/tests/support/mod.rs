use std::collections::BTreeMap;
use std::io::Cursor;

use soma::{
    BackendFailureKind, DestroyMachineRequest, ExecuteMachineRequest, ExecutionReceipt, FileAnswer,
    FileEntry, FileKind, FileMachineRequest, FileOperation, FileRefusal, InspectMachineRequest,
    LaunchMachineRequest, MachineState, ManagedFailure, ManagedStateError, StopMachineRequest,
};
use soma_api::{
    CommandOutcome, FileOutcome, LifecycleOutcome, Request, SandboxFacade, SandboxSnapshot, handle,
};

/// The instance id carried by the retained receipt these tests replay.
pub const FIXTURE_INSTANCE_ID: &str = "89db112753324c3e890ef78b74381aa5";

/// A real receipt captured from a live KVM run, retained under `tests/fixtures`.
///
/// A recorded receipt is used rather than a hand-built one because the facade validates receipts
/// on the way in, so a fabricated document would only prove that the fixture is wrong.
const RECEIPT: &str = include_str!("../fixtures/receipt.json");

#[must_use]
pub fn receipt() -> ExecutionReceipt {
    serde_json::from_str(RECEIPT).expect("the retained receipt is a valid execution receipt")
}

/// What the fake facade should do when the handler reaches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Succeed,
    NotFound,
    Conflict,
    /// The backend holds no machine a later call could address, which is what a macOS host does.
    Unsupported,
}

/// Which facade call the handler made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    Launch,
    Inspect,
    Execute,
    File,
    Stop,
    Destroy,
}

/// A facade that answers from a retained receipt and records what it was asked to do.
///
/// It exists so every route can be proved without KVM, and so a test can assert that a refused
/// route never reached the engine at all.
pub struct FakeFacade {
    mode: Mode,
    pub calls: Vec<Call>,
    hosts_addressable_sandboxes: bool,
    /// A flat in-memory filesystem, so every filesystem route can be proved without KVM.
    files: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl FakeFacade {
    #[must_use]
    pub const fn new(mode: Mode) -> Self {
        Self {
            mode,
            calls: Vec::new(),
            hosts_addressable_sandboxes: true,
            files: BTreeMap::new(),
        }
    }

    /// A facade whose backend keeps the machine in the process that created it.
    #[must_use]
    pub fn without_addressable_sandboxes(mut self) -> Self {
        self.hosts_addressable_sandboxes = false;
        self
    }

    fn record(&mut self, call: Call) -> Result<(), ManagedFailure> {
        self.calls.push(call);
        match self.mode {
            Mode::Succeed => Ok(()),
            Mode::NotFound => Err(ManagedFailure::State(ManagedStateError::MachineNotFound)),
            Mode::Conflict => Err(ManagedFailure::State(ManagedStateError::OperationConflict)),
            Mode::Unsupported => Err(ManagedFailure::Backend(BackendFailureKind::Unsupported)),
        }
    }

    fn lifecycle(&mut self, call: Call) -> Result<LifecycleOutcome, ManagedFailure> {
        self.record(call)?;
        let receipt = receipt();
        Ok(LifecycleOutcome {
            instance_id: receipt.instance_id().clone(),
            receipt,
        })
    }
}

impl SandboxFacade for FakeFacade {
    fn hosts_addressable_sandboxes(&self) -> bool {
        self.hosts_addressable_sandboxes
    }

    fn launch(
        &mut self,
        _request: LaunchMachineRequest,
    ) -> Result<LifecycleOutcome, ManagedFailure> {
        self.lifecycle(Call::Launch)
    }

    fn inspect(
        &mut self,
        _request: InspectMachineRequest,
    ) -> Result<SandboxSnapshot, ManagedFailure> {
        self.record(Call::Inspect)?;
        let receipt = receipt();
        Ok(SandboxSnapshot {
            instance_id: receipt.instance_id().clone(),
            state: MachineState::Ready,
            backend: receipt.backend(),
            receipt,
        })
    }

    fn execute(
        &mut self,
        _request: ExecuteMachineRequest,
    ) -> Result<CommandOutcome, ManagedFailure> {
        self.record(Call::Execute)?;
        let receipt = receipt();
        Ok(CommandOutcome {
            instance_id: receipt.instance_id().clone(),
            status: *receipt.terminal_status(),
            stdout: b"v22.23.2\n".to_vec(),
            stderr: Vec::new(),
            receipt,
        })
    }

    fn file(&mut self, request: FileMachineRequest) -> Result<FileOutcome, ManagedFailure> {
        self.record(Call::File)?;
        let operation = request.operation().clone();
        let answer = answer_for(&mut self.files, &operation);
        Ok(FileOutcome {
            instance_id: soma::InstanceId::new(FIXTURE_INSTANCE_ID)
                .expect("the fixture instance id is canonical"),
            operation: operation.name(),
            answer,
        })
    }

    fn stop(&mut self, _request: StopMachineRequest) -> Result<LifecycleOutcome, ManagedFailure> {
        self.lifecycle(Call::Stop)
    }

    fn destroy(
        &mut self,
        _request: DestroyMachineRequest,
    ) -> Result<LifecycleOutcome, ManagedFailure> {
        self.lifecycle(Call::Destroy)
    }
}

/// Drives one raw HTTP request through the parser, the router, and the handler.
///
/// The whole path is exercised rather than the handler alone, so a test that names a route names
/// the bytes a client would actually send to reach it.
#[must_use]
pub fn call(facade: &mut FakeFacade, raw: &str) -> (u16, serde_json::Value) {
    let mut reader = Cursor::new(raw.as_bytes().to_vec());
    let request = Request::read_from(&mut reader).expect("the fixture request parses");
    let response = handle(facade, &request);
    let body =
        serde_json::from_slice(&response.body).expect("every response body is a JSON envelope");
    (response.status, body)
}

/// Builds a raw HTTP request carrying a tenant identity.
#[must_use]
pub fn identified(method: &str, path: &str, body: &str) -> String {
    request_with_headers(method, path, "x-soma-tenant: acme\r\n", body)
}

/// Builds a raw HTTP request with no tenant identity at all.
#[must_use]
pub fn anonymous(method: &str, path: &str, body: &str) -> String {
    request_with_headers(method, path, "", body)
}

fn request_with_headers(method: &str, path: &str, headers: &str, body: &str) -> String {
    format!(
        "{method} {path} HTTP/1.1\r\nhost: localhost\r\n{headers}content-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// The in-memory answer to one operation, deliberately flat: directories exist only as the
/// prefixes of the paths stored under them.
fn answer_for(files: &mut BTreeMap<Vec<u8>, Vec<u8>>, operation: &FileOperation) -> FileAnswer {
    match operation {
        FileOperation::Read { path } => {
            files
                .get(path)
                .map_or(FileAnswer::Refused(FileRefusal::NotFound), |bytes| {
                    FileAnswer::Read {
                        bytes: bytes.clone(),
                    }
                })
        }
        FileOperation::Write { path, bytes } => {
            if path.starts_with(b"/readonly/") {
                return FileAnswer::Refused(FileRefusal::Denied);
            }
            files.insert(path.clone(), bytes.clone());
            FileAnswer::Written {
                bytes: bytes.len() as u64,
            }
        }
        FileOperation::MakeDirectory { .. } => FileAnswer::Done,
        FileOperation::ReadDirectory { path } => {
            let mut prefix = path.clone();
            if prefix.last() != Some(&b'/') {
                prefix.push(b'/');
            }
            FileAnswer::Listed {
                entries: files
                    .keys()
                    .filter_map(|stored| stored.strip_prefix(prefix.as_slice()))
                    .filter(|name| !name.is_empty() && !name.contains(&b'/'))
                    .map(|name| FileEntry {
                        name: name.to_vec(),
                        kind: FileKind::File,
                    })
                    .collect(),
                more: false,
            }
        }
        FileOperation::Exists { path } => FileAnswer::Status {
            kind: files.contains_key(path).then_some(FileKind::File),
        },
        FileOperation::Remove { path, .. } => {
            if files.remove(path).is_some() {
                FileAnswer::Done
            } else {
                FileAnswer::Refused(FileRefusal::NotFound)
            }
        }
    }
}
