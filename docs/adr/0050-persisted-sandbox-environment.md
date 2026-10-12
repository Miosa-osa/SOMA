# ADR 0050: Keep a sandbox's environment in the sandbox

## Status

Proposed.

## Context

A caller needs to set environment that later commands and terminals see: credentials, identity, a linked database.
Passing it on every command works for one caller and is lost the next time anyone else runs something.
A map in the API process is in memory, so a restart or roll would silently drop the environment of a sandbox that is still alive.

## Decision

1. The environment lives in the guest, in `/etc/miosa/identity.env`, as `NAME='value'` lines.
   It is written through the filesystem API, so no new wire message exists, and it lasts exactly as long as the sandbox does.
2. The guest agent applies it as the base environment of every direct command and every terminal.
   The order, lowest first, is the agent's fixed base, the persisted file, then the pairs the command names.
3. It is read at each command and each terminal open, so a change is seen by the next one with no restart.
4. The agent believes the file only when it is a regular file owned by root that no one else can write, opened without following a link, and no larger than 64 KiB.
   Anything else is ignored whole.
   The file can hold credentials, so the writer should create it mode 0600.
5. Parsing is strict.
   A line that is not `NAME='value'` (or a bare value of safe characters), has an invalid name, holds a NUL, or exceeds the field bound is skipped and never becomes another variable.
   A later line for a name replaces the earlier one.
   At most 256 variables are taken.
6. A value cannot contain a newline, because a line is the unit of the file.
   Whoever writes the file must refuse such a value rather than store it.

## Consequences

- Writers should update the file atomically: write a temporary file and rename it over the target, so an agent reading mid-update sees the old file or the new one.
  That needs a rename operation, which is the guest filesystem addition in the next ADR.
  Until then a writer overwrites in place and a command that starts mid-write may see a partial file, which the strict parser reduces to the complete lines before the cut.
- Shell-run commands also see the values through the profile script the writer keeps beside the file.
- No Generation is recertified: an agent that predates this simply ignores the file.
