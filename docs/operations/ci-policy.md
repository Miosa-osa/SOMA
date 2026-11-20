# Continuous integration execution policy

SOMA uses the cheapest workflow that can answer each engineering question honestly.
Routine GitHub-hosted jobs validate source correctness and portability contracts.
They do not substitute for real KVM execution, production admission, or performance evidence.

## Trigger map

| Change or event | Workflow | Purpose |
| --- | --- | --- |
| Rust, benchmark, or script change on a pull request or `main` | CI | Full Ubuntu 24.04 correctness plus Linux ARM64 compile-check |
| Markdown or claim-ledger change on a pull request or `main` | Documentation policy | Fast architecture, forbidden-dash, and claim-ledger validation |
| Rust, benchmark, or script change on a pull request or `main` | Portability | macOS ARM64 portable profile, Intel macOS compile-check, Windows x86_64 portable client with lint, tests, and rustdoc, Windows ARM64 compile-check, and the Ubuntu 26.04 Linux profile |
| Weekly schedule or manual request | Portability | Detect runner image and upstream drift on the same matrix even without repository changes |
| Dependency, toolchain, workflow-policy, or security-script change | Security and dependencies | Supply-chain, advisory, license, workflow, spelling, and secret policy |
| Weekly schedule or manual request | Security and dependencies | Detect upstream advisories and policy drift even without repository changes |
| Version tag or manual request | KVM smoke | Exercise the real KVM path on the isolated self-hosted runner |
| Version tag or manual request | Release bundle | Validate and package source and native client artifacts |

## Cost controls

Every platform the README claims is checked on every change that can break it, because a matrix that runs only weekly let macOS and Windows stay red for months unseen.
The CI and Portability workflows run only when Rust, benchmark, script, or their own workflow inputs change, so a documentation-only change starts no macOS or Windows runner.
Security tool installation runs only when relevant policy or dependency inputs change, plus the weekly drift scan.
The self-hosted KVM runner does not queue work for every commit.
Release packaging does not run outside tag or manual validation events.
Every repeatable workflow cancels an older run for the same ref when a newer commit supersedes it.

## Proof boundaries

Green CI means the repository compiled and passed its declared tests on the selected hosted runner.
Green portability means the portable client paths passed on the macOS, Windows, and Ubuntu 26.04 runners of that matrix.
Green KVM smoke means the isolated Linux runner completed the smoke contract defined by `scripts/kvm-smoke.sh`.
None of those results establishes latency, density, or production admission.
Those claims require retained benchmark artifacts and the evidence gates in the benchmark contract.

## Operator commands

Use the GitHub Actions interface or GitHub CLI to dispatch Portability, Security and dependencies, KVM smoke, or Release bundle when a change needs validation before its normal scheduled or release trigger.
Do not add a broad push trigger to obtain a one-time result.
If a new mandatory platform enters the release contract, add it to the portability matrix and the native release bundle in the same change.
