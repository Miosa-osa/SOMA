//! `soma-hostd`: serve claim, release, inspect, and reconcile for one prepared-worker pool
//! over one Unix `SOCK_SEQPACKET` socket.
//!
//! The jail launcher from decision-map ticket #9 is not on this branch yet, so the daemon
//! only starts with the explicitly requested in-process development launcher and refuses
//! every other launcher with a typed message.
//! On a non-Linux host it exits with a typed message and no side effect.

use std::process::ExitCode;

#[cfg(target_os = "linux")]
mod linux {
    use std::{path::PathBuf, sync::Arc, time::Duration};

    use soma_hostd::{
        Admission, CpuClass, ExhaustedBehavior, GenerationId, HostProfile, Limits, MemoryClass,
        MemoryShape, OverlayIdentity, Pool, PoolAdmission, PoolKey, SingleNode, WorkloadClass,
        daemon,
        testing::{InProcessBroker, InProcessLauncher, ProcessTable},
    };

    mod host;

    use host::{Host, profile, shape};
    use soma_netd::ProfileDigest;
    use soma_storage::{ClassName, TemplateDigest};

    struct Config {
        socket: PathBuf,
        ledger: PathBuf,
        launcher: Option<String>,
        generation: Option<[u8; 32]>,
        network_profile: Option<[u8; 32]>,
        template: Option<[u8; 32]>,
        class: String,
        version: u32,
        logical_bytes: u64,
        vcpus: u32,
        memory: u64,
        target: usize,
        max: usize,
        concurrency: usize,
        host: Host,
    }

