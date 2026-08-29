use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use soma::{InstanceId, StateRecord, StateStore, StateStoreFailureKind};
use soma_local::FileStateStore;

const INSTANCE: &str = "fedcba9876543210fedcba9876543210";
const CHILD_MODE: &str = "SOMA_LOCAL_TEST_CHILD_MODE";
const CHILD_ROOT: &str = "SOMA_LOCAL_TEST_ROOT";
const CHILD_VALUE: &str = "SOMA_LOCAL_TEST_VALUE";
const CHILD_RESULT: &str = "SOMA_LOCAL_TEST_RESULT";
const CHILD_READY: &str = "SOMA_LOCAL_TEST_READY";
const CHILD_GATE: &str = "SOMA_LOCAL_TEST_GATE";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

#[test]
fn state_created_by_one_process_is_loaded_after_process_restart() {
    let root = TempRoot::new("process-restart");
    let result = root.path().join("child-result");
    let mut child = spawn_child(root.path(), b"from-child", &result, None, None);

    assert!(child.wait().expect("wait for child").success());
    assert_eq!(
        fs::read_to_string(&result).expect("child outcome"),
        "created"
    );
    let mut restarted = FileStateStore::open(root.path()).expect("open restarted store");
    let stored = restarted
        .load(&instance())
        .expect("load state")
        .expect("state exists");
    assert_eq!(stored.record().as_bytes(), b"from-child");
}

#[test]
fn concurrent_processes_have_one_create_winner() {
    let root = TempRoot::new("process-concurrency");
    fs::create_dir_all(root.path()).expect("create test root");
    let gate = root.path().join("gate");
    let result_one = root.path().join("result-one");
    let result_two = root.path().join("result-two");
    let ready_one = root.path().join("ready-one");
    let ready_two = root.path().join("ready-two");
    let mut first = spawn_child(
        root.path(),
        b"first-process",
        &result_one,
        Some(&ready_one),
        Some(&gate),
    );
    let mut second = spawn_child(
        root.path(),
        b"second-process",
        &result_two,
        Some(&ready_two),
        Some(&gate),
    );
    wait_for_files([ready_one.as_path(), ready_two.as_path()]);
    fs::write(&gate, b"go").expect("release child barrier");

    assert!(first.wait().expect("wait for first child").success());
    assert!(second.wait().expect("wait for second child").success());
    let mut outcomes = [
        fs::read_to_string(result_one).expect("first outcome"),
        fs::read_to_string(result_two).expect("second outcome"),
    ];
    outcomes.sort();
    assert_eq!(outcomes, ["conflict", "created"]);
}

#[test]
fn child_create_state_from_environment() {
    if std::env::var_os(CHILD_MODE).is_none() {
        return;
    }
    let root = PathBuf::from(std::env::var_os(CHILD_ROOT).expect("child root"));
    let value = std::env::var_os(CHILD_VALUE)
        .expect("child value")
        .to_string_lossy()
        .into_owned();
    let result = PathBuf::from(std::env::var_os(CHILD_RESULT).expect("child result"));
    if let Some(ready) = std::env::var_os(CHILD_READY) {
        fs::write(ready, b"ready").expect("announce child readiness");
    }
    if let Some(gate) = std::env::var_os(CHILD_GATE) {
        wait_for_files([Path::new(&gate)]);
    }
    let mut store = FileStateStore::open(root).expect("open child store");
    let outcome = match store.create(
        &instance(),
        StateRecord::from_bytes(value.into_bytes()).expect("child record"),
    ) {
        Ok(_) => "created",
        Err(error) if error.kind() == StateStoreFailureKind::Conflict => "conflict",
        Err(error) => panic!("unexpected child store failure: {:?}", error.kind()),
    };
    fs::write(result, outcome).expect("persist child result");
}

fn spawn_child(
    root: &Path,
    value: &[u8],
    result: &Path,
    ready: Option<&Path>,
    gate: Option<&Path>,
) -> Child {
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command
        .args([
            "--exact",
            "child_create_state_from_environment",
            "--nocapture",
        ])
        .env(CHILD_MODE, "create")
        .env(CHILD_ROOT, root)
        .env(CHILD_VALUE, String::from_utf8_lossy(value).as_ref())
        .env(CHILD_RESULT, result);
    if let Some(ready) = ready {
        command.env(CHILD_READY, ready);
    }
    if let Some(gate) = gate {
        command.env(CHILD_GATE, gate);
    }
    command.spawn().expect("spawn child test process")
}

fn wait_for_files<'a>(paths: impl IntoIterator<Item = &'a Path>) {
    let paths = paths.into_iter().collect::<Vec<_>>();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !paths.iter().all(|path| path.exists()) {
        assert!(Instant::now() < deadline, "timed out waiting for child");
        thread::sleep(Duration::from_millis(10));
    }
}

fn instance() -> InstanceId {
    InstanceId::new(INSTANCE).expect("fixture Instance ID")
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let nonce = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "soma-local-process-test-{}-{label}-{nanos}-{nonce}",
            std::process::id()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("soma-local-process-test-"))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
