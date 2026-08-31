//! The four Instance lifecycle requests, answered by the Host Runtime.
//!
//! Each handler is one translation: a request frame into a Runtime call, and the Runtime's
//! typed answer into one reply frame. No policy lives here, so the same semantics survive a
//! second adapter that does not speak this socket.

use std::sync::Arc;

use crate::{
    FailureCode, InstanceId, LaunchFrame, Launched, Reply, ResourceBroker, Runtime, WorkerLauncher,
    failure_code, instance_failure_code,
};

use super::dispatch::{intent_of, launch_bytes};

/// Launches one Instance and replenishes the pool it came from.
pub(super) fn launch<L: WorkerLauncher, R: ResourceBroker>(
    runtime: &Arc<Runtime<L, R>>,
    frame: &LaunchFrame,
) -> Reply {
    let reply = match runtime.launch(&intent_of(frame)) {
        Ok(Launched::Live(view)) => Reply::Launched {
            worker: view.worker,
            lease_generation: view.lease_generation,
            launch: launch_bytes(view.launch),
        },
        Ok(Launched::Replayed {
            worker,
            lease_generation,
        }) => Reply::Replayed {
            worker,
            lease_generation,
        },
        Err(error) => Reply::Failed(failure_code(instance_failure_code(&error))),
    };
    let _ = runtime.pool().replenish();
    reply
}

/// Reports one live Instance, or the unknown code when the Host owns no such Instance.
pub(super) fn get<L: WorkerLauncher, R: ResourceBroker>(
    runtime: &Arc<Runtime<L, R>>,
    instance: InstanceId,
) -> Reply {
    match runtime.get(instance) {
        Some(view) => runtime.pool().inspect(view.worker).map_or_else(
            || Reply::Failed(failure_code(FailureCode::Unknown)),
            |worker| Reply::Live {
                worker: view.worker,
                lease_generation: view.lease_generation,
                phase: worker.phase.code(),
            },
        ),
        None => Reply::Failed(failure_code(FailureCode::Unknown)),
    }
}

/// Reports one bounded page of live Instances.
pub(super) fn list<L: WorkerLauncher, R: ResourceBroker>(
    runtime: &Arc<Runtime<L, R>>,
    after: Option<InstanceId>,
) -> Reply {
    let page = runtime.list(after);
    Reply::Listed {
        instances: page.instances,
        more: page.more,
    }
}

/// Destroys one Instance; a repeat of the same request receives the same reply.
pub(super) fn destroy<L: WorkerLauncher, R: ResourceBroker>(
    runtime: &Arc<Runtime<L, R>>,
    instance: InstanceId,
) -> Reply {
    let reply = match runtime.destroy(instance) {
        Ok(receipt) => Reply::Destroyed {
            worker: receipt.worker,
            complete: receipt.complete,
        },
        Err(error) => Reply::Failed(failure_code(instance_failure_code(&error))),
    };
    let _ = runtime.pool().replenish();
    reply
}
