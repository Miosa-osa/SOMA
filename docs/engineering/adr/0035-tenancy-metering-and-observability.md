# ADR 0035: Tenancy, quota, metering, and per-sandbox observability

- Status: Accepted
- Date: 2026-08-31
- Extends: ADR 0001, ADR 0004, ADR 0008, ADR 0010, ADR 0012, ADR 0021, and ADR 0031
- Capability status: Designed

## Context

SOMA is being prepared to run inside the MIOSA platform as a sandbox provider, and the
[provider contract gap analysis](../research/provider-contract-gap-analysis.md) records the interface
that requires.
The [capability survey](../research/sandbox-provider-capability-survey.md) records what the other
providers on that list already do.
Neither document names the gap this record exists to close, because it is not a missing operation in
the provider interface.
It is that SOMA has no concept of who is asking.

There is no tenant identity anywhere in the codebase.
There is no authentication on any surface: the development Backend runs inside the process that
called it, and the `soma-hostd` Unix endpoint accepts whatever can open the socket.
There is no quota, so nothing bounds how much of a host one caller may take.
There is no fairness, so nothing prevents one caller from taking all of it.
There is no usage accounting, so nothing can be billed and no consumption can be attributed.
There is per-operation evidence in the execution receipt of [ADR 0008](0008-evidence-carrying-execution-receipts.md),
but there is no per-sandbox observability an operator can use, and no rule about what an operator is
allowed to see.

That absence is not an oversight to be corrected by adding fields.
[ADR 0001](0001-direct-per-machine-interface.md) decided that tenants, billing, plans, placement, pools,
and public sandbox identifiers must remain outside SOMA, and
[ADR 0004](0004-portability-through-conformance.md) repeated the rule for the provider-neutral Machine
contract.
Those decisions are correct and this record does not reverse them.
The question is therefore not whether SOMA should learn about tenants but at exactly which seam it
should, and the answer has to be one seam rather than a concept that diffuses through the stack.

[ADR 0031](0031-persistent-host-runtime-ownership.md) supplies that seam.
One long-running `soma-hostd` process owns every managed Instance for one admitted host, presents one
versioned lifecycle interface, and holds the only durable record of what exists.
Everything below it, the jail, the VMM process, the Machine, and the guest session, already has
exactly one owner per lifecycle phase.
Tenancy belongs at the Host Runtime interface and nowhere else.

The [fleet control plane](../research/fleet-control-plane.md) has already decided the layer above:
bounded cells, a global directory from tenant and region to cell, control-plane ownership of
authentication and quotas, host-authoritative admission, explicit overload rejection rather than
unbounded queueing, bounded-cardinality metrics that carry no tenant or Instance label, and traces
that follow `OperationId` while redacting secrets and output.
This record extends those rules downward into the host, the VMM, and the guest rather than restating
or contradicting them.
The [rollout plan](../operations/miosa-custom-sandbox-rollout.md) gates a controlled MIOSA rollout on
separate host pools, quotas, dashboards, alerts, and a one-action rollback, and none of those five can
be built without the decisions below.

## Decision

### Tenant identity enters at exactly one authenticated seam

SOMA defines one identity type, `TenantId`, and it is an opaque bounded identifier that SOMA never
interprets, parses, or renders to a human.
SOMA does not model users, accounts, organizations, projects, teams, plans, roles, or API keys.
It models one boundary and gives it a name.

`TenantId` enters at the `soma-hostd` lifecycle interface and at no other point.
It is not a field of `LaunchRequest`, it is not part of the Machine contract, it does not appear in a
Template, a Template Lock, a Generation, or a snapshot, and it never reaches `soma-vmm`, the guest
kernel, or the guest agent.
A Generation may be private to a tenant, but that fact is a Host Runtime access rule about which
`GenerationId` a tenant may launch, not a tenant byte inside the artifact.
This keeps the reusable artifacts free of tenant authority, which is already a hard invariant of the
[threat model](../threat-model.md) and of [ADR 0006](0006-prepared-worker-allocation.md).

