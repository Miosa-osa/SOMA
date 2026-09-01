//! Child-side admission of either an on-demand launch or a prepared machine assignment.

use std::{
    io::{self, BufReader},
    os::fd::AsFd as _,
    path::Path,
};

use soma::BackendFailureKind;

use super::{
    channel,
    serve::{refuse, serve_machine},
    wire::{InitialWire, LaunchWire, PrewarmReady},
};
use crate::backend::kvm::{KvmBackend, prepared};

const REFUSED: i32 = 1;

/// Waits for one launch capability, serves its machine until release, and returns process status.
pub(crate) fn host_machine(expected_socket: Option<&Path>) -> i32 {
    let input = io::stdin();
    let Ok(descriptors) = soma_supervise::receive_descriptors(input.as_fd()) else {
        return REFUSED;
    };
    let mut input = BufReader::new(input);
    let Some(initial) = channel::read_line::<InitialWire>(&mut input) else {
        return REFUSED;
    };
    match initial {
        InitialWire::Launch(request) => serve_on_demand(expected_socket, &request, descriptors),
        InitialWire::Prewarm(plan) => serve_primed(expected_socket, plan, descriptors, &mut input),
    }
}

fn serve_on_demand(
    expected_socket: Option<&Path>,
    request: &LaunchWire,
    descriptors: Vec<std::os::fd::OwnedFd>,
) -> i32 {
    if expected_socket.is_some_and(|expected| expected != request.socket) {
        return REFUSED;
    }
    let socket = request.socket.clone();
    let Ok(listener) = channel::bind(&socket) else {
        return REFUSED;
    };
    let Ok(prepared) = prepared::from_handoff(
        request.reference.clone(),
        &request.generation_id,
        &request.manifest,
        descriptors,
    ) else {
        return REFUSED;
    };
    let Ok(backend) = KvmBackend::machine_host() else {
        return refuse(BackendFailureKind::Unavailable);
    };
    let status = serve_machine(&listener, &socket, request, &prepared, backend);
    let _ignored = std::fs::remove_file(socket);
    status
}

fn serve_primed(
    expected_socket: Option<&Path>,
    plan: super::wire::PrewarmWire,
    descriptors: Vec<std::os::fd::OwnedFd>,
    input: &mut BufReader<io::Stdin>,
) -> i32 {
    if expected_socket.is_some() {
        return REFUSED;
    }
    let Ok(prepared) = prepared::from_handoff(
        plan.reference,
        &plan.generation_id,
        &plan.manifest,
        descriptors,
    ) else {
        return REFUSED;
    };
    let Ok(backend) = KvmBackend::primed_machine_host(&prepared, plan.memory_mib) else {
        let _ignored = channel::write_line(&mut io::stdout(), &PrewarmReady::Refused);
        return REFUSED;
    };
    if channel::write_line(&mut io::stdout(), &PrewarmReady::Prepared).is_err() {
        return REFUSED;
    }
    let Some(request) = channel::read_line::<LaunchWire>(input) else {
        return REFUSED;
    };
    if request.reference != prepared.reference
        || request.generation_id != prepared.id
        || request.shape.memory_mib() != plan.memory_mib
    {
        return refuse(BackendFailureKind::WorkloadRejected);
    }
    let socket = request.socket.clone();
    let Ok(listener) = channel::bind(&socket) else {
        return REFUSED;
    };
    let status = serve_machine(&listener, &socket, &request, &prepared, backend);
    let _ignored = std::fs::remove_file(socket);
    status
}
