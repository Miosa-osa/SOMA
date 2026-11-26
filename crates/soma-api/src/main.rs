use std::{env, net::TcpListener, process, sync::Arc, time::Duration};

use soma_api::{
    ApiError, CREATE_ADMISSION_ENV, CreateAdmission, FacadePool, LocalFacade, SandboxFacade,
    retention,
    runner::{self, RunnerConfig},
    serve_with_tunnels,
    tunnel::{TunnelPolicy, TunnelTable},
};
use soma_local::{BackendSelection, LocalRuntimeConfig};

#[path = "main/options.rs"]
mod options;

use options::Options;

const POOL_WAIT_TIMEOUT: Duration = Duration::from_secs(1);

/// The argument the runtime re-enters this executable with to hold one machine.
///
/// It is the same word the command line uses, because the runtime names it and both binaries
/// must answer to whatever the runtime names.
const MACHINE_HOST: &str = "machine-host";

fn main() {
    if soma_version::version_requested(env::args_os()) {
        print!(
            "{}",
            soma_api::version::version_report(&soma_version::build!())
        );
        return;
    }
    process::exit(run());
}

/// Starts the public runner on facades leased from the request pool, or answers the exit status.
///
/// A runner whose machines would need a network this host cannot give is refused first, by the
/// key that asked for it, so the configuration fails here rather than every create (SOMA#56).
fn start_runner(config: RunnerConfig, pool: &FacadePool<LocalFacade>) -> Result<(), i32> {
    let Some(lease) = pool.acquire_timeout(POOL_WAIT_TIMEOUT) else {
        eprintln!("soma-api: the local sandbox runtime could not be opened");
        return Err(69);
    };
    if let Some(key) = config
        .launch
        .unservable_network(|policy| lease.serves_network(policy))
    {
        eprintln!(
            "soma-api: {key} gives the runner's machines a network this host cannot serve; set \
             it to false, or give this host an egress broker"
        );
        return Err(78);
    }
    drop(lease);
    let runner_pool = pool.clone();
    let opener: runner::FacadeOpener = Arc::new(move || {
        runner_pool
            .acquire_timeout(POOL_WAIT_TIMEOUT)
            .map(|lease| Box::new(lease) as Box<dyn SandboxFacade>)
            .ok_or_else(runtime_busy)
    });
    runner::spawn(config, opener).map(drop).map_err(|error| {
        eprintln!("soma-api: the public runner could not start: {error}");
        74
    })
}

