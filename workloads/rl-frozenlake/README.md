# FrozenLake reinforcement-learning workload

This image runs Gymnasium's deterministic 4×4 FrozenLake environment.
A tabular Q-learning policy learns from rewards and is evaluated against random actions on the same evaluation seeds. [Environment contract](https://gymnasium.farama.org/v1.2.3/environments/toy_text/frozen_lake/)

## Run the published image

Download the Linux x86_64 image archive and its checksum from a [FrozenLake release](https://github.com/Miosa-osa/SOMA/releases?q=rl-frozenlake).
The [build workflow](https://github.com/Miosa-osa/SOMA/actions/workflows/rl-workload.yml) links to the release for each source revision.
Verify the checksum, then load and run the image without network access.

```sh
sha256sum -c soma-rl-frozenlake-linux-amd64.tar.gz.sha256
docker load -i soma-rl-frozenlake-linux-amd64.tar.gz
docker run --rm --platform linux/amd64 --network none soma-rl-frozenlake:release --seed 42
```

On macOS, use `shasum -a 256 -c` to verify the same checksum.
The build also publishes to the [access-controlled SOMA container registry](https://github.com/orgs/Miosa-osa/packages/container/package/soma-rl-frozenlake).
Registry users can pin the OCI digest from the build workflow summary.

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
If you use the public archive, tag and push the loaded image to a registry your Docker daemon can reach, then set `image` to that reference.
The published image is x86_64.
On an ARM host, use a locally built image in your registry.

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

## Try the workload through MIOSA

The MIOSA CLI can run the same Python workload on an existing sandbox with Python 3.10 or later, the Python `venv` module, curl, and outbound access to GitHub and PyPI.
Create a private sandbox without a workspace environment if you need a temporary one.
Check which runtime MIOSA placed before attributing results to SOMA.

```sh
miosa create frozenlake-trial --runtime auto --size xs --no-env --visibility private --ttl 30m --wait
miosa --json sandbox resources frozenlake-trial
miosa --timeout 120 exec frozenlake-trial -- sh -eu -c '
  work=$(mktemp -d)
  trap '\''rm -rf "$work"'\'' EXIT
  source=https://raw.githubusercontent.com/Miosa-osa/SOMA/09cfacd9dc76f815740685f8646226eed07eddf7/workloads/rl-frozenlake
  curl -fsSL "$source/train.py" -o "$work/train.py"
  curl -fsSL "$source/requirements.txt" -o "$work/requirements.txt"
  python3 -m venv "$work/python"
  "$work/python/bin/python" -m pip install -q --only-binary=:all: -r "$work/requirements.txt"
  "$work/python/bin/python" "$work/train.py" --seed 42
'
miosa destroy frozenlake-trial --yes
```

The managed sandbox smoke used auto placement and reported `capabilities.runtime_engine=firecracker`.
It reached the goal 100 times out of 100, compared with 1 for the random baseline.
To require SOMA, use `--runtime soma` with a supported size such as `small` and check runtime capacity before running the workload.
This CLI example installs the workload inside the guest.
The published OCI image is an input for the separate SOMA Generation workflow.

## Larger training workloads

This workload trains a small policy inside each instance.
For larger experiments, an external trainer can own policy updates and batch observations across multiple isolated environment workers.
Gymnasium defines how to seed and reset workers and how to report termination and truncation separately. [Gymnasium environment API](https://gymnasium.farama.org/introduction/basic_usage/)

RLlib's EnvRunner architecture illustrates another scaling model, with multiple environments per runner and multiple runners per trainer. [RLlib scaling guide](https://docs.ray.io/en/latest/rllib/scaling-guide.html)

## Verify on a KVM host

Use a prepared Linux x86_64 test host and the existing [end-to-end check](../../docs/product/guides/end-to-end-check.md).
After logging in to the registry, pass the published x86_64 digest to compile a Generation, take a snapshot, run the workload twice, and check cleanup.
For the public archive, load the image into Docker, set `SOMA_IMAGE_SOURCE=docker-daemon:soma-rl-frozenlake:release` and pass `--image soma-rl-frozenlake:release`.
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

## Measure parallel CartPole rollouts

The same Gymnasium image includes a CartPole-v1 rollout benchmark.
It compares synchronous vector execution in one process with asynchronous vector execution using one spawned process per environment.
The random policy provides a baseline, and the balancing controller uses a fixed feedback rule.
[CartPole environment](https://gymnasium.farama.org/v1.2.3/environments/classic_control/cart_pole/) · [Gymnasium vector API](https://gymnasium.farama.org/api/vector/)

```sh
docker run --rm --network none --cpus 4 --entrypoint /usr/local/bin/python3 \
  soma-rl-frozenlake:release /opt/soma-rl/rollouts.py \
  --mode async --workers 4 --episodes-per-worker 25 --policy balance --seed 42
```

Use `--mode sync` to measure the same episode workload in one process.
Use `--policy random` to evaluate random moves.
The default run records the first 25 completed episodes from each of four environments.
The vector environment resets a finished lane in the same step, so resets never count as transitions.
If a step both terminates and reaches the time limit, the result counts it as a termination.
[Gymnasium autoreset modes](https://gymnasium.farama.org/main/api/vector/autoreset/)

The result reports startup and rollout time separately.
`recorded_steps` counts steps in the requested episodes.
`simulator_steps` also includes work on lanes that already met their episode quota while other lanes finished.
The throughput rate uses simulator steps and excludes startup and cleanup.
Use matching seeds, worker counts, and CPU limits across repeated runs.

CI checks that both vector modes complete the same seeded episodes and produce identical counters.
The timing fields are reported as observations because process and IPC overhead vary by host.


### Local SOMA measurements

These runs used the SOMA alpha.5 Docker backend on Apple Silicon with a two-vCPU container limit, 1 GiB of RAM, and guest networking disabled.
The image was already built and present in the local registry.
Each mode and worker count ran in three fresh instances with seeds 42, 43, and 44.
Each worker recorded 25 episodes under the balancing controller, and all 12 instances reported complete machine and storage cleanup.
The timing excludes OCI image retrieval, SOMA launch, and cleanup.

| Mode | Workers | Median startup | Median simulator steps/s |
| --- | ---: | ---: | ---: |
| sync | 1 | 0.002 s | 54,455 |
| async | 1 | 0.282 s | 5,974 |
| sync | 4 | 0.002 s | 117,175 |
| async | 4 | 0.461 s | 14,824 |

<details>
<summary>Individual samples</summary>

| Mode | Workers | Seed | Simulator steps | Startup (ms) | Rollout (ms) |
| --- | ---: | ---: | ---: | ---: | ---: |
| async | 1 | 42 | 12500 | 256.73 | 2092.39 |
| async | 1 | 43 | 12500 | 282.03 | 2192.12 |
| async | 1 | 44 | 12500 | 290.09 | 2086.34 |
| async | 4 | 42 | 50000 | 522.48 | 3703.08 |
| async | 4 | 43 | 50000 | 459.35 | 3315.35 |
| async | 4 | 44 | 50000 | 460.71 | 3372.92 |
| sync | 1 | 42 | 12500 | 1.99 | 230.37 |
| sync | 1 | 43 | 12500 | 1.70 | 229.55 |
| sync | 1 | 44 | 12500 | 2.08 | 228.08 |
| sync | 4 | 42 | 50000 | 2.25 | 426.71 |
| sync | 4 | 43 | 50000 | 2.24 | 435.71 |
| sync | 4 | 44 | 50000 | 2.45 | 409.63 |

</details>