The tenant is a property of the authenticated connection, never of the request body.
A caller cannot state which tenant it is, because a value a caller supplies is a value a caller can
change.
The local `SOCK_SEQPACKET` adapter authenticates the peer with its kernel-supplied credentials and a
per-adapter control credential held outside the tenant's reach, and the future remote adapter
authenticates the client with a mutually authenticated transport.
In both cases the Host Runtime resolves the verified principal to exactly one `TenantId` through a
mapping the caller cannot influence, and rejects the connection when the mapping is absent or
ambiguous rather than defaulting to a shared or anonymous tenant.
A single MIOSA adapter process may hold many connections, one authenticated principal per tenant it is
acting for, and it may not multiplex two tenants over one connection.
That constraint costs the adapter connections and buys the property that no code path anywhere below
the socket has to decide which tenant a frame belongs to.

Binding is structural rather than checked.
Every durable record the Host Runtime creates, which is the operation record, the Instance owner, the
capacity reservation, the prepared-worker claim, the storage lease, the network bundle, the metering
journal entry, and the receipt, is created by a code path that already holds the connection's
`TenantId`, and every lookup is a lookup inside a tenant-scoped namespace.
There is no interface that accepts an `InstanceId` or an `OperationId` alone and then verifies
ownership afterwards, because a verification that can be written can be forgotten, and the forgotten
one is found by an incident rather than by a test.
`InstanceId` remains unguessable, but unguessability is a second line and not the control.
A cross-tenant read is therefore not a permission failure that the code declines to allow; it is a
lookup in a map that does not contain the key.

Cleanup keeps the same shape.
[ADR 0001](0001-direct-per-machine-interface.md) already requires that cleanup cannot target a
resource that is not owned by the matching launch receipt, and the tenant scope sits above that
requirement rather than replacing it.

### Quota is per host, per tenant, typed, and fails closed

The Host Runtime bounds, for each tenant on the host it owns, the number of concurrently admitted
Instances, the number of launches in flight, the launch admission rate, the reserved memory, the CPU
weight and CPU ceiling, the private overlay bytes that could be written, the terminal cleanup
operations in flight, and, when [ADR 0012](0012-fail-closed-networking.md) networking is actually
wired, the egress bytes and packet rate.
Launch concurrency and launch rate are bounded separately from live Instance count because launch is
where the expensive restore, assignment, and repair work happens, and a tenant that launches and
destroys rapidly consumes far more of a host than its steady-state count suggests.
Cleanup is bounded for the same reason, since a host with all its cleanup capacity consumed by one
tenant's churn cannot return capacity for anyone.

Memory deserves a separate sentence because SOMA's shape rules make it unusual.
The capability survey records that memory must exactly equal the size the Generation's snapshot was
captured with, so a tenant does not request memory at all and a memory quota is arithmetic over
admitted Instances and their Generations' fixed sizes.
That is a weaker instrument than it looks, and it stays weaker until SOMA decides whether a caller may
request a shape, which is an open decision the survey names and this record does not make.
Admission must still reserve against plausible dirty memory rather than against clean shared pages,
which the rollout plan already requires, so the memory quota is a reservation quota and not an
observed-usage quota.

Exhaustion produces an immediate typed rejection.
The Host Runtime does not queue, does not park the request behind a deadline, and does not silently
substitute a smaller shape, a different Generation, or a weaker isolation class.
The rejection distinguishes at least three classes, because the control plane above must act
differently on each.
A tenant quota rejection says that this tenant's own limit is reached and that retrying anywhere in
the fleet will fail identically until the tenant releases something.
A host capacity rejection says that this host is full and that the same request may succeed on another
host, which is the signal the fleet scheduler needs in order to re-place rather than to give up.
An admission rate rejection says that the request was well formed and within quota but arrived faster
than the host will accept, and it carries the interval after which retrying is reasonable.
Collapsing these into one error would make the fleet scheduler either retry uselessly or abandon
requests that were placeable, so the distinction is part of the contract rather than a diagnostic
nicety.

