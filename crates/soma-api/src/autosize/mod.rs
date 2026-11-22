//! Automatic sizing: each sandbox that asks for it is kept between a minimum and a maximum of
//! processors and memory, grown the moment its work needs more and shrunk once it winds down,
//! live, with nothing restarted. Every change is kept in the sandbox's scaling history.
//!
//! The controller reads the guest's own `/proc/stat` and `/proc/meminfo` through the file
//! operation, which mints no receipt and writes nothing durable, and changes size through the same
//! live resize `PATCH /v1/sandboxes/{id}/resources` uses: a CPU quota on the machine's cgroup and
//! the virtio-mem device. A guest only gives back memory it is not using, so a shrink never takes
//! memory a process holds.

mod decide;
mod guest;
mod route;
mod saved;

use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use soma::InstanceId;

use guest::measure;

pub use decide::{Policy, Sample};
pub use route::{AutosizeBody, serve};
pub use saved::{FILE as SAVED_FILE, remember_in};

use crate::facade::SandboxFacade;

/// How often each sandbox is read.
pub const INTERVAL: Duration = Duration::from_secs(2);
/// How many changes a sandbox's history keeps.
const HISTORY: usize = 200;

/// One change of size.
#[derive(Clone, Debug, Serialize)]
pub struct Change {
    pub at_unix: u64,
    /// `cpu` or `memory`.
    pub dimension: &'static str,
    pub from: u64,
    pub to: u64,
}

/// What the controller holds for one sandbox.
#[derive(Clone, Debug)]
struct Entry {
    policy: Policy,
    calm: decide::Calm,
    /// The CPU counters at the last reading: busy ticks, total ticks (or the instant in hundredths
    /// of a second for a host reading), and whether they came from the host.
    ticks: Option<(u64, u64, bool)>,
    last: Option<Sample>,
    history: VecDeque<Change>,
    /// Why the last reading or change did not happen, when one did not.
    problem: Option<String>,
}

static SANDBOXES: LazyLock<Mutex<HashMap<String, Entry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn sandboxes() -> std::sync::MutexGuard<'static, HashMap<String, Entry>> {
    SANDBOXES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Turns automatic sizing on for `instance` with `policy`, or off with `None`, and saves it.
pub fn set(instance: &InstanceId, policy: Option<Policy>) {
    set_quiet(instance, policy);
    saved::save();
}

/// [`set`] without saving, for restoring what was saved.
fn set_quiet(instance: &InstanceId, policy: Option<Policy>) {
    let mut all = sandboxes();
    match policy {
        Some(policy) => {
            let entry = all.entry(instance.as_str().to_owned()).or_insert(Entry {
                policy,
                calm: decide::Calm::default(),
                ticks: None,
                last: None,
                history: VecDeque::new(),
                problem: None,
            });
            entry.policy = policy;
        }
        None => {
            all.remove(instance.as_str());
        }
    }
}

/// What a sandbox's sizing is, for the route.
#[derive(Clone, Debug, Serialize)]
pub struct View {
    pub enabled: bool,
    pub policy: Option<Policy>,
    pub usage: Option<Sample>,
    pub history: Vec<Change>,
    pub problem: Option<String>,
}

/// The sizing of `instance` as the controller holds it.
#[must_use]
pub fn view(instance: &InstanceId) -> View {
    sandboxes().get(instance.as_str()).map_or(
        View {
            enabled: false,
            policy: None,
            usage: None,
            history: Vec::new(),
            problem: None,
        },
        |entry| View {
            enabled: true,
            policy: Some(entry.policy),
            usage: entry.last,
            history: entry.history.iter().rev().cloned().collect(),
            problem: entry.problem.clone(),
        },
    )
}

/// Starts the controller on facades `open` lends it, one reading of every sandbox per interval.
///
/// # Errors
///
/// Returns the failure to start the thread.
pub fn spawn(
    open: impl Fn() -> Option<Box<dyn SandboxFacade>> + Send + 'static,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("soma-autosize".to_owned())
        .spawn(move || {
            loop {
                thread::sleep(INTERVAL);
                let instances: Vec<String> = sandboxes().keys().cloned().collect();
                if instances.is_empty() {
                    continue;
                }
                if let Some(mut facade) = open() {
                    for instance in instances {
                        if let Ok(instance) = InstanceId::new(instance) {
                            tick(facade.as_mut(), &instance);
                        }
                    }
                }
            }
        })
}

