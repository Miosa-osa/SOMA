# ADR 0054: Prepared Generations are released by a prune inside soma-api

## Status

Proposed.

## Context

A host keeps every Generation it ever prepared.
On host-11 the certified store (`SOMA_GENERATION_STORE=/srv/soma/certified/native-prepared`) held 13 entries, about 615 GB by `du`, on 2026-10-26 ([SOMA#130](https://github.com/Miosa-osa/SOMA/issues/130)).
An operator cannot remove one by hand.
`preload` admits every entry at start, and an admitted Generation keeps its verified artifacts open, so `soma-api` held 7 descriptors into every entry and 1,031 into one.
Removing an entry from under it frees nothing while those descriptors are open, and a cached Generation whose directory is gone is a launch capability for bytes no lookup can find again.
Entries reflink each other on XFS, so `du` over-counts what a removal frees.

## Decision

1. `POST /v1/prepared/prune` on the loopback listener plans the release, and performs it only when the body says so.
   The body is `{"keep": [reference, ...], "apply": false}`; both fields are optional, and `apply` defaults to false, so an absent body is a dry run.
   The answer is the plan: `keep`, `remove`, and `refused`, each entry with its reference, family, Generation, publish time, reason, the detail the reason refers to, and its unshared and mapped bytes; plus `pins`, `unmatched_keep`, the total `unshared_bytes` of `remove`, and `applied`.
   A dry run is exactly the applied run's plan: every refusal an applied run would make, including a Generation held outside the cache, is made in the dry run too.
2. A reference is pinned by any of: the request's `keep` list, `SOMA_PREWARM_REFERENCE`, a prewarm target set at runtime ([ADR 0052](0052-runtime-prewarm-targets.md)), and the public runner's `launch.image`, which `soma-api` pins when it loads `--runner-config`.
   A keep value of the form `reference@memory_mib` (a prewarm floor key) keeps `reference`; a trailing `@` followed by digits is a memory size, and a digest (`@sha256:...`) is kept whole.
   An applied prune with an empty `keep` list is refused with `keep_required` (422): the host's own pins name only what it prewarms, not what a control plane may launch next.
   A dry run with an empty `keep` list is allowed, because it changes nothing and shows what the host's pins alone protect.
3. Entries are grouped by family: the image and the tag up to the tag's first `-`.
   Versions of one image are cut as a tag suffix (`capture-sized-generation.sh` builds `soma-default:${NAME}-${SUFFIX}`), so `soma-default:G10`, `soma-default:G10-b2`, and `soma-default:G10-sr1` are one family, and `node:22-c35b568` is in the family of `node:22`.
   A reference with no tag, a tag with no `-`, or a digest is its own family.
   Within a family, certified entries are ordered by when `generation.id` was published (its modification time; certification publishes it last), newest first.
   The family keeps every pinned entry and its newest unpinned entry.
   When that newest unpinned entry was certified after the family's newest pinned one (`newer_than_pinned`), the family also keeps its newest unpinned entry certified no later than that pin (`previous_in_family`), so both a rollback target and a freshly captured candidate survive.
   Every older unpinned certified entry is `superseded`.
   Where the rule is ambiguous it keeps more: an uncertified entry (it may still be being certified), an entry that cannot be read, a link, and two entries claiming one reference are all kept.
4. A removal is refused while anything may hold the entry.
   - `has_instances`: a durable record that is not terminal names the entry's Generation.
     The sandbox listing now carries the Generation each record names (`SandboxEntry::generation_id`), read from the launch intent or the launch receipt.
   - `unattributed_instance`: a KVM record names no Generation, or a record cannot be decoded.
     Either could hold any entry, so nothing is removed while one exists.
   - `unattributed_head`: a head is still named under `SOMA_HEAD_DIR` (in a shard or the root) and no live Instance has that name, or the head root cannot be read.
     A head is cloned and unlinked in one step, so a running machine's head has no name and its Instance's record already refuses its Generation; a named head is one whose unlink never ran, it records nothing about its Generation, and it shares extents with an entry nothing can name.
     The fan directory holds template replicas, which are copies rather than heads, and is not read.
   - `in_use`: the cached Generation's artifact handles have a holder other than the cache (a launch in flight, a primed machine, a prewarm plan).
5. An applied removal is crash safe and never half done.
   Under the cache's lock the cached value leaves the cache and the entry is renamed into `<root>/.pruning/<name>.<nanos>`.
   A lookup that read the entry before the rename caches what it read only if the entry is still in the root, checked under the same lock, so a released entry is never re-admitted.
   The holders are then read a second time; a launch that resolved the entry before it left the cache recorded itself before it used it, so it is seen, and the entry is moved back and its value cached again.
   Otherwise the value is dropped, which closes this process's handles, and the entry is deleted only if no descriptor in `/proc/self/fd` still reaches it.
   An entry under `.pruning` is one level too deep for a lookup to read, so a crash after the rename leaves a whole entry out of service; the next applied prune deletes it (`interrupted`) unless a holder names its Generation.
6. Bytes are measured with `FS_IOC_FIEMAP` (`soma_storage::fiemap`).
   An entry's `unshared_bytes` sums the extents of its regular files that carry no `FIEMAP_EXTENT_SHARED` flag; `mapped_bytes` sums them all.
   The total is a lower bound on what removing `remove` frees, because extents shared only among removed entries are freed too and no single entry counts them.
   An entry with any file that cannot be mapped reports no bytes rather than a partial sum.
7. One prune runs at a time; a second is refused with `prune_in_progress` (409).

## Consequences

- Refusals are named: `keep_required` and `invalid_keep` (422), `prune_store_unavailable` (503), `instances_unavailable` (503, retryable), `prune_in_progress` (409, retryable), and `prune_unsupported` (501) on a target with no prepared store.
- The route is served with a facade, because the durable records are what say an entry is in use; a burst that holds every facade answers it `runtime_busy`, like any other route.
- Holders are what this `soma-api`'s durable state records.
  Another process using the same store with a different state root (a `soma` CLI run, for example) is not visible to it, so a prune is for hosts where `soma-api` is the store's only user, which is how production hosts run.
- SOMA has no suspended Instance yet: [ADR 0034](0034-per-sandbox-state-and-suspension.md) designs the Suspension but nothing writes one.
  When it lands, its header binds the Generation it was launched from and must be added to the holders before a prune can run beside it.
- A leaked named head blocks every removal until an operator removes it; it belongs to no live Instance, so removing it is safe.
- A removed reference is gone for this host: a prewarm set naming it is `unknown_reference`, and a launch for it is refused as not prepared.
- An entry put back after an `in_use` descriptor check has no cached value, so the next lookup verifies and admits it again.
- The keep-N gap for host template Generations on the MIOSA side (miosa-compute #3554) is separate and unchanged.
