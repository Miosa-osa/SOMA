# ADR 0057: Keep a paused machine resident

## Status

Proposed.

## Decision

The Linux KVM engine can pause one already-running machine in place.
The machine retains its vCPU handles, memory, devices, and instance identity.
The owning runtime acknowledges the pause only after the device worker joins and every vCPU returns its KVM descriptor outside `KVM_RUN`.
Replaying a completed pause succeeds without releasing or relaunching the machine.
A failed pause stays reclaimable and cannot be relabeled successful by a retry.
A request made before launch or after teardown leaves the machine state intact.

The sandbox worker answers a pause request only after the engine acknowledges it.
The worker refuses guest commands while paused and releases a paused guest directly when the controlling session closes.
An uncertain pause ends the session and reclaims the machine.

The jailed worker's private control socket accepts a pause for its exact Instance identity.
It answers with the verified identity and keeps the same Machine in its paused state.
The Machine rejects new guest operations, continues to serve retained receipts, and can stop the paused Instance.
Customer-facing pause requires the managed lifecycle to keep the same instance addressable for resume or destroy.
Live acceptance requires a real Linux KVM host and an executing guest whose progress stops after a successful pause.

## Resident resume

A complete pause keeps the same KVM vCPU descriptors, port bus, MMIO dispatchers, and device eventfds.
Resume starts a new set of service threads over those retained resources and resumes the vCPUs without a guest boot or launch-page repair.
The retained queue eventfds preserve notifications posted between the device thread joining and the last vCPU leaving KVM_RUN.
A resume against an already-running machine succeeds without creating additional workers.
An incomplete pause is reclaimable and cannot be resumed.
A startup failure must stop all partially restarted workers before returning an error.
The sandbox worker resumes over the existing authenticated guest session.
An uncertain worker response ends the session, so the caller cannot mistake a late response for the result of a different request.
The device thread's epoll instance and stop descriptor are allocated at launch and reused across resident pause and resume.
This avoids allocating new event descriptors after the jailed worker enters steady state.
The launch-time watchdog handler remains registered across pause and resume.
A resumed vCPU reuses the KVM descriptor's existing signal mask.
Multiple machines in one process share the same watchdog signal handler, which is restored after the last ordinary machine releases it.
The dedicated jailed process will retain that handler until process exit after its startup filter is sealed.
The private VMM socket accepts a resume request only for the owned Instance and returns a receipt naming that Instance.
The worker's startup filter accepts both canonical C ABI encodings of an allowlisted KVM ioctl request.
A sign-extended request is accepted only if its low 32 bits name an already allowed request with the sign bit set.
The steady-state ioctl list stays the same.