Fairness on a shared host uses two mechanisms that answer two different failure modes.
Enforcement is hierarchical cgroup v2, with one slice per tenant on the host and one child per
Instance, carrying `cpu.weight` and a CPU ceiling, a memory limit, and an I/O weight and ceiling at
the tenant slice.
That bounds what a tenant's admitted Instances can consume in aggregate, so twenty Instances belonging
to one tenant compete with each other inside that tenant's share instead of competing with another
tenant's single Instance.
Admission is the second mechanism, and it caps the fraction of one host's admitted capacity that any
single tenant may hold even when the host is otherwise idle.
Without that cap a tenant that arrives first takes the host, and every later arrival is answered with
a capacity rejection that the fleet scheduler correctly interprets as a full host, which converts a
fairness problem into a phantom capacity shortage.
Reserving headroom costs measurable utilization, and that cost is accepted deliberately.

Two limits of this design are stated rather than hidden.
Cgroups bound CPU time, memory, and block I/O, and they do not bound microarchitectural interference
between tenants sharing a core complex, a cache, or a memory controller, nor do they bound the host
CPU that one guest's exit behaviour imposes on the VMM thread serving it.
Mitigating co-tenancy interference is a placement decision, which means dedicated host pools per trust
class, and placement belongs to the fleet control plane and to MIOSA rather than to this record.
Second, per-host quota is not per-account quota; a tenant spread across ten hosts can hold ten times
its per-host limit, and the aggregate limit belongs above SOMA.

### Metering is a projection of the same evidence the receipt is built from

SOMA counts, for each Instance, the shape class it ran at, the wall time from the milestone at which
tenant code could first execute to the milestone at which the vCPU stopped, the wall time from
admission to proven capacity release, the memory reserved for it, the high-water mark of private
overlay bytes written, the number of commands executed, and, when networking exists, egress and
ingress bytes.
It counts, for each tenant on the host, launches admitted and launches rejected by rejection class,
and it counts Generation storage occupancy separately because that is a build-time and residency cost
rather than an Instance cost.

Two durations are recorded rather than one, and this is deliberate.
The executing window is what the tenant received, and the reserved window from admission to proven
cleanup is what the host actually spent, because capacity is not returned until cleanup is proven and
a failed cleanup costs the host real time that no tenant occupied usefully.
Reporting only the first understates cost and reporting only the second charges tenants for SOMA's own
failures.
SOMA reports both and does not decide which one is priced, because pricing is MIOSA's.

Resolution is one record per Instance at terminal, plus a partial record at a fixed interval for any
Instance that outlives that interval.
Per-second and per-command billing records were rejected because they multiply record volume by orders
of magnitude in order to refine a number that the terminal record already carries to the millisecond.
The periodic partial exists only so that a host crash loses at most one interval of accounting rather
than the whole lifetime of a long-running sandbox, and it is a checkpoint rather than an increment, so
the terminal record supersedes every partial for the same Instance rather than adding to it.

Metering derives from the receipt's evidence rather than from an independent measurement, and this is
the more consequential half of the decision.
Two independent measurements of one lifecycle will eventually disagree, and they will disagree exactly
in the cases that matter, which are crash, timeout, forced destroy, and ambiguous cleanup, because
those are the cases where two code paths make different assumptions about what happened.
The disagreement would then be discovered by a customer dispute rather than by a test.
The usage record is therefore computed from the same internal milestone, shape, preparation class, and
cleanup-state facts that [ADR 0008](0008-evidence-carrying-execution-receipts.md) uses to build the
receipt, and it carries the receipt's operation identity so that a record and a receipt can be
reconciled.

It is nonetheless a separate durable stream rather than the receipt itself, for a reason that is not
about measurement.
A receipt is a terminal response returned to a caller, and a response that is never read is lost,
whereas usage must survive a caller that disconnects, a Host Runtime that restarts, and an Instance
that is destroyed by reconciliation long after its caller has gone.
The metering journal is written on the host before the terminal receipt is returned, and the record's
existence does not depend on anybody receiving anything.
It is also a narrower object than the receipt, carrying only the fields listed above, which lets it be
retained longer and shared with a billing system that has no business holding execution evidence.

