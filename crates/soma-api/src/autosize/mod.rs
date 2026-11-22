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

use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use soma::InstanceId;

use guest::{cpu_ticks, meminfo_kb, read};

pub use decide::{Policy, Sample};
pub use route::{AutosizeBody, serve};

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
    /// The guest's busy and total CPU ticks at the last reading.
    ticks: Option<(u64, u64)>,
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

/// Turns automatic sizing on for `instance` with `policy`, or off with `None`.
pub fn set(instance: &InstanceId, policy: Option<Policy>) {
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
    let (Some(stat), Some(meminfo)) = (
        read(facade, instance, "/proc/stat"),
        read(facade, instance, "/proc/meminfo"),
    ) else {
        note(
            instance,
            Some("the guest's usage could not be read".to_owned()),
        );
        return;
    };
    let Some(ticks) = cpu_ticks(&stat) else {
        return;
    };
    let (Some(total_kb), Some(available_kb)) = (
        meminfo_kb(&meminfo, "MemTotal:"),
        meminfo_kb(&meminfo, "MemAvailable:"),
    ) else {
        return;
    };
    let decision = {
        let mut all = sandboxes();
        let Some(entry) = all.get_mut(instance.as_str()) else {
            return;
        };
        let previous = entry.ticks.replace(ticks);
        let Some(previous) = previous else {
            return;
        };
        let busy = ticks.0.saturating_sub(previous.0);
        let elapsed = ticks.1.saturating_sub(previous.1).max(1);
        // The guest's ticks count every processor it booted; the share of them that was busy,
        // times how many it booted, is how many it kept busy.
        #[allow(clippy::cast_precision_loss)]
        let cpu_used = busy as f64 / elapsed as f64 * f64::from(current.cpu.booted.max(1));
        let sample = Sample {
            cpu_used,
            cpu_limit: current.cpu.applied,
            memory_used_mb: total_kb.saturating_sub(available_kb) / 1024,
            // What the machine has, as the device states it; the guest's own total leaves out
            // what its kernel reserved, and every change is stated against the device.
            memory_total_mb: if current.memory.total_mb > 0 {
                current.memory.total_mb
            } else {
                total_kb / 1024
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