fn run() -> i32 {
    // A machine host is not a request this service serves: it becomes the process that holds one
    // sandbox. The runtime starts one by re-entering this executable, so every binary that opens
    // a hosted runtime has to answer here, before any listener exists. Without it this service
    // can create no sandbox at all: the host it spawns would be a copy of itself parsing
    // `machine-host` as an option it does not have.
    let mut arguments = env::args().skip(1);
    if let Some(first) = arguments.next()
        && first == MACHINE_HOST
    {
        if arguments.next().is_some() {
            eprintln!("soma-api: usage: soma-api {MACHINE_HOST}");
            return 64;
        }
        return soma_local::host_machine(None);
    }
    let Some(options) = Options::parse(env::args().skip(1)) else {
        eprintln!(
            "soma-api: usage: soma-api [--listen ADDR] [--backend auto|kvm|macos|docker] \
             [--runtime PATH] [--state-root PATH] [--workers COUNT] [--runner-config PATH]"
        );
        return 64;
    };
    let Some(admission) =
        CreateAdmission::from_setting(env::var(CREATE_ADMISSION_ENV).ok().as_deref())
    else {
        eprintln!("soma-api: {CREATE_ADMISSION_ENV} must be a positive integer");
        return 78;
    };
    // The public runner is configured before anything is opened, so a broken configuration
    // stops the service before it holds a single facade or sterile host.
    let runner_config = match options.runner_config.as_deref().map(RunnerConfig::load) {
        None => None,
        Some(Ok(config)) => Some(config),
        Some(Err(error)) => {
            eprintln!("soma-api: {error}");
            return 78;
        }
    };
    // The runner launches this one image, so a prune never removes the entry it launches from.
    if let Some(config) = &runner_config {
        soma_local::pin_launch_reference(&config.launch.image);
    }
    let tunnels = match TunnelPolicy::from_env() {
        Ok(policy) => TunnelTable::new(policy),
        Err(variable) => {
            eprintln!("soma-api: {variable} is not usable");
            return 78;
        }
    };
    let Ok(listener) = TcpListener::bind(&options.listen) else {
        eprintln!("soma-api: could not bind {}", options.listen);
        return 74;
    };
    let Ok(config) = LocalRuntimeConfig::discover(
        options.backend,
        options.runtime.clone(),
        options.state_root.clone(),
    ) else {
        eprintln!("soma-api: the local sandbox runtime configuration is invalid");
        return 78;
    };
    let config = config.with_hosted_machines(true);
    if matches!(
        options.backend,
        BackendSelection::Auto | BackendSelection::Kvm
    ) && soma_local::start_prewarm(config.state_root(), options.workers).is_err()
    {
        eprintln!("soma-api: sterile machine hosts could not be prepared");
        return 69;
    }
    let Ok(pool) = FacadePool::open(options.workers, || LocalFacade::open(config.clone())) else {
        eprintln!("soma-api: the local sandbox runtime could not be opened");
        return 69;
    };
    if let Some(config) = runner_config
        && let Err(code) = start_runner(config, &pool)
    {
        return code;
    }
    soma_api::autosize::adopt_legacy(config.state_root());
    let restored =
        soma_api::autosize::remember_in(config.state_root().join(soma_api::autosize::SAVED_FILE));
    if restored > 0 {
        eprintln!("soma-api: automatic sizing restored for {restored} sandbox(es)");
    }
    start_autosize(pool.clone());
    if !start_record_sweep(pool.clone()) {
        return 71;
    }
    report_ownerless(&pool);
    eprintln!(
        "soma-api: listening on {} (at most {} concurrent creates)",
        options.listen,
        admission.limit()
    );
    let open_facade = move || {
        pool.acquire_timeout(POOL_WAIT_TIMEOUT)
            .ok_or_else(runtime_busy)
    };
    let build = soma_version::build!();
    if serve_with_tunnels(&listener, open_facade, &admission, &tunnels, build).is_err() {
        eprintln!("soma-api: the listener stopped accepting connections");
        return 74;
    }
    0
}

/// Starts the automatic sizing controller on facades leased from the request pool.
/// Names every sandbox a tenant cannot reach because no owner record names it.
fn report_ownerless(pool: &FacadePool<LocalFacade>) {
    let Some(mut facade) = pool.acquire_timeout(POOL_WAIT_TIMEOUT) else {
        eprintln!("soma-api: WARNING: sandbox owners could not be checked at start");
        return;
    };
    match soma_api::handler::ownerless(&mut *facade) {
        Ok(ownerless) => {
            for id in &ownerless {
                eprintln!(
                    "soma-api: WARNING: sandbox {} has no owner record, so every tenant reads it \
                     as absent until the platform records its owner",
                    id.as_str()
                );
            }
        }
        Err(_) => eprintln!("soma-api: WARNING: sandbox owners could not be checked at start"),
    }
}

fn start_autosize(pool: FacadePool<LocalFacade>) {
    let started = soma_api::autosize::spawn(move || {
        pool.acquire_timeout(POOL_WAIT_TIMEOUT)
            .map(|lease| Box::new(lease) as Box<dyn SandboxFacade>)
    });
    if started.is_err() {
        eprintln!("soma-api: automatic sizing could not start");
    }
}

/// Starts the sweep that releases terminal records (SOMA#127).
///
/// Records are released on the same rule whichever surface created them, so the sweep runs with
/// or without the runner, on facades leased from the request pool.
fn start_record_sweep(pool: FacadePool<LocalFacade>) -> bool {
    let started = retention::spawn(move || {
        pool.acquire_timeout(POOL_WAIT_TIMEOUT)
            .map(|lease| Box::new(lease) as Box<dyn SandboxFacade>)
    });
    if started.is_err() {
        eprintln!("soma-api: the terminal record sweep could not start");
    }
    started.is_ok()
}

const fn runtime_busy() -> ApiError {
    ApiError::new(
        503,
        "runtime_busy",
        "the local sandbox runtime is at its bounded request capacity",
        true,
    )
}