A usage record must be tamper-evident, and the reason is specific rather than general.
It is produced on a host that deliberately runs hostile tenant code, whose stated adversaries include
code running as root inside the guest and a compromised guest kernel, and it is the input to a
transfer of money between two parties who may later disagree about it.
A tenant who achieves any host-side foothold should not be able to reduce their own bill by editing or
deleting records, and an operator error that silently drops a range of records should be detectable as
an error rather than absorbed as revenue.
Each record therefore commits to the digest of the previous record, to the host identity, to a journal
epoch that changes on every Host Runtime start, and to a monotonic sequence number within that epoch,
and the control plane retains the last acknowledged chain head per host and epoch.
An edited record breaks the chain, a deleted record leaves a sequence gap, and a shortened journal is
visible as a head that moved backwards.

What that does not give is stated plainly, because a hash chain is routinely oversold.
It gives tamper evidence against after-the-fact modification and against loss, and it gives nothing
against a host that was compromised before it wrote the record, since such a host can construct a
consistent chain over false facts from the beginning.
It is not a signature, it is not attestation, and it does not make the underlying measurement true.
Non-repudiation would need a host signing key with a trust root, a rotation policy, a canonical
encoding, and a verifier, which is the same deferred verifiable profile that
[ADR 0008](0008-evidence-carrying-execution-receipts.md) declined to build early for the same reason.
The defence against a lying host is not cryptographic at this layer: it is the control plane comparing
host-reported usage with the admissions it issued, and that comparison lives in MIOSA.

### Observability shows the operator the shape of a sandbox and none of its content

The fleet control plane already requires bounded-cardinality labels on metrics and forbids tenant and
Instance labels.
That rule extends unchanged into the host and VMM layers, and it gains one addition: `GenerationId`
digests, `OperationId`, command text, filesystem paths, and network addresses are also prohibited as
metric labels, because each of them is unbounded in exactly the way the original rule was written to
prevent.
Host and VMM metrics are therefore labelled by cell, host profile, Generation class, shape class,
preparation class, milestone, result, and failure class, and by nothing else.

The reason that rule does not starve operators of per-tenant numbers is that per-tenant numbers exist
in the metering journal, which is a record stream keyed by identity, held under access control, with a
retention policy.
An operator dashboard that wants a per-tenant time series builds it from that store rather than from
the metric system.
Keeping the two apart means the metric system stays bounded and cheap while the identified data stays
governed, and it prevents the common accident in which a billing question is answered by adding one
label and quietly making every metric per tenant forever.

Traces follow `OperationId` as the fleet document requires, and the span set extends downward to cover
admission, quota evaluation, prepared-worker claim, sterile restore, assignment, network activation,
guest repair, authenticated readiness, command execution, terminal result, and each cleanup step.
Span attributes are typed classes and identities, never values: a span records that a command ran with
a deadline and an output limit and produced an exit status and byte counts, and never the executable,
the arguments, the environment, or a byte of output.
A trace may carry the opaque `TenantId` as an attribute even though a metric may not, and the
distinction is not an inconsistency.
The metric prohibition exists because of cardinality, and a trace is already one object per operation,
so the cardinality argument does not apply to it.
The confidentiality question that remains is answered by access control on the trace store, and by the
rule that a trace exported to a tenant contains no identifier belonging to another tenant and that a
host-level span covering shared work names no tenant at all.

What an operator can see about a running sandbox, without seeing any tenant data, is its `InstanceId`
and opaque tenant reference, its `GenerationId` and Template Lock identity, its shape, preparation
class, isolation class and effective network policy class, its current lifecycle state and its ordered
milestones, its deadline and remaining time, its VMM process and cgroup identity, its resource
consumption in the terms the density standard already requires, which includes CPU time, resident and
proportional memory, dirty pages, page-fault rate, overlay bytes written, descriptors, KVM exits, and
network counters, and its cleanup state including any remaining uncertainty.
That is enough to answer every operational question: whether it is stuck, what it is consuming, whether
it is about to be killed by a deadline, whether it failed and in which class, and whether its resources
came back.

