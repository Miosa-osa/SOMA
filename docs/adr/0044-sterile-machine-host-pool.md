# ADR 0044: Prepare sterile machine-host processes outside Launch

## Status

Accepted.

## Context

Every hosted sandbox belongs to one dedicated machine-host process.
Creating that process inside a 100-way Launch burst adds scheduler and executable-startup contention before the VMM can restore a machine.
The isolation boundary requires one process per machine, but it does not require creating that empty process after a request arrives.

## Decision

The hosted API creates a bounded sterile machine-host pool before it accepts traffic.
A sterile host owns no VM, guest memory, Instance identity, Generation descriptor, network resource, or customer state.
It blocks on one private Unix socket until a Launch claims it and transfers the exact admitted Generation handles and launch request.
Only after that transfer does the child bind the Instance socket, construct the KVM backend, restore the machine, repair identity, and report Ready.

Claim removes one process from the pool atomically.
Depletion falls back to an ordinary synchronous process creation rather than queueing a Launch indefinitely.
One refill worker waits 250 milliseconds before restoring the target, keeping refill process creation outside the measured create-through-first-command window.
The target equals the API worker bound, so both overload controls have one operator-visible capacity input.

## Consequences

Process creation is preparation work, while VM creation remains on demand and is still reported as `on_demand`.
The optimization does not constitute a warm VM pool and does not preassign memory or customer identity.
Every claimed process still owns exactly one sandbox for its complete lifetime and exits after cleanup.
API startup fails closed if it cannot prepare the configured sterile process set.
Qualification must verify that the pool returns to its target, leaves no zombie children, and leaks no Instance sockets after a burst.
