# The evidence corpus, and what is checked about it

Each file here records one run: what was measured, on what, and what the run does not say.
`raw/` holds the samples a record was computed from, one directory per record.

`scripts/check-evidence.py` runs in CI and in `./scripts/check.sh architecture`. It exists so that
a reader who did not make a measurement can tell whether its record still resolves. It checks four
things, each of which had already failed at least once when it was written:

- **links**: every relative link under `docs/` resolves. The headline findings table once linked a
  retained record by a path with neither the `raw/` prefix nor an extension, and the one row with
  no reachable record was a shipped optimisation's before and after numbers.
- **ledger**: every live-proved row in [the claim ledger](../claim-ledger.md) names a commit that
  is in history and links at least one retained artifact. Nothing previously verified either.
- **retention**: an evidence document that quotes a figure in milliseconds or below either links
  something under `raw/` or holds a transcript of its own.
- **orphans**: every directory under `raw/` is cited by some document. A record that nothing links
  is a measurement nobody can check, and it is also what a broken link leaves behind.

## What it deliberately does not check

These were considered and left out, because a check that reports things that are fine is worse
than no check at all.

- **Whether a quoted figure is derivable from the sample it links.** Every retained record has its
  own format: JSON lines, summary objects, console transcripts, ad hoc text. Recomputing a
  percentile from each would mean a parser per record, and a parser that is wrong reports a
  discrepancy that is not there. The link is checked; the arithmetic behind it is not.
- **Whether a figure was measured at the shape, branch, and host its row implies.** The findings
  table once carried a row measured on a different branch at a different memory size, folded in as
  if it were current. Nothing in the text distinguishes that from a correct row, and inferring it
  would mean guessing. This is the largest real gap, and the defence against it is the rule at the
  top of the claim ledger rather than a script.
- **Figures in whole seconds.** `\d+ s` collides with ordinary prose far too often to be usable.
  The threshold is milliseconds and below, so a record that only ever quotes seconds is exempt from
  the retention rule.
- **Prose claims outside a table.** A sentence asserting a result is not distinguishable by any
  reliable pattern from a sentence describing one.

## Records with a known weakness

- [The declared device set on the merged binary](2026-08-31-merged-binary-device-set-c100.md)
  retains its twelve concurrency-100 cohorts but not its sequential arm; the record says so.
- [Local Docker Node 22](2026-08-29-docker-node22-local.md) quotes elapsed times in seconds with
  no retained sample. It passes the retention rule only because its figures are in seconds, and it
  is a development-machine record that no ledger row rests a latency claim on.
