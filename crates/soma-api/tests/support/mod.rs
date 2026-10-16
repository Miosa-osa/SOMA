use std::collections::BTreeMap;
use std::io::Cursor;

use soma::{
    BackendFailureKind, DestroyMachineRequest, ExecuteMachineRequest, ExecutionReceipt, FileAnswer,
    FileEntry, FileKind, FileMachineRequest, FileMetadata, FileOperation, FileRefusal,
    InspectMachineRequest, LaunchMachineRequest, MachineState, ManagedFailure, ManagedStateError,
    PtyMachineRequest, SandboxEntry, StopMachineRequest,
};
use soma_api::{
    CommandOutcome, FileOutcome, LifecycleOutcome, Request, SandboxFacade, SandboxSnapshot,
    TerminalOutcome, TunnelFailure, handle,
};

mod terminal;

pub use terminal::Session;

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
    Terminal,
    List,
    Stop,
    Destroy,
    Tunnel(u16),
    Resize(soma_api::ResizeRequest),
}

/// A facade that answers from a retained receipt and records what it was asked to do.
///
/// It exists so every route can be proved without KVM, and so a test can assert that a refused
/// route never reached the engine at all.
pub struct FakeFacade {
    mode: Mode,
    pub calls: Vec<Call>,
    /// The standard input of the last command it was asked to run.
    pub last_stdin: Vec<u8>,
    hosts_addressable_sandboxes: bool,
    /// A flat in-memory filesystem, so every filesystem route can be proved without KVM.
    files: BTreeMap<Vec<u8>, Vec<u8>>,
    /// The one terminal session, for the same reason.
    session: Option<Session>,
    /// What an enumeration answers with.
    sandboxes: Vec<SandboxEntry>,
    /// Records an enumeration found but could not decode.
    unreadable: Vec<soma::UnreadableRecord>,
    /// Why a tunnel is refused; `None` connects it to an echoing guest.
    tunnel_failure: Option<TunnelFailure>,
    /// What a resize answers; `None` is a backend that cannot resize.
    resize_outcome: Option<soma_api::ResizeOutcome>,
    /// What the host daemon answers when a start asks for the volume of an Instance.
    binding: Option<soma_api::volumes::VolumeBinding>,
    /// Every Instance whose volume a start asked about.
    pub volume_calls: Vec<String>,
    /// The volume attachment the last launch carried.
    pub launched_volume: Option<soma::VolumeAttachment>,
    /// The volume the host answers that this Instance owns.
    pub owned: Option<soma_api::volumes::OwnedVolume>,
    /// Every volume operation the routes asked for, by name.
    pub volume_operations: Vec<&'static str>,
}

impl FakeFacade {
    #[must_use]
    pub const fn new(mode: Mode) -> Self {
        Self {
            mode,
            calls: Vec::new(),
            last_stdin: Vec::new(),
            hosts_addressable_sandboxes: true,
            files: BTreeMap::new(),
            session: None,
            sandboxes: Vec::new(),
            unreadable: Vec::new(),
            tunnel_failure: None,
            resize_outcome: None,
            binding: None,
            volume_calls: Vec::new(),
            launched_volume: None,
            owned: None,
            volume_operations: Vec::new(),
        }
    }

    /// A facade whose host answers that this Instance owns this volume, in this state.
    #[must_use]
    pub fn with_owned_volume(mut self, volume: &str, state: &'static str) -> Self {
        self.owned = Some(soma_api::volumes::OwnedVolume {
            volume: volume.to_owned(),
            state,
        });
        self
    }

    /// A facade whose host daemon has this volume attached to the Instance it is asked about.
    #[must_use]
    pub fn with_volume_binding(mut self, volume: &str, path: &str) -> Self {
        self.binding = Some(soma_api::volumes::VolumeBinding {
            volume: volume.to_owned(),
            path: std::path::PathBuf::from(path),
            size_class_gib: 10,
            fsck: soma_api::volumes::FsckReport::from_codes(1, 0),
        });
        self
    }

    /// A facade whose sandboxes resize with `outcome`.
    #[must_use]
    pub fn resizing(mut self, outcome: soma_api::ResizeOutcome) -> Self {
        self.resize_outcome = Some(outcome);
        self
    }

    /// A facade whose tunnels are refused with `failure`.
    #[must_use]
    pub const fn refusing_tunnels(mut self, failure: TunnelFailure) -> Self {
        self.tunnel_failure = Some(failure);
        self
    }

