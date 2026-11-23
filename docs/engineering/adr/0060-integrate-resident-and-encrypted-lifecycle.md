# ADR 0060: Integrate resident and encrypted lifecycle contracts

## Context

The resident lifecycle branch added loopback API versions 1.1 through 1.7.
The upstream branch independently added encrypted suspension, capacity reporting, and autosizing and numbered those revisions 1.1 through 1.3.
Both branches share the original 1.0 contract.

## Decision

The integrated API advertises version 1.8 and keeps both route families.
The resident routes retain their existing request bodies and ownership checks.
Encrypted suspension retains the process state and distinct suspension identity defined by ADR 0034.
The guest accepts a resident reconnect only with fresh launch authority for the same instance or a permitted fork.
An authenticated encrypted suspension still quiesces the guest before capture and opens a fresh control session on resume.
Each restore path retains its own host capability requirements.
Each development branch also assigned a different guest capability to level 4.
The integrated guest advertises level 5, which the host requires before encrypted suspension or connected checkpoint recovery.
Earlier guests retain their shared execution and file protocol levels.
The desktop starts Voxl through the existing viewer lifecycle and uses the same authorized tunnel on guest port 6080.

## Verification

The API surface test records the integrated routes and request fields under a single digest.
Live Linux acceptance must exercise both lifecycle families, capacity reporting, autosizing, and the desktop image before the integrated build is qualified.
