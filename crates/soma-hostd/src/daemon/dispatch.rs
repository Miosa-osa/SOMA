//! One request frame applied to the Host Runtime it addresses.
//!
//! Worker requests reach the pool the Runtime allocates from; Instance requests reach the
//! Runtime itself, which is the only thing in the process that owns a Machine.

use std::{sync::Arc, time::Duration};

use soma_guest::LaunchNetwork;

use crate::{
    AssignmentIntent, Claim, ClaimOutcome, FailureCode, LaunchFrame, Pool, Reply, Request,
    ResourceBroker, Runtime, WorkerLauncher, claim_failure_code, failure_code,
    lifecycle_failure_code, transfer_failure_code,
};

/// Applies one request to the Host Runtime.
pub fn handle<L: WorkerLauncher, R: ResourceBroker>(
    runtime: &Arc<Runtime<L, R>>,
    request: Request,
) -> Reply {
    let pool = runtime.pool();
    match request {
        Request::Claim(frame) => {
            let reply = claimed(pool, &intent_of(&frame));
            let _ = pool.replenish();
            reply
        }
        Request::Release { worker } => match pool.release(worker) {
            Ok(evidence) => {
                let _ = pool.replenish();
                Reply::Released {
                    complete: evidence.destroyed.complete && evidence.released.complete,
                }
            }
            Err(error) => Reply::Failed(failure_code(lifecycle_failure_code(&error))),
        },
        Request::Inspect { worker } => match pool.inspect(worker) {
            Some(view) => Reply::Inspected {
                phase: view.phase.code(),
                lease_generation: view.lease_generation,
            },
            None => Reply::Failed(failure_code(FailureCode::Unknown)),
        },
        Request::Reconcile => match pool.reconcile() {
            Ok(report) => {
                let (terminated, released, retained) = report.counts();
                let _ = pool.replenish();
                Reply::Reconciled {
                    suspects: report.suspects as u32,
                    terminated: terminated as u32,
                    released: released as u32,
                    retained: retained as u32,
                }
            }
            Err(_) => Reply::Failed(failure_code(FailureCode::Ledger)),
        },
        Request::Launch(frame) => super::lifecycle::launch(runtime, &frame),
        Request::Get { instance } => super::lifecycle::get(runtime, instance),
        Request::List { after } => super::lifecycle::list(runtime, after),
        Request::Destroy { instance } => super::lifecycle::destroy(runtime, instance),
    }
}

/// Rebuilds the assignment intent one frame declares.
pub(super) fn intent_of(frame: &LaunchFrame) -> AssignmentIntent {
    AssignmentIntent {
        instance: frame.instance,
        operation: frame.operation,
        vsock_cid: frame.vsock_cid,
        network: frame.intent.clone(),
        deadline: Duration::from_nanos(frame.deadline_nanos),
        launch_material: frame.launch_material,
    }
}

/// Claims and transfers one worker without creating an Instance the Runtime owns.
fn claimed<L: WorkerLauncher, R: ResourceBroker>(
    pool: &Arc<Pool<L, R>>,
    intent: &AssignmentIntent,
) -> Reply {
    match pool.claim(intent.operation, intent.fingerprint()) {
        Ok(Claim {
            outcome,
            grant: None,
        }) => replayed(pool, outcome),
        Ok(Claim {
            grant: Some(grant), ..
        }) => match pool.transfer(grant, intent) {
            Ok(evidence) => Reply::Claimed {
                worker: evidence.worker,
                lease_generation: evidence.lease_generation,
                launch: launch_bytes(evidence.launch),
            },
            Err(failure) => Reply::Failed(failure_code(transfer_failure_code(&failure))),
        },
        Err(error) => Reply::Failed(failure_code(claim_failure_code(&error))),
    }
}

/// Answers a replay from the disposition of the worker the operation is bound to.
///
/// A replay whose worker was destroyed, by a failed transfer or by a release, is answered
/// with the typed terminal failure rather than a reply naming a worker that is gone, and a
/// replay of a transfer this process performed repeats the launch-page values of the reply
/// that was lost.
/// Only a worker retained across a restart, whose delivery this process never saw, is
/// answered with the reduced replay reply.
fn replayed<L: WorkerLauncher, R: ResourceBroker>(
    pool: &Arc<Pool<L, R>>,
    outcome: ClaimOutcome,
) -> Reply {
    match pool.inspect(outcome.worker) {
        Some(view) if view.phase.is_terminal() => {
            Reply::Failed(failure_code(FailureCode::Terminated))
        }
        Some(view) => view.launch.map_or(
            Reply::Replayed {
                worker: outcome.worker,
                lease_generation: outcome.lease_generation,
            },
            |launch| Reply::Claimed {
                worker: outcome.worker,
                lease_generation: outcome.lease_generation,
                launch: launch_bytes(launch),
            },
        ),
        None => Reply::Failed(failure_code(FailureCode::Terminated)),
    }
}

/// Encodes the launch-page network values in launch-page order.
#[must_use]
pub fn launch_bytes(launch: LaunchNetwork) -> [u8; 35] {
    let mut out = [0; 35];
    out[..4].copy_from_slice(&launch.vsock_cid().to_be_bytes());
    out[4..8].copy_from_slice(&launch.generation().to_be_bytes());
    out[8..14].copy_from_slice(&launch.mac());
    out[14..18].copy_from_slice(&launch.address());
    out[18] = launch.prefix_length();
    out[19..23].copy_from_slice(&launch.gateway());
    out[23..27].copy_from_slice(&launch.resolver());
    out[27..35].copy_from_slice(&launch.time_sample_nanos().to_be_bytes());
    out
}