What an operator cannot see is the content: guest stdout and stderr bytes, the executable and its
arguments, environment values, file names or contents in the overlay, guest memory, the launch page,
the readiness challenge, session keys, and anything computed from them.
This is structural rather than policy, because those bytes never leave the VMM process except as
bounded metadata, and the Host Runtime has no facility that reads them.
No such facility is added here, and adding one later would require its own decision record and its own
answer to the question of tenant consent.

Logs follow the same split.
Host logs are operator-facing and carry typed events and identifiers only, and no guest-produced byte
enters them.
Guest console and serial output is tenant data rather than a host diagnostic; it is disabled by
default, and when a tenant enables it the output is delivered to that tenant and not to the host log,
and the receipt records that the capability was enabled so that the tenant can see that it was.

### Which receipt fields may cross a tenant boundary

The receipt is deliberately detailed, and detail is what makes this question sharp rather than
rhetorical.
There are three audiences and they are not ordered by privilege.
The owning tenant sees content but not the host.
The operator sees the host but not content.
Another tenant sees nothing, ever, and SOMA renders no receipt to a party that is not the owner.

The receipt is therefore built as two projections from one internal evidence structure, a tenant
projection and an operator projection, and the operator projection is not the tenant projection with
fields blanked out.
Redaction by blanking at presentation time is one forgotten field away from a leak, and the forgotten
field is a new one added later by somebody who did not read this record.
Constructing each projection from typed evidence means that a new field is absent from both
projections until somebody deliberately places it in one, which is the failure direction we want.

The tenant projection carries the operation, Instance, and Generation identities, the resolved OCI
manifest digest and platform, the effective shape and declared capabilities, the isolation class, the
preparation class, the ordered milestones, the terminal command status, the bounded output metadata
and the output digest, the canonical request fingerprint, the cleanup state per resource class, and
the measurement-boundary metadata.
It does not carry the host identity beyond an opaque reference, the host kernel, microcode, mitigation
or topology detail, process, cgroup, namespace, TAP or device names, any filesystem path, the
prepared-worker pool occupancy, the Generation cache warmth, or any fact about another Instance.
Host detail is withheld because it is reconnaissance for an escape attempt, and pool occupancy and
cache warmth are withheld because they are a channel that reports other tenants' activity on the
shared host.

Preparation class is an acknowledged exception and is reported to the tenant, because without it the
tenant cannot interpret their own latency and cannot tell a cold path from a prepared one, which is a
property of their own launch and is required by the performance standard.
For a Generation private to one tenant it discloses nothing.
For a Generation shared between tenants it is a coarse signal about aggregate fleet activity, and that
residual leak is accepted and named here rather than left to be discovered.

The operator projection carries the identities, classes, milestones, resource counters, failure class,
cleanup state, host detail, and placement facts, and it carries bounded output metadata in the form of
byte counts and exit status, because a failure cannot be explained without knowing whether the command
produced output and how it ended.
It does not carry the output digest.
A digest is a confirmation oracle: an operator who suspects the content can test that suspicion
against it, which is a weaker property than reading the output but is not the absence of a property,
so the digest stays on the tenant side of the line.
The operator projection also carries no command text, no arguments, no environment, and no file names.

The canonical request fingerprint is computed with a per-tenant key.
[ADR 0008](0008-evidence-carrying-execution-receipts.md) introduced the fingerprint so that a caller
can correlate an operation without revealing its command, and an unkeyed fingerprint achieves that
against the receipt reader while quietly creating a cross-tenant correlator, since two tenants running
the same request would produce the same value and anybody holding both receipts would learn it.
Keying it per tenant preserves correlation where it is wanted, inside one tenant, and destroys it
where it was never intended.
Key rotation breaks correlation across the rotation boundary, and that is accepted.

Some material crosses no boundary at all and appears in no projection: session keys, launch page
material, readiness challenges, guest secrets, raw environment values, registry credentials, guest
memory, and overlay contents.
That list is already prohibited by ADR 0008 and by the threat model, and it is repeated here so that
the classification above is complete rather than partial.

