# ADR 0059: Carry environment variables on loopback commands

## Status

Accepted for implementation.

## Context

The runner's authenticated command API can already pass environment variables to a guest.
The host-local command route accepts standard input but lacks the environment field.
A caller using the host-local route needs the same bounded execution context for scheduled jobs.

## Decision

Add the optional `env` string map to the loopback command request at API version 1.5.
Sort names and use the existing bounded `CommandContext` for both environment and standard input.
Reject invalid context before requesting guest execution.
Keep the command context out of request diagnostics and use the existing context fingerprint and guest transport.
The variables are scoped to that execution.

## Consequences

No guest protocol or checkpoint format changes.
A host adapter can require API 1.5 when sending variables.
The host adapter owns encryption of secrets outside the loopback boundary.
