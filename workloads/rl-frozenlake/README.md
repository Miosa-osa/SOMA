# FrozenLake reinforcement-learning workload

This image runs Gymnasium's deterministic 4×4 FrozenLake environment.
A tabular Q-learning policy learns from rewards and is evaluated against random actions on the same evaluation seeds. [Environment contract](https://gymnasium.farama.org/v1.2.3/environments/toy_text/frozen_lake/)

## Run the published image

The release workflow verifies the Linux x86_64 image before publishing it to the [SOMA container registry](https://github.com/orgs/Miosa-osa/packages/container/package/soma-rl-frozenlake).
Use the `main` tag to try the latest build and the OCI digest recorded in the workflow summary for reproducible runs.

```sh
image=ghcr.io/miosa-osa/soma-rl-frozenlake:main
docker run --rm --platform linux/amd64 --network none "$image" --seed 42
```

## Build and run locally

From the repository root, build for your Docker host architecture.

```sh
docker build -t soma-rl-frozenlake:1 workloads/rl-frozenlake
docker run --rm --network none soma-rl-frozenlake:1 --seed 42
```

The command prints one JSON object with the seed, episode counts, goals reached by each policy, and the trained action for each observation.
Adjust `--train-episodes`, `--eval-episodes`, or `--seed` to run another trial.

## Run through SOMA

The local Docker backend resolves images from a registry.
The published image is x86_64.
On an ARM host, build locally, then tag and push that build to a test registry the Docker daemon can reach and set `image` to that reference.

```sh
image=ghcr.io/miosa-osa/soma-rl-frozenlake:main
cargo run --locked -p soma-cli -- --backend docker run "$image" -- \
  /usr/local/bin/python3 /opt/soma-rl/train.py --seed 42
```

For multiple trials, build the CLI once and start one command per seed with `target/debug/soma`.
With `--format json`, each run returns its instance ID and cleanup receipt, and the training result is in base64-encoded stdout.

## Local verification

Four concurrent runs through the SOMA alpha.5 Docker backend each trained for 2,000 episodes and evaluated both policies for 100 episodes.

| Seed | Random goals | Trained goals |
| --- | ---: | ---: |
| 42 | 1 | 100 |
| 43 | 2 | 100 |
| 44 | 0 | 100 |
| 45 | 3 | 100 |

Each run had a distinct instance ID and reported complete machine and storage cleanup.
The results apply to the deterministic 4×4 map and these seeds.

## Larger training workloads

This workload trains a small policy inside each instance.
For larger experiments, an external trainer can own policy updates and batch observations across multiple isolated environment workers.
Gymnasium defines how to seed and reset workers and how to report termination and truncation separately. [Gymnasium environment API](https://gymnasium.farama.org/introduction/basic_usage/)

RLlib's EnvRunner architecture illustrates another scaling model, with multiple environments per runner and multiple runners per trainer. [RLlib scaling guide](https://docs.ray.io/en/latest/rllib/scaling-guide.html)

## Verify on a KVM host

Use a prepared Linux x86_64 test host and the existing [end-to-end check](../../docs/product/guides/end-to-end-check.md).
Pass the published x86_64 digest to compile a Generation, take a snapshot, run the workload twice, and check cleanup.
The release workflow runs this check separately from the container tests and retains the VM receipts.

```sh
scripts/end-to-end-check.sh \
  --image "ghcr.io/miosa-osa/soma-rl-frozenlake@${DIGEST:?set DIGEST from the release workflow}" \
  --fs-tools ./fs-tools \
  --expect '"trained_goal_count": 100' \
  --command /usr/local/bin/python3 /opt/soma-rl/train.py --seed 42
```

The check uses a private test work directory and an uncertified Generation.
A hosted runner result applies to the tested OCI digest and SOMA source revision.
MIOSA fleet placement and a customer template require their own live verification.