    fn parse() -> Result<Config, String> {
        let mut config = Config {
            socket: PathBuf::from("/run/soma-hostd/allocator.sock"),
            ledger: PathBuf::from("/run/soma-hostd/ledger"),
            launcher: None,
            generation: None,
            network_profile: None,
            template: None,
            class: String::from("default"),
            version: 1,
            logical_bytes: 4 << 30,
            vcpus: 1,
            memory: 512 << 20,
            target: 4,
            max: 8,
            concurrency: 2,
            host: Host {
                threads: None,
                reserved_threads: 1,
                memory: None,
                reserved_memory: 0,
                overhead: None,
                storage: None,
                network_units: 4096,
                processes: 4096,
                descriptors: 1 << 20,
                residents: u32::MAX,
                launches: u32::MAX,
                runnable_vcpus: u32::MAX,
                dirty_memory: u64::MAX / 2,
                cleanup_slots: 64,
            },
        };
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            let mut value = || args.next().ok_or(format!("{flag} needs a value"));
            match flag.as_str() {
                "--socket" => config.socket = PathBuf::from(value()?),
                "--ledger" => config.ledger = PathBuf::from(value()?),
                "--launcher" => config.launcher = Some(value()?),
                "--generation" => config.generation = Some(hex32(&value()?, &flag)?),
                "--host-threads" => config.host.threads = Some(number(&value()?, &flag)?),
                "--host-reserved-threads" => {
                    config.host.reserved_threads = number(&value()?, &flag)?;
                }
                "--host-memory-bytes" => config.host.memory = Some(number(&value()?, &flag)?),
                "--host-reserved-memory-bytes" => {
                    config.host.reserved_memory = number(&value()?, &flag)?;
                }
                "--host-overhead-bytes" => config.host.overhead = Some(number(&value()?, &flag)?),
                "--host-storage-bytes" => config.host.storage = Some(number(&value()?, &flag)?),
                "--host-network-units" => config.host.network_units = number(&value()?, &flag)?,
                "--host-processes" => config.host.processes = number(&value()?, &flag)?,
                "--host-descriptors" => config.host.descriptors = number(&value()?, &flag)?,
                "--resident-instances" => config.host.residents = number(&value()?, &flag)?,
                "--concurrent-launches" => config.host.launches = number(&value()?, &flag)?,
                "--runnable-vcpus" => config.host.runnable_vcpus = number(&value()?, &flag)?,
                "--dirty-memory-bytes" => config.host.dirty_memory = number(&value()?, &flag)?,
                "--cleanup-slots" => config.host.cleanup_slots = number(&value()?, &flag)?,
                "--network-profile" => config.network_profile = Some(hex32(&value()?, &flag)?),
                "--overlay-template" => config.template = Some(hex32(&value()?, &flag)?),
                "--overlay-class" => config.class = value()?,
                "--overlay-version" => config.version = number(&value()?, &flag)?,
                "--overlay-bytes" => config.logical_bytes = number(&value()?, &flag)?,
                "--vcpus" => config.vcpus = number(&value()?, &flag)?,
                "--memory" => config.memory = number(&value()?, &flag)?,
                "--target" => config.target = number(&value()?, &flag)?,
                "--max" => config.max = number(&value()?, &flag)?,
                "--concurrency" => config.concurrency = number(&value()?, &flag)?,
                _ => return Err(format!("unknown argument {flag}")),
            }
        }
        Ok(config)
    }

    fn key(config: &Config, profile: &HostProfile) -> Result<PoolKey, String> {
        Ok(PoolKey {
            host_profile: profile.digest(),
            generation: GenerationId::new(config.generation.ok_or("--generation is required")?)
                .map_err(|error| error.to_string())?,
            cpu: CpuClass {
                vcpus: config.vcpus,
                workload: WorkloadClass::ApiWaiting,
            },
            memory: MemoryShape {
                guest_bytes: config.memory,
                class: MemoryClass::Guaranteed,
            },
            overlay: OverlayIdentity {
                name: ClassName::new(config.class.clone()).map_err(|error| error.to_string())?,
                version: config.version,
                logical_bytes: config.logical_bytes,
                template: TemplateDigest::from_bytes(
                    config.template.ok_or("--overlay-template is required")?,
                ),
            },
            network: ProfileDigest(
                config
                    .network_profile
                    .ok_or("--network-profile is required")?,
            ),
        })
    }

    pub fn run() -> Result<(), String> {
        let config = parse()?;
        match config.launcher.as_deref() {
            Some("in-process") => {}
            Some(other) => {
                return Err(format!(
                    "launcher {other} is not available: the jail adapter is pending decision-map #9"
                ));
            }
            None => {
                return Err("--launcher in-process is required; no jail adapter exists yet".into());
            }
        }
        let profile = profile(&config.host)?;
        let shape = shape(&config)?;
        let key = key(&config, &profile)?;
        let capacity = PoolAdmission::new(Arc::new(Admission::new(profile, SingleNode)), shape);
        let limits = Limits {
            min: config.target.min(1),
            target: config.target,
            max: config.max,
            replenish_concurrency: config.concurrency,
            claim_deadline: Duration::from_millis(500),
            construction_deadline: Duration::from_secs(5),
            exhausted: ExhaustedBehavior::Reject,
            binding_limit: config.max.saturating_mul(64).max(config.max),
        };
        let pool = Arc::new(
            Pool::open(
                key,
                limits,
                InProcessLauncher::new(ProcessTable::new()),
                InProcessBroker::new(),
                capacity,
                &config.ledger,
            )
            .map_err(|error| error.to_string())?,
        );
        let report = pool.reconcile().map_err(|error| error.to_string())?;
        eprintln!("soma-hostd: reconciled {report}");
        let built = pool
            .replenish_blocking()
            .map_err(|error| error.to_string())?;
        eprintln!("soma-hostd: prepared {built} workers with the in-process development launcher");
        daemon::serve(&pool, &config.socket).map_err(|error| error.to_string())
    }

    fn hex32(text: &str, flag: &str) -> Result<[u8; 32], String> {
        if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("{flag} must be 64 hexadecimal characters"));
        }
        let mut out = [0; 32];
        for (index, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
                .map_err(|_| format!("{flag} must be hexadecimal"))?;
        }
        Ok(out)
    }

    fn number<T: std::str::FromStr>(text: &str, flag: &str) -> Result<T, String> {
        text.parse().map_err(|_| format!("{flag} must be a number"))
    }
}

#[cfg(target_os = "linux")]
use linux::run;

#[cfg(not(target_os = "linux"))]
fn run() -> Result<(), String> {
    Err("soma-hostd requires a Linux host".to_owned())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("soma-hostd: {message}");
            ExitCode::FAILURE
        }
    }
}