There is no break-glass path in version 1.
An operator who needs tenant content in order to diagnose a problem asks the tenant for it.
A standing mechanism that lets an operator read tenant data is a mechanism that can be misused, and
making it safe requires authorization, tenant notification, and an audit trail that is itself
protected, which is more work than the diagnosis it would save at this stage of the project.

## The SOMA and MIOSA boundary

SOMA owns the per-host half.
It authenticates the connecting principal, resolves it to one `TenantId`, scopes every durable record
and every resource to that tenant, enforces per-host quota and per-host fairness, rejects overload with
typed errors, writes the tamper-evident metering journal, builds the two receipt projections, and
emits bounded-cardinality metrics and per-operation traces.

MIOSA owns the platform half.
It authenticates end users, models accounts, organizations, projects, plans, roles, and API keys, and
decides which principal corresponds to which tenant.
It sets and enforces limits that span hosts, cells, and regions, because SOMA can only see one host.
It rates usage records into money, issues invoices, and owns the price list.
It sets retention for usage records and receipts, answers deletion requests, decides who is an
operator, and decides whether a receipt may ever be shown to somebody other than its owner, for
instance inside a support ticket.
It also decides co-tenancy policy, meaning which trust classes share a host, since that is a placement
decision.

The boundary is exactly the authenticated Host Runtime lifecycle interface of
[ADR 0031](0031-persistent-host-runtime-ownership.md).
MIOSA presents an authenticated principal and a request; SOMA returns an admission or a typed
rejection, and later a receipt and a usage record.
The rollout plan already requires that the MIOSA adapter live outside this repository, never import
KVM internals, and never weaken SOMA admission, and this record adds that the adapter also never
supplies a tenant identifier as data and never receives an operator projection on a tenant's behalf.

## Consequences

Tenancy becomes a property of a connection, which means the Host Runtime gains a connection table, a
principal-to-tenant mapping that must be provisioned and revoked, and a rejection path for unmapped
principals.
Revocation must take effect for connections that are already open, so an authenticated connection
carries a revocable lease rather than a permanent grant, and a revoked tenant's live Instances are
destroyed under the normal terminal path rather than orphaned.

Scoped lookups are a small, pervasive change rather than a large local one.
Every map keyed by `InstanceId` or `OperationId` inside the Host Runtime becomes a map reached through
a tenant scope, and the durable ledger of [ADR 0010](0010-durable-managed-lifecycle-state.md) gains a
tenant column that participates in every index.
Reconciliation after a restart must restore the scope along with the record, because a reconciled
Instance whose tenant was lost is exactly the leak this design exists to prevent, and the correct
behaviour for a record whose tenant cannot be resolved is to destroy the Instance rather than to adopt
it.

Hierarchical cgroups add a per-tenant slice with its own creation, limit application, reconciliation
after restart, and removal when the tenant's last Instance ends, and an empty slice that is never
removed is a slow leak of kernel objects that the density accounting must notice.
Reserving per-tenant headroom on every host reduces achievable density by a measurable amount, and
that number has to appear in the density campaign rather than being quietly excluded from it.

Quota counters must be durable and must be reconciled after a restart, and reconciliation can be wrong
in two directions.
Over-counting blocks a tenant who is within their limit, and under-counting oversells the host.
SOMA fails closed, so reconciliation starts from the ledger and treats an unresolved record as
consuming quota until it is proven terminal, which means that a host restarting with uncertain cleanup
temporarily admits less than its true capacity.
This is the same rule the reliability standard already applies to capacity, which is not returned until
cleanup is proven.

Metering adds a durable append on the terminal path and a periodic append for long-lived Instances.
It adds nothing to the warm launch path, because the record for an admitted Instance is opened from the
durable operation record that already has to exist before side effects, so time to first command is
unaffected and the benchmark boundary does not move.
The terminal append sits inside cleanup, where it is bounded work beside work that is already bounded.
The hash chain requires a durable head per epoch and a defined recovery when the head and the last
record disagree, and the safe recovery is a new epoch rather than an attempt to repair the old chain.