    /// A facade whose durable state holds exactly these sandboxes.
    #[must_use]
    pub fn holding(mut self, sandboxes: Vec<SandboxEntry>) -> Self {
        self.sandboxes = sandboxes;
        self
    }

    /// A facade whose enumeration also names records the store could not decode.
    #[must_use]
    pub fn with_unreadable(mut self, unreadable: Vec<soma::UnreadableRecord>) -> Self {
        self.unreadable = unreadable;
        self
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
                // A directory under /legacy/ plays a level 1 guest: names and kinds, no metadata.
                entries: files
                    .iter()
                    .filter_map(|(stored, bytes)| {
                        stored
                            .strip_prefix(prefix.as_slice())
                            .map(|name| (name, bytes))
                    })
                    .filter(|(name, _)| !name.is_empty() && !name.contains(&b'/'))
                    .map(|(name, bytes)| FileEntry {
                        name: name.to_vec(),
                        kind: FileKind::File,
                        metadata: (!prefix.starts_with(b"/legacy/")).then_some(FileMetadata {
                            size: bytes.len() as u64,
                            mode: 0o644,
                            modified: 1_760_000_000,
                        }),
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
        FileOperation::Stat { .. }
        | FileOperation::Rename { .. }
        | FileOperation::SetMode { .. }
        | FileOperation::ReadRange { .. }
        | FileOperation::WriteRange { .. } => level_two_answer(files, operation),
    }
}

/// The in-memory answer to the guest API level 2 operations.
fn level_two_answer(
    files: &mut BTreeMap<Vec<u8>, Vec<u8>>,
    operation: &FileOperation,
) -> FileAnswer {
    match operation {
        FileOperation::Stat { path } => {
            files
                .get(path)
                .map_or(FileAnswer::Refused(FileRefusal::NotFound), |bytes| {
                    FileAnswer::Stat {
                        kind: FileKind::File,
                        size: bytes.len() as u64,
                        mode: 0o644,
                        modified: 0,
                    }
                })
        }
        FileOperation::Rename { path, to } => match files.remove(path) {
            Some(bytes) => {
                files.insert(to.clone(), bytes);
                FileAnswer::Done
            }
            None => FileAnswer::Refused(FileRefusal::NotFound),
        },
        FileOperation::SetMode { path, .. } => {
            if files.contains_key(path) {
                FileAnswer::Done
            } else {
                FileAnswer::Refused(FileRefusal::NotFound)
            }
        }
        FileOperation::ReadRange {
            path,
            offset,
            length,
        } => files
            .get(path)
            .map_or(FileAnswer::Refused(FileRefusal::NotFound), |bytes| {
                let start = usize::try_from(*offset).map_or(bytes.len(), |o| o.min(bytes.len()));
                let stop = start.saturating_add(*length as usize).min(bytes.len());
                FileAnswer::Range {
                    bytes: bytes[start..stop].to_vec(),
                    end: stop == bytes.len(),
                }
            }),
        FileOperation::WriteRange {
            path,
            offset,
            create,
            shorten,
            bytes,
        } => {
            if !create && !files.contains_key(path) {
                return FileAnswer::Refused(FileRefusal::NotFound);
            }
            let file = files.entry(path.clone()).or_default();
            let start = usize::try_from(*offset).unwrap_or(usize::MAX);
            let stop = start + bytes.len();
            if file.len() < stop || *shorten {
                file.resize(stop, 0);
            }
            file[start..stop].copy_from_slice(bytes);
            FileAnswer::Written {
                bytes: bytes.len() as u64,
            }
        }
        _ => unreachable!("the caller routes only level 2 operations here"),
    }
}

/// A guest port reached through the broker: speaks first, then echoes until told to `quit`.
///
/// The returned end is what the facade hands the service; the thread behind the other end plays
/// the broker's connected socket to the guest's service.
#[must_use]
pub fn echo_guest(port: u16) -> std::os::unix::net::UnixStream {
    use std::io::{Read, Write};
    let (service, guest) = std::os::unix::net::UnixStream::pair().expect("a socket pair");
    std::thread::spawn(move || {
        let mut guest = guest;
        if guest
            .write_all(format!("guest port {port} ready\n").as_bytes())
            .is_err()
        {
            return;
        }
        let mut buffer = [0_u8; 4096];
        loop {
            match guest.read(&mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    if buffer[..read].starts_with(b"quit") {
                        return;
                    }
                    if guest.write_all(&buffer[..read]).is_err() {
                        return;
                    }
                }
            }
        }
    });
    service
}

mod impls;