/// One reading and, when it calls for one, one change for one sandbox.
pub fn tick<F: SandboxFacade + ?Sized>(facade: &mut F, instance: &InstanceId) {
    let current = match facade.resize(instance, soma_local::ResizeRequest::default()) {
        Ok(current) => current,
        // The sandbox is gone: it was destroyed, stopped, or suspended, so its sizing ends with it.
        Err(soma_local::ResizeFailure::NotFound) => {
            sandboxes().remove(instance.as_str());
            saved::save();
            return;
        }
        Err(soma_local::ResizeFailure::Unavailable) => {
            note(
                instance,
                Some("the sandbox could not be reached".to_owned()),
            );
            return;
        }
    };
    let Some(reading) = measure(facade, instance, current.host_pid) else {
        note(
            instance,
            Some("the sandbox's usage could not be read".to_owned()),
        );
        return;
    };
    let decision = {
        let mut all = sandboxes();
        let Some(entry) = all.get_mut(instance.as_str()) else {
            return;
        };
        let previous = entry.ticks.replace(reading.ticks);
        let Some(previous) = previous.filter(|previous| previous.2 == reading.ticks.2) else {
            return;
        };
        let cpu_used = reading.cores_since(previous, current.cpu.booted);
        let sample = Sample {
            cpu_used,
            cpu_limit: current.cpu.applied,
            memory_used_mb: reading.memory_used_mb,
            // What the machine has, as the device states it; every change is stated against it.
            memory_total_mb: if current.memory.total_mb > 0 {
                current.memory.total_mb
            } else {
                reading.memory_total_mb
            },
        };
        entry.last = Some(sample);
        entry.problem = None;
        let policy = entry.policy;
        let (cpu, memory) = decide::next(&policy, &sample, &mut entry.calm);
        // Never past what the machine booted with (processors) or its ceiling (memory).
        let cpu = cpu.map(|count| count.min(current.cpu.booted.max(1)));
        let memory = memory
            .filter(|_| current.memory.ceiling_mb > current.memory.booted_mb)
            .map(|mb| mb.clamp(current.memory.booted_mb, current.memory.ceiling_mb))
            .filter(|mb| *mb != sample.memory_total_mb);
        (sample, cpu, memory)
    };
    let (sample, cpu, memory) = decision;
    if cpu.is_none() && memory.is_none() {
        return;
    }
    let request = soma_local::ResizeRequest {
        cpu_count: cpu.filter(|count| *count != sample.cpu_limit),
        memory_mb: memory,
    };
    if request.cpu_count.is_none() && request.memory_mb.is_none() {
        return;
    }
    match facade.resize(instance, request) {
        Ok(outcome) => record(instance, &sample, &outcome),
        Err(_) => note(instance, Some("the resize did not apply".to_owned())),
    }
}

fn record(instance: &InstanceId, sample: &Sample, outcome: &soma_local::ResizeOutcome) {
    let at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let mut all = sandboxes();
    let Some(entry) = all.get_mut(instance.as_str()) else {
        return;
    };
    let mut push = |dimension, from: u64, to: u64| {
        if from != to {
            entry.history.push_back(Change {
                at_unix,
                dimension,
                from,
                to,
            });
            while entry.history.len() > HISTORY {
                entry.history.pop_front();
            }
        }
    };
    push(
        "cpu",
        u64::from(sample.cpu_limit),
        u64::from(outcome.cpu.applied),
    );
    push("memory", sample.memory_total_mb, outcome.memory.total_mb);
    entry.problem = outcome
        .memory
        .reason
        .clone()
        .or_else(|| outcome.cpu.reason.clone());
}

fn note(instance: &InstanceId, problem: Option<String>) {
    if let Some(entry) = sandboxes().get_mut(instance.as_str()) {
        entry.problem = problem;
    }
}
