# Raw records for the status surface audit, eval-1, `d38515b`

- `build.json` - the controlled release build manifest both runs were measured against.
- `kvm8.jsonl` - eight KVM slots at the shape the Generation was captured at. Scores zero, and
  the `run_completion` record's `failure_breakdown` says why: every launch was refused
  `machine_not_hosted` at exit 76, and every destroy then reported `machine_not_found` at exit
  66. Before this branch the same run reported a count and nothing else.
- `docker2.jsonl` - two docker slots at `--storage-mib 10240`. Scores two of two and still
  reports `shape_disagreements`, because the storage dimension the caller asked for was never
  observed.
