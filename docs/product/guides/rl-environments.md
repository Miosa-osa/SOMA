# Run reinforcement-learning environments on SOMA

A SOMA instance can run a reinforcement-learning (RL) environment as a Linux workload.
For example, it can run a simulator, a game environment, or an evaluation task with its own dependencies and private workspace.

The environment defines observations, actions, rewards, and when an episode ends.
Your trainer chooses actions, collects trajectories, and updates its policy.
SOMA supplies the isolated execution environment and the lifecycle for starting workers, running code, retrieving results, and cleaning up.

## Train a small policy

The [FrozenLake workload](../../../workloads/rl-frozenlake/README.md) provides a published Gymnasium image, build instructions, and a policy training example that compares its results with a random baseline.

### Verified execution paths

The [public x86_64 release](https://github.com/Miosa-osa/SOMA/releases/tag/rl-frozenlake-0ad59e527577) includes FrozenLake, CartPole, and a checksum for the image archive.
The FrozenLake release passed checksum verification and the seed 42 run reached the goal in all 100 evaluation episodes.
The [managed MIOSA CLI example](../../../workloads/rl-frozenlake/README.md#try-the-workload-through-miosa) reached the same result on a Firecracker placement.

The [standalone SOMA KVM run](https://github.com/Miosa-osa/SOMA/actions/runs/45144288502) compiled the published image into a Generation, captured a snapshot, and restored two hardware-isolated instances that each ran FrozenLake successfully.
The [previous diagnostic trace](https://github.com/Miosa-osa/SOMA/actions/runs/45139971535) showed the restore workers were denied read-write access to `/dev/kvm`.
The successful run used the Linux `kvm` group on the disposable Ubuntu runner.
Managed SOMA placement for this workload remains unverified.

## Run one episode locally

Run the example from a SOMA checkout with Docker running.
A fixed policy walks along a small corridor from a seeded starting position.
The example prints an episode trace you can inspect or pass to a trainer.

```sh
cargo run --locked -p soma-cli -- --backend docker run python:3.12-slim -- \
  /usr/local/bin/python3 -c '
import json
import random

seed = 42
goal = 4
position = random.Random(seed).randrange(goal)
trajectory = []

while position < goal:
    observation = position
    position += 1
    done = position == goal
    trajectory.append({
        "observation": observation,
        "action": "right",
        "reward": int(done),
        "next_observation": position,
        "done": done,
    })

print(json.dumps({"seed": seed, "trajectory": trajectory}))
'
```

This demonstrates one rollout with a fixed policy.
For a training job, replace the example with your environment and policy client.

## Run parallel episodes

The [CartPole rollout benchmark](../../../workloads/rl-frozenlake/README.md#measure-parallel-cartpole-rollouts) measures one process stepping multiple environments and separate worker processes stepping the same seeded workload.
It reports episode returns, termination and time-limit counts, worker startup, and transitions per second.
Both vector modes completed 100 episodes and 50,000 recorded transitions through the MIOSA CLI on Firecracker.
Gymnasium recommends measuring across worker counts because process communication can outweigh parallelism for small environments.
[Gymnasium vectorization guide](https://gymnasium.farama.org/main/introduction/speed_up_env/)


1. Package the environment and its dependencies in a versioned OCI image.
2. Start workers from the same image or prepared Generation.
   Set a seed and episode ID for every worker.
3. Run one episode per instance for a clean reset, or reuse a worker when the environment reliably resets its state between episodes.
4. Write trajectories and scores to a result file or stream them to your trainer.
   Record timeouts and failed episodes alongside the scores.
5. Destroy workers when their batches finish.
   Keep model checkpoints and experiment results in storage managed by the trainer.

Use the [HTTP API](../reference/api-surface.md) for instance lifecycle and file operations.
GPU training and distributed policy updates can run in a separate training service while CPU-based environments run on SOMA.