Two receipt projections roughly double the fixture and redaction test surface of ADR 0008, and both
projections become part of the stable compatibility surface with the same schema-evolution rules.
The per-tenant fingerprint key is one more key with a lifecycle, and it must live where a guest escape
cannot reach it, which means in the Host Runtime and not in the VMM process.

Nothing here is implemented.
The capability status is Designed, and it stays Designed until the gates below have retained evidence
on a named host profile.

## What this record does not decide

It does not decide any price, rate card, free tier, or billing period, and it does not decide which of
the two recorded durations is charged.
It does not decide account-level or region-level quota, because SOMA sees one host.
It does not decide retention windows for usage records, receipts, or traces, nor the behaviour on a
deletion request, both of which are platform obligations.
It does not decide cross-host or cross-region fairness.

It does not decide whether a caller may request a shape, which the capability survey names as an open
decision and which currently makes the CPU and memory quotas arithmetic over fixed Generation shapes
rather than independent limits.
It does not decide the privacy class of a per-sandbox snapshot, which is the other open decision the
survey names and which would introduce an artifact holding tenant state that every rule above would
have to be re-read against.
It does not decide egress metering behaviour beyond naming the counters, because
[ADR 0012](0012-fail-closed-networking.md) networking is designed and unwired and there is no egress to
count, and it decides nothing about ingress accounting or exposed ports.
It does not decide guest-side per-agent users inside one sandbox.
It does not sign receipts or usage records, which remains the deferred verifiable profile of ADR 0008,
and it does not build a break-glass path into tenant data.

## Verification gates

- Compile-level and architecture checks must prove that no tenant identifier appears in the Machine
  contract, in a Template, in a Template Lock, in a Generation, in a snapshot, or in any type that
  crosses into `soma-vmm` or the guest.
- Tests must prove that a tenant identifier supplied in a request body is rejected rather than
  honoured, and that a connection whose principal maps to no tenant is refused rather than defaulted.
- Tests must prove that every Host Runtime lookup is tenant scoped, by showing that a valid
  `InstanceId` and a valid `OperationId` belonging to one tenant are not found under another, for
  inspect, execute, stop, destroy, receipt retrieval, and reconciliation.
- Tests must prove that revoking a tenant's authorization terminates its open connections and destroys
  its live Instances through the normal terminal path with complete cleanup evidence.
- Linux KVM tests must prove that each quota class rejects at its bound with the correct typed
  rejection, that a tenant quota rejection is distinguishable from a host capacity rejection and from
  an admission rate rejection, and that no exhausted path queues, retries, or substitutes a weaker
  class.
- Concurrency and saturation tests must prove that one tenant saturating CPU, memory, block I/O,
  launch rate, and cleanup rate cannot push another tenant's admitted Instances past their declared
  service objective on the same host, and must record the density cost of the reserved headroom.
- Tests must prove that a usage record and its receipt agree on shape, milestones, preparation class,
  and cleanup state for every terminal case, including timeout, guest failure, VMM crash, caller
  death, forced destroy, and incomplete cleanup.
- Tests must prove that the metering chain detects an edited record, a deleted record, a reordered
  record, and a shortened tail, and must prove that a Host Runtime restart opens a new epoch rather
  than extending a chain whose head is uncertain.
- Crash tests must prove that at most one partial interval of usage is lost when a host dies, and that
  a terminal record supersedes rather than accumulates with the partials for the same Instance.
- Golden fixtures must cover both receipt projections for every case ADR 0008 already enumerates, and
  redaction tests must prove that the operator projection contains no output digest, command text,
  argument, environment value, or file name, and that the tenant projection contains no host path,
  process identifier, device name, pool occupancy, or cache-warmth field.
- Tests must prove that the canonical request fingerprint is stable within a tenant and differs across
  tenants for a structurally identical request.
- Metric tests must prove that no emitted label carries a tenant, Instance, operation, Generation
  digest, path, address, or command value, and trace tests must prove that spans carry typed classes
  and identities only and that an exported tenant trace names no other tenant.
- Host proof on a named HostProfile must show an operator inspecting a running sandbox and obtaining
  every field of the operator projection while no interface returns guest output, guest memory, or
  overlay content.
