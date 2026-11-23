# Run reinforcement-learning environments on SOMA

A SOMA instance can run a reinforcement-learning (RL) environment as a Linux workload.
For example, it can run a simulator, a game environment, or an evaluation task with its own dependencies and private workspace.

The environment defines observations, actions, rewards, and when an episode ends.
Your trainer chooses actions, collects trajectories, and updates its policy.
SOMA supplies the isolated execution environment and the lifecycle for starting workers, running code, retrieving results, and cleaning up.

## Train a small policy

The [FrozenLake workload](../../../workloads/rl-frozenlake/README.md) provides a published Gymnasium image, build instructions, and a policy training example that compares its results with a random baseline.

### Verified execution paths

The [public x86_64 release](https://github.com/Miosa-osa/SOMA/releases/tag/rl-frozenlake-642dec1f93fc) includes the image archive and a checksum.
The downloaded archive passed checksum verification and the seed 42 run reached the goal in all 100 evaluation episodes.
The [managed MIOSA CLI example](../../../workloads/rl-frozenlake/README.md#try-the-workload-through-miosa) reached the same result on a Firecracker placement.

The [standalone SOMA KVM run](https://github.com/Miosa-osa/SOMA/actions/runs/45133739268) prepared a Generation and captured its snapshot.
The first launch returned `backend_unavailable` before the guest ran, so the KVM run does not establish workload execution or cleanup.
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
