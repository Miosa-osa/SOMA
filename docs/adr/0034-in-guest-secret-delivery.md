# ADR 0034: Deliver a secret into an Instance over its authenticated session

- Status: Accepted
- Date: 2026-08-31
- Extends: ADR 0020, ADR 0021, ADR 0024, and ADR 0030

## Context

A sandbox that cannot be given a credential cannot run the workloads this product exists for.
SOMA's Template schema already carries secret references, a delivery mode, a destination, and a file mode whose default is owner-read-only, and `templates/coding-agent.toml` already names a `secret://` source.
Nothing delivered a value to a running Instance.

The research cut of a comparable product records the distinction SOMA adopts.
Some programs genuinely need the credential itself, in their own environment or in a file inside the guest.
Some only need authenticated outbound requests, so a host-side mediator can hold the credential and the guest never sees it.
This decision implements the first of those two modes and nothing of the second.

The hard rule the same research records is that SOMA must never place a reusable secret in a Template, a Template Lock, a Generation, a snapshot, a log, or a receipt.
Three of those are shared by every Instance of a Generation, which is the reason the delivery cannot be part of the image the Instance boots from.

## Decision

### The mechanism is the one the session already has

A secret reaches the guest as bounded filesystem requests over the authenticated session of ADR 0021, and by no other path.
No secret field is added to the launch page, whose fixed schema of ADR 0020 has none and is published to a machine that has authenticated nothing.
No secret is written into a Generation artifact or captured by the snapshot, whose capture point of ADR 0030 is before any launch.

Two operations are added to the bounded filesystem protocol so that a mode can be honoured at all.
`Create` brings a file into existence at exactly one mode and fails when the path already exists.
`SetMode` sets the permission bits of a path that exists.
Both carry a mode bounded to the permission bits, so neither can create a set-user, set-group, or sticky file.
The guest agent sets the mode explicitly on the descriptor it created, because the process umask would otherwise decide what a delivered credential is readable by.

### One delivery is four ordered steps

The destination's directory is made, the destination is created exclusively at owner read and write, the value is written, and the requested mode is applied last.
The exclusive create refuses a path that already exists, so a delivery never writes into a file whose mode, owner, or link target it did not choose.
The mode is applied last because a delivered credential should end without write permission, and applying it first would make the write depend on the guest agent's privilege.

### Delivery is per Instance and fails closed

Placement happens on the sandbox thread between the repaired session and Ready.
Before that moment there is no authenticated session to deliver over; after it, a failure would be a running sandbox that nothing can be told about.
A refused step fails the launch, which finishes the machine and releases its private overlay head, so no sandbox runs without a credential it was launched with and no partly written destination survives.

### No secret is renderable

`SecretValue` has no `Clone`, no `Display`, no comparison, and a `Debug` that reports a length.
Its bytes are readable only by the crate-internal delivery step, and the owned copy is zeroized on drop.
`SecretFile` reports a destination length and a mode, never the destination or the value.
The refusal evidence names the step and the guest's cause, and carries neither.

### What this decision does not do

It does not build a host-side egress proxy or a credential vault.
It does not resolve a `secret://` source into a value, so the Backend has no secret to place yet and the placement runs over an empty set.
It does not deliver a secret into a process environment.

## Consequences

The first delivery mode exists as a mechanism with its own types, and the second mode can be added without changing it.
A future credential source has one place to deliver into and one failure mode to honour.
A Template Lock still binds only the reference, the delivery mode, the destination, and the mode, which is what makes two Instances of one Generation able to hold different values.

## Verification gates

- Unit tests must prove the exact request sequence, the private creation mode, and the owner-read-only default the Template schema names.
- Unit tests must prove that a refused step stops the delivery before the value reaches the wire and that a later secret is not offered after an earlier refusal.
- Tests must prove that a known value appears in no debug rendering of any type that carries it, in no refusal evidence, and in no launch page.
- Guest-agent tests must prove that a created file has exactly the mode requested, that an existing path is refused, and that a mode change is reported honestly.
- A Linux KVM live run must prove a placed secret and a fail-closed launch once a credential source exists.
